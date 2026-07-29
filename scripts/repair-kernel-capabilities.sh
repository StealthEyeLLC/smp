#!/bin/bash
set -euo pipefail
umask 077

KERNEL_VERSION=6.1.177
ASSETS_ROOT=/var/lib/smp/assets
BUILD_COMMIT=${1:-unknown}
MIN_FREE_BYTES=$((10 * 1024 * 1024 * 1024))
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODULE_TREE_DIGEST="$SCRIPT_DIR/module-tree-digest.sh"

[[ $(id -u) -eq 0 ]] || { printf 'repair-kernel-capabilities.sh requires root\n' >&2; exit 77; }
[[ $BUILD_COMMIT =~ ^[0-9a-f]{40}$ ]] || { printf 'expected full build commit SHA\n' >&2; exit 64; }
[[ -x $MODULE_TREE_DIGEST ]] || {
    printf 'SMP module-tree digest helper is unavailable: %s\n' "$MODULE_TREE_DIGEST" >&2
    exit 66
}
for tool in awk cp cut depmod df e2fsck find grep install jq make mount mv nproc rm rsync sha256sum sort sync tar umount xargs; do
    command -v "$tool" >/dev/null || { printf 'missing kernel repair tool: %s\n' "$tool" >&2; exit 69; }
done

MANIFEST="$ASSETS_ROOT/manifest.json"
ARCHIVE="$ASSETS_ROOT/downloads/linux-${KERNEL_VERSION}.tar.xz"
SUMS="$ASSETS_ROOT/downloads/kernel-sha256sums.asc"
KERNEL_IMAGE="$ASSETS_ROOT/kernel/vmlinux-${KERNEL_VERSION}"
KERNEL_MODULES="$ASSETS_ROOT/kernel/modules-${KERNEL_VERSION}"
ROOTFS="$(jq -er '.rootfs.path' "$MANIFEST")"
KERNEL_CONFIG_PROVENANCE="$ASSETS_ROOT/provenance/kernel-config.sha256"
VMLINUX_PROVENANCE="$ASSETS_ROOT/provenance/vmlinux.sha256"
MODULE_PROVENANCE="$ASSETS_ROOT/provenance/module-tree.sha256"
REVISION_PROVENANCE="$ASSETS_ROOT/provenance/kernel-capabilities-revision.json"

[[ -r $MANIFEST && -f $ARCHIVE && -f $SUMS && -f $KERNEL_IMAGE && -d $KERNEL_MODULES && -f $ROOTFS ]] || {
    printf 'canonical kernel/rootfs assets are incomplete\n' >&2
    exit 66
}

AVAILABLE_BYTES="$(df -PB1 "$ASSETS_ROOT" | awk 'NR == 2 {print $4}')"
[[ $AVAILABLE_BYTES =~ ^[0-9]+$ ]] || { printf 'could not determine repair free space\n' >&2; exit 69; }
if (( AVAILABLE_BYTES < MIN_FREE_BYTES )); then
    printf 'kernel repair requires at least %s free bytes; found %s\n' "$MIN_FREE_BYTES" "$AVAILABLE_BYTES" >&2
    exit 70
fi
printf 'Kernel-repair free space: %s bytes\n' "$AVAILABLE_BYTES"

hash_tree() {
    local directory=$1
    "$MODULE_TREE_DIGEST" normalized "$directory"
}

EXPECTED_ARCHIVE_SHA="$(awk -v file="linux-${KERNEL_VERSION}.tar.xz" '$2 == file {print $1; exit}' "$SUMS")"
[[ $EXPECTED_ARCHIVE_SHA =~ ^[0-9a-f]{64}$ ]] || { printf 'kernel archive checksum is unavailable\n' >&2; exit 65; }
printf '%s  %s\n' "$EXPECTED_ARCHIVE_SHA" "$ARCHIVE" | sha256sum --check --strict -

