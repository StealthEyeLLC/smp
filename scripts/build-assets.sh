#!/bin/bash
set -euo pipefail
umask 077

FIRECRACKER_VERSION=1.15.1
KERNEL_VERSION=6.1.177
DEBIAN_SUITE=trixie
DEBIAN_VERSION=13.6
DEBIAN_MIRROR=https://deb.debian.org/debian
SECURITY_MIRROR=https://security.debian.org/debian-security
ARCH=x86_64
ASSETS_ROOT=/var/lib/smp/assets
ETC_ROOT=/etc/smp
OFFLINE=0
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -d "$SCRIPT_DIR/../assets" ]]; then
    SOURCE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
else
    SOURCE_ROOT="$SCRIPT_DIR"
fi

while (($#)); do
    case "$1" in
        --assets-root) ASSETS_ROOT="$2"; shift 2 ;;
        --etc-root) ETC_ROOT="$2"; shift 2 ;;
        --offline) OFFLINE=1; shift ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 64 ;;
    esac
done

[[ $(id -u) -eq 0 ]] || { printf 'build-assets.sh requires root\n' >&2; exit 77; }
[[ $(uname -m) == x86_64 ]] || { printf 'SMP canonical assets require x86_64\n' >&2; exit 69; }
[[ -d "$SOURCE_ROOT/assets/guest" && -d "$SOURCE_ROOT/assets/guest-tools" ]] || {
    printf 'SMP guest asset sources are missing beneath %s\n' "$SOURCE_ROOT" >&2
    exit 66
}

for tool in curl tar sha256sum jq make gcc debootstrap truncate mkfs.ext4 mount umount chroot rsync blkid; do
    command -v "$tool" >/dev/null || { printf 'missing required tool: %s\n' "$tool" >&2; exit 69; }
done

mkdir -p "$ASSETS_ROOT/downloads" "$ASSETS_ROOT/firecracker" "$ASSETS_ROOT/kernel" "$ASSETS_ROOT/rootfs" "$ASSETS_ROOT/provenance" "$ETC_ROOT"
WORK="$(mktemp -d "$ASSETS_ROOT/.build.XXXXXX")"
ROOT_MOUNTED=0
DEV_MOUNTED=0
PROC_MOUNTED=0
SYS_MOUNTED=0
cleanup() {
    if [[ $SYS_MOUNTED -eq 1 ]]; then umount "$WORK/root/sys" >/dev/null 2>&1 || true; fi
    if [[ $PROC_MOUNTED -eq 1 ]]; then umount "$WORK/root/proc" >/dev/null 2>&1 || true; fi
    if [[ $DEV_MOUNTED -eq 1 ]]; then umount "$WORK/root/dev" >/dev/null 2>&1 || true; fi
    if [[ $ROOT_MOUNTED -eq 1 ]]; then umount "$WORK/root" >/dev/null 2>&1 || true; fi
    rm -rf "$WORK"
}
trap cleanup EXIT

download() {
    local url=$1 destination=$2
    if [[ -s "$destination" ]]; then return 0; fi
    [[ $OFFLINE -eq 0 ]] || { printf 'offline asset missing: %s\n' "$destination" >&2; exit 69; }
    curl --fail --location --proto '=https' --tlsv1.2 --retry 4 --output "$destination.part" "$url"
    mv "$destination.part" "$destination"
}