PRIOR_KERNEL_SHA="$(jq -er '.kernel.sha256' "$MANIFEST")"
PRIOR_MODULE_SHA="$(jq -er '.moduleTreeSha256' "$MANIFEST")"
PRIOR_ROOTFS_SHA="$(jq -er '.rootfs.sha256' "$MANIFEST")"
PRIOR_ROOTFS_PROVENANCE="$(jq -er '.rootfs.provenancePath' "$MANIFEST")"
OBSERVED_KERNEL_SHA="$(sha256sum "$KERNEL_IMAGE" | cut -d' ' -f1)"
OBSERVED_MODULE_SHA="$(hash_tree "$KERNEL_MODULES")"
OBSERVED_ROOTFS_SHA="$(sha256sum "$ROOTFS" | cut -d' ' -f1)"
[[ $OBSERVED_KERNEL_SHA == "$PRIOR_KERNEL_SHA" ]] || { printf 'kernel digest mismatch before repair\n' >&2; exit 65; }
[[ $OBSERVED_MODULE_SHA == "$PRIOR_MODULE_SHA" ]] || { printf 'module-tree digest mismatch before repair\n' >&2; exit 65; }
[[ $OBSERVED_ROOTFS_SHA == "$PRIOR_ROOTFS_SHA" ]] || { printf 'rootfs digest mismatch before repair\n' >&2; exit 65; }

WORK="$(mktemp -d "$ASSETS_ROOT/.kernel-capability-repair.XXXXXX")"
SOURCE="$WORK/linux-${KERNEL_VERSION}"
BUILD="$WORK/build"
MODULE_STAGE="$WORK/modules"
ROOT_MOUNT="$WORK/root"
ROOTFS_CANDIDATE="$WORK/rootfs.ext4"
KERNEL_CANDIDATE="$WORK/vmlinux"
MODULES_CANDIDATE="$WORK/modules-${KERNEL_VERSION}"
MOUNTED=0
COMMITTED=0
KERNEL_BACKUP="${KERNEL_IMAGE}.previous"
MODULES_BACKUP="${KERNEL_MODULES}.previous"
ROOTFS_BACKUP="${ROOTFS}.previous"
MANIFEST_BACKUP="${MANIFEST}.previous"

cleanup() {
    if [[ $MOUNTED -eq 1 ]]; then umount "$ROOT_MOUNT" >/dev/null 2>&1 || true; fi
    if [[ $COMMITTED -eq 0 ]]; then
        if [[ -f $KERNEL_BACKUP ]]; then rm -f "$KERNEL_IMAGE"; mv "$KERNEL_BACKUP" "$KERNEL_IMAGE"; fi
        if [[ -d $MODULES_BACKUP ]]; then rm -rf "$KERNEL_MODULES"; mv "$MODULES_BACKUP" "$KERNEL_MODULES"; fi
        if [[ -f $ROOTFS_BACKUP ]]; then rm -f "$ROOTFS"; mv "$ROOTFS_BACKUP" "$ROOTFS"; fi
        if [[ -f $MANIFEST_BACKUP ]]; then rm -f "$MANIFEST"; mv "$MANIFEST_BACKUP" "$MANIFEST"; fi
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT

printf 'Extracting pinned Linux %s source\n' "$KERNEL_VERSION"
tar -xJf "$ARCHIVE" -C "$WORK"
mkdir -p "$BUILD"
make -C "$SOURCE" O="$BUILD" x86_64_defconfig

REQUIRED_SETTINGS=(
    MODULES BLK_DEV_LOOP EXT4_FS DEVTMPFS DEVTMPFS_MOUNT TMPFS POSIX_MQUEUE
    NAMESPACES UTS_NS IPC_NS USER_NS PID_NS NET_NS CGROUPS CGROUP_BPF CGROUP_SCHED
    OVERLAY_FS NETFILTER NF_TABLES NF_TABLES_IPV4 NF_TABLES_IPV6 NF_TABLES_INET
    NFT_CT NFT_NAT NFT_MASQ NFT_REDIR NFT_REJECT DUMMY
    TUN VETH BRIDGE BRIDGE_NETFILTER INET IP_ADVANCED_ROUTER IP_MULTIPLE_TABLES
    VIRTIO VIRTIO_PCI VIRTIO_PCI_LEGACY VIRTIO_BLK VIRTIO_NET VIRTIO_CONSOLE
    HW_RANDOM HW_RANDOM_VIRTIO VSOCKETS VIRTIO_VSOCKETS VIRTIO_VSOCKETS_COMMON
    SERIAL_8250 SERIAL_8250_CONSOLE UNIX PACKET IPV6 SECCOMP SECCOMP_FILTER
    BPF_SYSCALL KPROBES FTRACE DEBUG_FS
)
for setting in "${REQUIRED_SETTINGS[@]}"; do
    "$SOURCE/scripts/config" --file "$BUILD/.config" --enable "CONFIG_${setting}"
done
"$SOURCE/scripts/config" --file "$BUILD/.config" --disable CONFIG_MODULE_SIG_ALL
make -C "$SOURCE" O="$BUILD" olddefconfig

for setting in NF_TABLES NF_TABLES_IPV4 NF_TABLES_IPV6 NF_TABLES_INET DUMMY TUN VETH BRIDGE OVERLAY_FS; do
    grep -qx "CONFIG_${setting}=y" "$BUILD/.config" || {
        printf 'required kernel capability did not resolve to built-in: CONFIG_%s\n' "$setting" >&2
        exit 65
    }
done

KERNEL_CONFIG_SHA="$(sha256sum "$BUILD/.config" | cut -d' ' -f1)"
printf 'Building corrected Linux %s kernel and modules\n' "$KERNEL_VERSION"
make -C "$SOURCE" O="$BUILD" -j"$(nproc)" vmlinux modules
make -C "$SOURCE" O="$BUILD" modules_install INSTALL_MOD_PATH="$MODULE_STAGE"
install -m 0644 "$BUILD/vmlinux" "$KERNEL_CANDIDATE"
mv "$MODULE_STAGE/lib/modules" "$MODULES_CANDIDATE"
VMLINUX_SHA="$(sha256sum "$KERNEL_CANDIDATE" | cut -d' ' -f1)"
MODULE_TREE_SHA="$(hash_tree "$MODULES_CANDIDATE")"

printf 'Patching only kernel modules into the existing Debian rootfs\n'
cp --reflink=auto --sparse=always --preserve=mode,timestamps -- "$ROOTFS" "$ROOTFS_CANDIDATE"
chmod 0600 "$ROOTFS_CANDIDATE"
mkdir -p "$ROOT_MOUNT"
mount -o loop,rw "$ROOTFS_CANDIDATE" "$ROOT_MOUNT"
MOUNTED=1
rm -rf "$ROOT_MOUNT/lib/modules/${KERNEL_VERSION}"
mkdir -p "$ROOT_MOUNT/lib/modules"
rsync -aH "$MODULES_CANDIDATE/" "$ROOT_MOUNT/lib/modules/"
depmod -b "$ROOT_MOUNT" "$KERNEL_VERSION"
sync
umount "$ROOT_MOUNT"
MOUNTED=0
set +e
e2fsck -pf "$ROOTFS_CANDIDATE"
FSCK_STATUS=$?
set -e
if (( FSCK_STATUS > 1 )); then
    printf 'kernel-repaired rootfs failed e2fsck with status %s\n' "$FSCK_STATUS" >&2
    exit 65
fi
ROOTFS_SHA="$(sha256sum "$ROOTFS_CANDIDATE" | cut -d' ' -f1)"

printf 'Stopping certification VMs before atomically replacing kernel assets\n'
for machine in smp-cert-disposable smp-cert-no-fallback smp-cert-isolated smp-cert-persistent; do
    /usr/local/bin/smp destroy "$machine" --force >/dev/null 2>&1 || true
done

rm -f "$KERNEL_BACKUP" "$ROOTFS_BACKUP" "$MANIFEST_BACKUP"
rm -rf "$MODULES_BACKUP"
mv "$KERNEL_IMAGE" "$KERNEL_BACKUP"
mv "$KERNEL_MODULES" "$MODULES_BACKUP"
mv "$ROOTFS" "$ROOTFS_BACKUP"
cp "$MANIFEST" "$MANIFEST_BACKUP"

install -m 0644 "$KERNEL_CANDIDATE" "$KERNEL_IMAGE"
mv "$MODULES_CANDIDATE" "$KERNEL_MODULES"
chmod 0444 "$ROOTFS_CANDIDATE"
mv "$ROOTFS_CANDIDATE" "$ROOTFS"

printf '%s  .config\n' "$KERNEL_CONFIG_SHA" > "${KERNEL_CONFIG_PROVENANCE}.new"
printf '%s  vmlinux-%s\n' "$VMLINUX_SHA" "$KERNEL_VERSION" > "${VMLINUX_PROVENANCE}.new"
printf '%s  modules-%s\n' "$MODULE_TREE_SHA" "$KERNEL_VERSION" > "${MODULE_PROVENANCE}.new"

jq -n \
  --argjson schemaVersion 1 \
  --arg buildCommit "$BUILD_COMMIT" \
  --arg repairedAt "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
  --arg priorKernelSha256 "$PRIOR_KERNEL_SHA" \
  --arg kernelSha256 "$VMLINUX_SHA" \
  --arg priorModuleTreeSha256 "$PRIOR_MODULE_SHA" \
  --arg moduleTreeSha256 "$MODULE_TREE_SHA" \
  --arg priorRootfsSha256 "$PRIOR_ROOTFS_SHA" \
  --arg rootfsSha256 "$ROOTFS_SHA" \
  --arg priorRootfsProvenancePath "$PRIOR_ROOTFS_PROVENANCE" \
  --arg kernelConfigSha256 "$KERNEL_CONFIG_SHA" \
  --arg moduleTreeDigestAlgorithm "sha256-relative-regular-files-v1" \
  '{schemaVersion:$schemaVersion,buildCommit:$buildCommit,repairedAt:$repairedAt,
    priorKernelSha256:$priorKernelSha256,kernelSha256:$kernelSha256,
    priorModuleTreeSha256:$priorModuleTreeSha256,moduleTreeSha256:$moduleTreeSha256,
    priorRootfsSha256:$priorRootfsSha256,rootfsSha256:$rootfsSha256,
    priorRootfsProvenancePath:$priorRootfsProvenancePath,kernelConfigSha256:$kernelConfigSha256,
    moduleTreeDigestAlgorithm:$moduleTreeDigestAlgorithm,
    enabledCapabilities:["CONFIG_NF_TABLES","CONFIG_NF_TABLES_IPV4","CONFIG_NF_TABLES_IPV6","CONFIG_NF_TABLES_INET","CONFIG_DUMMY"]}' \
  > "${REVISION_PROVENANCE}.new"