printf 'Preparing pinned Firecracker %s\n' "$FIRECRACKER_VERSION"
FC_ARCHIVE="firecracker-v${FIRECRACKER_VERSION}-${ARCH}.tgz"
FC_URL="https://github.com/firecracker-microvm/firecracker/releases/download/v${FIRECRACKER_VERSION}/${FC_ARCHIVE}"
FC_SUM_URL="${FC_URL}.sha256.txt"
download "$FC_URL" "$ASSETS_ROOT/downloads/$FC_ARCHIVE"
download "$FC_SUM_URL" "$ASSETS_ROOT/downloads/$FC_ARCHIVE.sha256.txt"
(
    cd "$ASSETS_ROOT/downloads"
    sed "s#  .*/#  #" "$FC_ARCHIVE.sha256.txt" | sha256sum --check --strict -
)
mkdir -p "$WORK/firecracker"
tar -xzf "$ASSETS_ROOT/downloads/$FC_ARCHIVE" -C "$WORK/firecracker"
FC_SOURCE="$(find "$WORK/firecracker" -type f -name "firecracker-v${FIRECRACKER_VERSION}-${ARCH}" -print -quit)"
[[ -n "$FC_SOURCE" ]] || { printf 'Firecracker binary absent from official archive\n' >&2; exit 65; }
install -m 0755 "$FC_SOURCE" "$ASSETS_ROOT/firecracker/firecracker-v${FIRECRACKER_VERSION}-${ARCH}"
FC_ARCHIVE_SHA="$(sha256sum "$ASSETS_ROOT/downloads/$FC_ARCHIVE" | cut -d' ' -f1)"
FC_BINARY_SHA="$(sha256sum "$ASSETS_ROOT/firecracker/firecracker-v${FIRECRACKER_VERSION}-${ARCH}" | cut -d' ' -f1)"
printf '%s  %s\n' "$FC_ARCHIVE_SHA" "$FC_ARCHIVE" > "$ASSETS_ROOT/provenance/firecracker-archive.sha256"
printf '%s  %s\n' "$FC_BINARY_SHA" "firecracker-v${FIRECRACKER_VERSION}-${ARCH}" > "$ASSETS_ROOT/provenance/firecracker-binary.sha256"

printf 'Building pinned Linux %s vmlinux and modules\n' "$KERNEL_VERSION"
KERNEL_ARCHIVE="linux-${KERNEL_VERSION}.tar.xz"
KERNEL_URL="https://cdn.kernel.org/pub/linux/kernel/v6.x/${KERNEL_ARCHIVE}"
KERNEL_SUMS_URL="https://cdn.kernel.org/pub/linux/kernel/v6.x/sha256sums.asc"
download "$KERNEL_URL" "$ASSETS_ROOT/downloads/$KERNEL_ARCHIVE"
download "$KERNEL_SUMS_URL" "$ASSETS_ROOT/downloads/kernel-sha256sums.asc"
KERNEL_EXPECTED="$(awk -v file="$KERNEL_ARCHIVE" '$2 == file {print $1; exit}' "$ASSETS_ROOT/downloads/kernel-sha256sums.asc")"
[[ "$KERNEL_EXPECTED" =~ ^[0-9a-f]{64}$ ]] || { printf 'kernel checksum not found in official checksum file\n' >&2; exit 65; }
printf '%s  %s\n' "$KERNEL_EXPECTED" "$ASSETS_ROOT/downloads/$KERNEL_ARCHIVE" | sha256sum --check --strict -
tar -xJf "$ASSETS_ROOT/downloads/$KERNEL_ARCHIVE" -C "$WORK"
KERNEL_SOURCE="$WORK/linux-$KERNEL_VERSION"
pushd "$KERNEL_SOURCE" >/dev/null
make x86_64_defconfig
for setting in \
    MODULES BLK_DEV_LOOP EXT4_FS DEVTMPFS DEVTMPFS_MOUNT TMPFS POSIX_MQUEUE \
    NAMESPACES UTS_NS IPC_NS USER_NS PID_NS NET_NS CGROUPS CGROUP_BPF CGROUP_SCHED \
    OVERLAY_FS NETFILTER NF_TABLES NFT_CT NFT_NAT NFT_MASQ NFT_REDIR NFT_REJECT \
    TUN VETH BRIDGE BRIDGE_NETFILTER INET IP_ADVANCED_ROUTER IP_MULTIPLE_TABLES \
    VIRTIO VIRTIO_PCI VIRTIO_PCI_LEGACY VIRTIO_BLK VIRTIO_NET VIRTIO_CONSOLE \
    HW_RANDOM HW_RANDOM_VIRTIO VSOCKETS VIRTIO_VSOCKETS VIRTIO_VSOCKETS_COMMON \
    SERIAL_8250 SERIAL_8250_CONSOLE UNIX PACKET INET IPV6 SECCOMP SECCOMP_FILTER \
    BPF_SYSCALL KPROBES FTRACE DEBUG_FS; do
    scripts/config --enable "CONFIG_${setting}"
done
scripts/config --disable CONFIG_MODULE_SIG_ALL
make olddefconfig
KERNEL_CONFIG_SHA="$(sha256sum .config | cut -d' ' -f1)"
make -j"$(nproc)" vmlinux modules
make modules_install INSTALL_MOD_PATH="$WORK/modules"
install -m 0644 vmlinux "$ASSETS_ROOT/kernel/vmlinux-${KERNEL_VERSION}"
popd >/dev/null
rm -rf "$ASSETS_ROOT/kernel/modules-${KERNEL_VERSION}"
mv "$WORK/modules/lib/modules" "$ASSETS_ROOT/kernel/modules-${KERNEL_VERSION}"
VMLINUX_SHA="$(sha256sum "$ASSETS_ROOT/kernel/vmlinux-${KERNEL_VERSION}" | cut -d' ' -f1)"
MODULE_TREE_SHA="$(find "$ASSETS_ROOT/kernel/modules-${KERNEL_VERSION}" -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1)"
printf '%s  .config\n' "$KERNEL_CONFIG_SHA" > "$ASSETS_ROOT/provenance/kernel-config.sha256"
printf '%s  vmlinux-%s\n' "$VMLINUX_SHA" "$KERNEL_VERSION" > "$ASSETS_ROOT/provenance/vmlinux.sha256"
printf '%s  modules-%s\n' "$MODULE_TREE_SHA" "$KERNEL_VERSION" > "$ASSETS_ROOT/provenance/module-tree.sha256"

printf 'Building Debian %s %s ext4 root filesystem\n' "$DEBIAN_VERSION" "$DEBIAN_SUITE"
ROOTFS_NEW="$ASSETS_ROOT/rootfs/debian-${DEBIAN_VERSION}-${DEBIAN_SUITE}-amd64.ext4.new"
ROOTFS_FINAL="$ASSETS_ROOT/rootfs/debian-${DEBIAN_VERSION}-${DEBIAN_SUITE}-amd64.ext4"
rm -f "$ROOTFS_NEW"
truncate -s 8G "$ROOTFS_NEW"
mkfs.ext4 -F -L SMP_ROOT "$ROOTFS_NEW" >/dev/null
mkdir -p "$WORK/root"
mount -o loop "$ROOTFS_NEW" "$WORK/root"
ROOT_MOUNTED=1
debootstrap --arch=amd64 --variant=minbase "$DEBIAN_SUITE" "$WORK/root" "$DEBIAN_MIRROR"
cat > "$WORK/root/etc/apt/sources.list" <<SOURCES
deb $DEBIAN_MIRROR $DEBIAN_SUITE main
deb $SECURITY_MIRROR ${DEBIAN_SUITE}-security main
SOURCES
cp /etc/resolv.conf "$WORK/root/etc/resolv.conf"
mount --bind /dev "$WORK/root/dev"; DEV_MOUNTED=1
mount -t proc proc "$WORK/root/proc"; PROC_MOUNTED=1
mount -t sysfs sys "$WORK/root/sys"; SYS_MOUNTED=1
chroot "$WORK/root" /bin/bash -eux <<'CHROOT'
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get -y full-upgrade
apt-get install -y --no-install-recommends \
  systemd-sysv dbus openssh-server nftables iproute2 iputils-ping dnsutils \
  ca-certificates curl e2fsprogs util-linux kmod procps bash coreutils findutils \
  grep sed gawk tar gzip xz-utils gcc make libc6-dev pkg-config git mount rsync \
  bridge-utils netcat-openbsd file less vim-tiny