jq \
  --arg kernelSha "$VMLINUX_SHA" \
  --arg moduleSha "$MODULE_TREE_SHA" \
  --arg rootfsSha "$ROOTFS_SHA" \
  --arg configSha "$KERNEL_CONFIG_SHA" \
  --arg rootfsProv "$REVISION_PROVENANCE" \
  --arg builtAt "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
  '.kernel.sha256=$kernelSha | .moduleTreeSha256=$moduleSha | .rootfs.sha256=$rootfsSha |
   .rootfs.provenancePath=$rootfsProv | .kernelConfigSha256=$configSha | .builtAt=$builtAt' \
  "$MANIFEST_BACKUP" > "${MANIFEST}.new"
chmod 0600 "${MANIFEST}.new" "${REVISION_PROVENANCE}.new"

mv "${KERNEL_CONFIG_PROVENANCE}.new" "$KERNEL_CONFIG_PROVENANCE"
mv "${VMLINUX_PROVENANCE}.new" "$VMLINUX_PROVENANCE"
mv "${MODULE_PROVENANCE}.new" "$MODULE_PROVENANCE"
mv "${REVISION_PROVENANCE}.new" "$REVISION_PROVENANCE"
mv "${MANIFEST}.new" "$MANIFEST"
COMMITTED=1
rm -f "$KERNEL_BACKUP" "$ROOTFS_BACKUP" "$MANIFEST_BACKUP"
rm -rf "$MODULES_BACKUP"

printf 'Kernel capabilities repaired and canonical digests rebound\n'
printf 'kernel_sha256=%s\n' "$VMLINUX_SHA"
printf 'module_tree_sha256=%s\n' "$MODULE_TREE_SHA"
printf 'rootfs_sha256=%s\n' "$ROOTFS_SHA"
printf 'kernel_config_sha256=%s\n' "$KERNEL_CONFIG_SHA"