apt-get clean
rm -rf /var/lib/apt/lists/*
mkdir -p /root/.ssh /usr/local/libexec /var/lib/smp-seed /etc/systemd/network /etc/ssh/sshd_config.d
chmod 0700 /root/.ssh
cat > /etc/ssh/sshd_config.d/10-smp-root.conf <<'SSH'
PermitRootLogin prohibit-password
PasswordAuthentication no
PubkeyAuthentication yes
PermitEmptyPasswords no
SSH
rm -f /etc/ssh/ssh_host_* /etc/machine-id
: > /etc/machine-id
ln -sfn /run/systemd/resolve/stub-resolv.conf /etc/resolv.conf
systemctl enable ssh systemd-networkd systemd-resolved
CHROOT

install -m 0755 "$SOURCE_ROOT/assets/guest/smp-seed-init.sh" "$WORK/root/usr/local/libexec/smp-seed-init"
install -m 0644 "$SOURCE_ROOT/assets/guest/smp-seed-init.service" "$WORK/root/etc/systemd/system/smp-seed-init.service"
mkdir -p "$WORK/root/etc/systemd/system/multi-user.target.wants"
ln -sfn ../smp-seed-init.service "$WORK/root/etc/systemd/system/multi-user.target.wants/smp-seed-init.service"
install -m 0644 "$SOURCE_ROOT/assets/guest-tools/smp-exec-hex.c" "$WORK/root/tmp/smp-exec-hex.c"
install -m 0644 "$SOURCE_ROOT/assets/guest-tools/smp-file-hex.c" "$WORK/root/tmp/smp-file-hex.c"
chroot "$WORK/root" gcc -O2 -Wall -Wextra -Werror -o /usr/local/libexec/smp-exec-hex /tmp/smp-exec-hex.c
chroot "$WORK/root" gcc -O2 -Wall -Wextra -Werror -o /usr/local/libexec/smp-file-hex /tmp/smp-file-hex.c
rm -f "$WORK/root/tmp/smp-exec-hex.c" "$WORK/root/tmp/smp-file-hex.c"
ln "$WORK/root/usr/local/libexec/smp-file-hex" "$WORK/root/usr/local/libexec/smp-file-write-hex"
ln "$WORK/root/usr/local/libexec/smp-file-hex" "$WORK/root/usr/local/libexec/smp-file-read-hex"
rsync -aH "$ASSETS_ROOT/kernel/modules-${KERNEL_VERSION}/" "$WORK/root/lib/modules/"
printf '%s\n' "$DEBIAN_VERSION" > "$WORK/root/etc/smp-debian-version"
chroot "$WORK/root" dpkg-query -W -f='${Package}\t${Version}\n' | LC_ALL=C sort > "$ASSETS_ROOT/provenance/debian-packages.tsv"
[[ "$(cat "$WORK/root/etc/debian_version")" == "$DEBIAN_VERSION"* ]] || {
    printf 'built rootfs reports Debian %s, expected %s\n' "$(cat "$WORK/root/etc/debian_version")" "$DEBIAN_VERSION" >&2
    exit 65
}

TRIXIE_INRELEASE="$ASSETS_ROOT/downloads/trixie-InRelease"
download "$DEBIAN_MIRROR/dists/$DEBIAN_SUITE/InRelease" "$TRIXIE_INRELEASE"
install -m 0644 "$TRIXIE_INRELEASE" "$ASSETS_ROOT/provenance/trixie-InRelease"
INRELEASE_SHA="$(sha256sum "$ASSETS_ROOT/provenance/trixie-InRelease" | cut -d' ' -f1)"
sync
umount "$WORK/root/sys"; SYS_MOUNTED=0
umount "$WORK/root/proc"; PROC_MOUNTED=0
umount "$WORK/root/dev"; DEV_MOUNTED=0
umount "$WORK/root"; ROOT_MOUNTED=0
e2fsck -pf "$ROOTFS_NEW" || [[ $? -eq 1 ]]
ROOTFS_SHA="$(sha256sum "$ROOTFS_NEW" | cut -d' ' -f1)"
rm -f "$ROOTFS_FINAL"
mv "$ROOTFS_NEW" "$ROOTFS_FINAL"
chmod 0444 "$ROOTFS_FINAL"
BUILT_AT="$(date --utc +%Y-%m-%dT%H:%M:%SZ)"

jq -n \
  --arg architecture "$ARCH" \
  --arg fcPath "$ASSETS_ROOT/firecracker/firecracker-v${FIRECRACKER_VERSION}-${ARCH}" \
  --arg fcSha "$FC_BINARY_SHA" \
  --arg fcVersion "$FIRECRACKER_VERSION" \
  --arg fcProv "$ASSETS_ROOT/provenance/firecracker-binary.sha256" \
  --arg kernelPath "$ASSETS_ROOT/kernel/vmlinux-${KERNEL_VERSION}" \
  --arg kernelSha "$VMLINUX_SHA" \
  --arg kernelVersion "$KERNEL_VERSION" \
  --arg kernelProv "$ASSETS_ROOT/provenance/vmlinux.sha256" \
  --arg rootfsPath "$ROOTFS_FINAL" \
  --arg rootfsSha "$ROOTFS_SHA" \
  --arg rootfsVersion "$DEBIAN_VERSION" \
  --arg rootfsProv "$ASSETS_ROOT/provenance/debian-packages.tsv" \
  --arg moduleTreeSha "$MODULE_TREE_SHA" \
  --arg fcAsset "$FC_ARCHIVE" \
  --arg fcAssetSha "$FC_ARCHIVE_SHA" \
  --arg kernelUrl "$KERNEL_URL" \
  --arg kernelConfigSha "$KERNEL_CONFIG_SHA" \
  --arg debianSuite "$DEBIAN_SUITE" \
  --arg debianVersion "$DEBIAN_VERSION" \
  --arg debianMirror "$DEBIAN_MIRROR" \
  --arg inReleaseSha "$INRELEASE_SHA" \
  --arg packageManifest "$ASSETS_ROOT/provenance/debian-packages.tsv" \
  --arg builtAt "$BUILT_AT" \
  '{schemaVersion:1, architecture:$architecture,
    firecracker:{path:$fcPath,sha256:$fcSha,version:$fcVersion,provenancePath:$fcProv},
    kernel:{path:$kernelPath,sha256:$kernelSha,version:$kernelVersion,provenancePath:$kernelProv},
    rootfs:{path:$rootfsPath,sha256:$rootfsSha,version:$rootfsVersion,provenancePath:$rootfsProv},
    moduleTreeSha256:$moduleTreeSha,firecrackerReleaseAsset:$fcAsset,
    firecrackerReleaseSha256:$fcAssetSha,kernelSourceUrl:$kernelUrl,
    kernelConfigSha256:$kernelConfigSha,debianSuite:$debianSuite,
    debianVersion:$debianVersion,debianMirror:$debianMirror,
    debianInreleaseSha256:$inReleaseSha,packageManifestPath:$packageManifest,builtAt:$builtAt}' \
  > "$ASSETS_ROOT/manifest.json.new"
chmod 0600 "$ASSETS_ROOT/manifest.json.new"
mv "$ASSETS_ROOT/manifest.json.new" "$ASSETS_ROOT/manifest.json"
printf 'SMP assets ready: %s\n' "$ASSETS_ROOT/manifest.json"
