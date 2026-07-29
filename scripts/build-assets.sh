#!/usr/bin/env bash
set -euo pipefail

readonly FIRECRACKER_VERSION=1.15.1
readonly FIRECRACKER_ARCH=x86_64
readonly FIRECRACKER_ARCHIVE="firecracker-v${FIRECRACKER_VERSION}-${FIRECRACKER_ARCH}.tgz"
readonly FIRECRACKER_URL="https://github.com/firecracker-microvm/firecracker/releases/download/v${FIRECRACKER_VERSION}/${FIRECRACKER_ARCHIVE}"
readonly FIRECRACKER_CHECKSUM_URL="${FIRECRACKER_URL}.sha256.txt"
readonly KERNEL_VERSION=6.1.178
readonly KERNEL_ARCHIVE="linux-${KERNEL_VERSION}.tar.xz"
readonly KERNEL_URL="https://cdn.kernel.org/pub/linux/kernel/v6.x/${KERNEL_ARCHIVE}"
readonly KERNEL_CHECKSUMS_URL="https://cdn.kernel.org/pub/linux/kernel/v6.x/sha256sums.asc"
readonly DEBIAN_VERSION=13.6
readonly DEBIAN_SUITE=trixie
readonly DEBIAN_ARCH=amd64
readonly DEBIAN_SNAPSHOT=20260711T000000Z
readonly DEBIAN_REPOSITORY="https://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}/"
readonly DEBIAN_SECURITY_REPOSITORY="https://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}/"
readonly DEBIAN_KEYRING_VERSION=2025.1
readonly DEBIAN_KEYRING_PACKAGE="debian-archive-keyring_${DEBIAN_KEYRING_VERSION}_all.deb"
readonly DEBIAN_KEYRING_PATH="pool/main/d/debian-archive-keyring/${DEBIAN_KEYRING_PACKAGE}"
readonly DEBIAN_KEYRING_URL="${DEBIAN_REPOSITORY}${DEBIAN_KEYRING_PATH}"
readonly DEBIAN_KEYRING_SHA256=9ea7778e443144ca490668737a8ab22dd3e748bb99e805e22ec055abeb3c7fac
readonly ROOTFS_UUID=53504d31-0000-4000-8000-000000000001
readonly SEED_TEMPLATE_UUID=53504d31-0000-4000-8000-000000000002

stage=all
release_binary=
asset_workspace="${SMP_ASSET_WORKSPACE:-}"

usage() {
  printf 'usage: %s [--stage all|firecracker|kernel|rootfs] --workspace ABSOLUTE_PATH [--release-binary ABSOLUTE_PATH]\n' "$0" >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --stage)
      [[ $# -ge 2 ]] || usage
      stage="$2"
      shift 2
      ;;
    --workspace)
      [[ $# -ge 2 ]] || usage
      asset_workspace="$2"
      shift 2
      ;;
    --release-binary)
      [[ $# -ge 2 ]] || usage
      release_binary="$2"
      shift 2
      ;;
    *)
      usage
      ;;
  esac
done
[[ "$stage" =~ ^(all|firecracker|kernel|rootfs)$ ]] || usage
[[ "$asset_workspace" == /* && "$asset_workspace" != / && "$asset_workspace" != /tmp ]] || usage
if [[ "$stage" == all || "$stage" == rootfs ]]; then
  [[ "$release_binary" == /* && -x "$release_binary" ]] || {
    printf '%s\n' '--release-binary must name the final executable for rootfs construction' >&2
    exit 2
  }
fi

REPOSITORY_ROOT="$(cd -- "$(dirname -- "$0")/.." && pwd -P)"
readonly REPOSITORY_ROOT
readonly ASSET_WORKSPACE="$asset_workspace"
readonly CACHE="$ASSET_WORKSPACE/cache"
readonly BUILD="$ASSET_WORKSPACE/build"
readonly OUTPUT="$ASSET_WORKSPACE/output"
readonly LOGS="$ASSET_WORKSPACE/logs"
readonly PROVENANCE="$ASSET_WORKSPACE/provenance"
readonly MANIFEST="$OUTPUT/manifest.json"

umask 077
install -d -m 0700 "$ASSET_WORKSPACE" "$CACHE" "$BUILD" "$LOGS" "$PROVENANCE"
install -d -m 0755 "$OUTPUT"
[[ "$(realpath -e "$ASSET_WORKSPACE")" == "$ASSET_WORKSPACE" ]]

safe_remove() {
  local candidate="$1"
  [[ "$candidate" == "$ASSET_WORKSPACE"/* && "$candidate" != "$ASSET_WORKSPACE" ]]
  if [[ -L "$candidate" ]]; then
    printf 'refusing symlink cleanup: %s\n' "$candidate" >&2
    return 1
  fi
  if [[ -e "$candidate" ]]; then
    rm -rf --one-file-system -- "$candidate"
  fi
}

download_verified() {
  local url="$1"
  local expected="$2"
  local destination="$3"
  if [[ -f "$destination" ]]; then
    local current
    current="$(sha256sum "$destination" | awk '{print $1}')"
    if [[ "$current" != "$expected" ]]; then
      rm -f -- "$destination"
    fi
  fi
  if [[ ! -f "$destination" ]]; then
    curl --fail --location --proto '=https' --tlsv1.2 --output "$destination.part" "$url"
    printf '%s  %s\n' "$expected" "$destination.part" | sha256sum --check - >/dev/null
    mv -f -- "$destination.part" "$destination"
  fi
}

build_firecracker() {
  local archive="$CACHE/$FIRECRACKER_ARCHIVE"
  local checksum_file="$CACHE/${FIRECRACKER_ARCHIVE}.sha256.txt"
  curl --fail --location --proto '=https' --tlsv1.2 --output "$checksum_file.part" "$FIRECRACKER_CHECKSUM_URL"
  mv -f -- "$checksum_file.part" "$checksum_file"
  local archive_digest
  archive_digest="$(awk -v name="$FIRECRACKER_ARCHIVE" '$2 == name || $2 == ("*" name) {print $1}' "$checksum_file")"
  if [[ ! "$archive_digest" =~ ^[a-f0-9]{64}$ ]]; then
    archive_digest="$(awk 'NF >= 1 && $1 ~ /^[a-f0-9]{64}$/ {print $1; exit}' "$checksum_file")"
  fi
  [[ "$archive_digest" =~ ^[a-f0-9]{64}$ ]]
  download_verified "$FIRECRACKER_URL" "$archive_digest" "$archive"

  local extract="$BUILD/firecracker"
  safe_remove "$extract"
  install -d -m 0700 "$extract"
  tar --extract --gzip --file "$archive" --directory "$extract"
  mapfile -t candidates < <(find "$extract" -type f -name "firecracker-v${FIRECRACKER_VERSION}-${FIRECRACKER_ARCH}" -print)
  [[ "${#candidates[@]}" -eq 1 ]]
  local source_binary="${candidates[0]}"
  chmod 0755 "$source_binary"
  local version_output
  version_output="$("$source_binary" --version)"
  [[ "$version_output" == *"Firecracker v${FIRECRACKER_VERSION}"* ]]
  install -m 0755 "$source_binary" "$OUTPUT/firecracker"
  local binary_digest
  binary_digest="$(sha256sum "$OUTPUT/firecracker" | awk '{print $1}')"
  chmod 0555 "$OUTPUT/firecracker"
  jq -n -S \
    --arg version "$FIRECRACKER_VERSION" \
    --arg architecture "$FIRECRACKER_ARCH" \
    --arg sourceUrl "$FIRECRACKER_URL" \
    --arg archiveSha256 "$archive_digest" \
    --arg binarySha256 "$binary_digest" \
    --arg versionOutput "$version_output" \
    --arg buildTimestamp "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    '{
      version:$version,
      architecture:$architecture,
      sourceUrl:$sourceUrl,
      archiveSha256:$archiveSha256,
      binaryPath:"firecracker",
      binarySha256:$binarySha256,
      versionOutput:$versionOutput,
      buildTimestamp:$buildTimestamp
    }' >"$PROVENANCE/firecracker.json"
}

kernel_source_digest() {
  local checksums="$CACHE/kernel-sha256sums.asc"
  curl --fail --location --proto '=https' --tlsv1.2 --output "$checksums.part" "$KERNEL_CHECKSUMS_URL"
  mv -f -- "$checksums.part" "$checksums"
  awk -v name="$KERNEL_ARCHIVE" '$2 == name {print $1}' "$checksums"
}

build_kernel() {
  local expected_digest
  expected_digest="$(kernel_source_digest)"
  [[ "$expected_digest" =~ ^[a-f0-9]{64}$ ]]
  local archive="$CACHE/$KERNEL_ARCHIVE"
  download_verified "$KERNEL_URL" "$expected_digest" "$archive"
  local source="$BUILD/linux-${KERNEL_VERSION}"
  safe_remove "$source"
  tar --extract --xz --file "$archive" --directory "$BUILD"
  local committed_config="$REPOSITORY_ROOT/config/kernel/linux-${KERNEL_VERSION}.config"
  [[ -f "$committed_config" ]]
  install -m 0600 "$committed_config" "$source/.config"
  make -C "$source" olddefconfig
  cmp --silent "$committed_config" "$source/.config"
  local jobs="${SMP_BUILD_JOBS:-2}"
  [[ "$jobs" =~ ^[1-9][0-9]*$ ]]
  make -C "$source" --jobs "$jobs" vmlinux modules
  local modules_root="$BUILD/kernel-modules"
  safe_remove "$modules_root"
  make -C "$source" modules_install INSTALL_MOD_PATH="$modules_root"
  find "$modules_root" -type l -delete
  install -m 0644 "$source/vmlinux" "$OUTPUT/vmlinux"
  tar --sort=name --mtime='UTC 2026-07-24' --owner=0 --group=0 --numeric-owner \
    --create --xz --file "$OUTPUT/kernel-modules.tar.xz" --directory "$modules_root" .
  local config_digest
  local kernel_digest
  local module_tree_digest
  local modules_archive_digest
  config_digest="$(sha256sum "$committed_config" | awk '{print $1}')"
  kernel_digest="$(sha256sum "$OUTPUT/vmlinux" | awk '{print $1}')"
  module_tree_digest="$(
    find "$modules_root" -type f -print0 |
      LC_ALL=C sort -z |
      xargs -0 sha256sum |
      sha256sum |
      awk '{print $1}'
  )"
  modules_archive_digest="$(sha256sum "$OUTPUT/kernel-modules.tar.xz" | awk '{print $1}')"
  file "$OUTPUT/vmlinux" | grep -q 'ELF 64-bit LSB executable, x86-64'
  chmod 0444 "$OUTPUT/vmlinux" "$OUTPUT/kernel-modules.tar.xz"
  jq -n -S \
    --arg version "$KERNEL_VERSION" \
    --arg sourceDate "2026-07-24" \
    --arg sourceUrl "$KERNEL_URL" \
    --arg sourceSha256 "$expected_digest" \
    --arg configSha256 "$config_digest" \
    --arg vmlinuxSha256 "$kernel_digest" \
    --arg moduleTreeSha256 "$module_tree_digest" \
    --arg modulesArchiveSha256 "$modules_archive_digest" \
    --arg buildTimestamp "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    '{
      version:$version,
      sourceDate:$sourceDate,
      sourceUrl:$sourceUrl,
      sourceSha256:$sourceSha256,
      configPath:"config/kernel/linux-6.1.178.config",
      configSha256:$configSha256,
      vmlinuxPath:"vmlinux",
      vmlinuxSha256:$vmlinuxSha256,
      moduleTreeSha256:$moduleTreeSha256,
      modulesArchivePath:"kernel-modules.tar.xz",
      modulesArchiveSha256:$modulesArchiveSha256,
      buildTimestamp:$buildTimestamp
    }' >"$PROVENANCE/kernel.json"
}

configure_rootfs() {
  local root="$1"
  install -d -m 0755 "$root/usr/lib/smp-guest" "$root/etc/systemd/system" "$root/etc/ssh/sshd_config.d"
  install -m 0755 "$REPOSITORY_ROOT/assets/guest/smp-firstboot.sh" "$root/usr/lib/smp-guest/smp-firstboot"
  install -m 0644 "$REPOSITORY_ROOT/assets/guest/smp-firstboot.service" "$root/etc/systemd/system/smp-firstboot.service"
  install -m 0755 "$release_binary" "$root/usr/local/libexec/smp"
  cat >"$root/etc/ssh/sshd_config.d/10-smp-root.conf" <<'EOF'
PermitRootLogin prohibit-password
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
EOF
  printf 'smp\n' >"$root/etc/hostname"
  printf 'nameserver 1.1.1.1\nnameserver 9.9.9.9\noptions timeout:2 attempts:3\n' >"$root/etc/resolv.conf"
  rm -f -- "$root/etc/machine-id" "$root/var/lib/dbus/machine-id"
  : >"$root/etc/machine-id"
  rm -f -- "$root"/etc/ssh/ssh_host_*_key "$root"/etc/ssh/ssh_host_*_key.pub
  chroot "$root" systemctl enable systemd-networkd.service ssh.service smp-firstboot.service
  chroot "$root" passwd -d root
}

prepare_debian_keyring() {
  local package="$CACHE/$DEBIAN_KEYRING_PACKAGE"
  local extracted="$BUILD/debian-archive-keyring"
  download_verified "$DEBIAN_KEYRING_URL" "$DEBIAN_KEYRING_SHA256" "$package"
  safe_remove "$extracted"
  install -d -m 0700 "$extracted"
  dpkg-deb --extract "$package" "$extracted"
  local keyring="$extracted/usr/share/keyrings/debian-archive-keyring.gpg"
  [[ -s "$keyring" ]]
  printf '%s\n' "$keyring"
}

build_rootfs() {
  [[ -f "$PROVENANCE/kernel.json" ]]
  local root="$BUILD/rootfs-directory"
  local keyring
  local keyring_digest
  keyring="$(prepare_debian_keyring)"
  keyring_digest="$(sha256sum "$keyring" | awk '{print $1}')"
  safe_remove "$root"
  install -d -m 0755 "$root"
  debootstrap \
    --arch="$DEBIAN_ARCH" \
    --variant=minbase \
    --keyring="$keyring" \
    "$DEBIAN_SUITE" "$root" "$DEBIAN_REPOSITORY"
  cat >"$root/etc/apt/sources.list" <<EOF
deb [check-valid-until=no] $DEBIAN_REPOSITORY $DEBIAN_SUITE main
deb [check-valid-until=no] $DEBIAN_SECURITY_REPOSITORY ${DEBIAN_SUITE}-security main
EOF
  cat >"$root/etc/apt/apt.conf.d/99smp-snapshot" <<'EOF'
Acquire::Check-Valid-Until "false";
Acquire::Retries "3";
APT::Install-Recommends "false";
EOF
  cat >"$root/usr/sbin/policy-rc.d" <<'EOF'
#!/bin/sh
exit 101
EOF
  chmod 0755 "$root/usr/sbin/policy-rc.d"
  cp -L /etc/resolv.conf "$root/etc/resolv.conf"
  chroot "$root" apt-get update
  chroot "$root" env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    systemd systemd-sysv dbus openssh-server nftables iproute2 dnsutils ca-certificates curl \
    e2fsprogs util-linux procps bash coreutils findutils grep sed gawk tar gzip xz-utils \
    iputils-ping kmod jq gcc libc6-dev make pkg-config
  chroot "$root" apt-get clean
  find "$root/var/lib/apt/lists" -mindepth 1 -delete
  install -d -m 0755 "$root/var/lib/apt/lists"
  tar --extract --xz --file "$OUTPUT/kernel-modules.tar.xz" --directory "$root"
  configure_rootfs "$root"
  local package_list="$PROVENANCE/debian-packages.txt"
  local dpkg_format="\${Package}\t\${Version}\n"
  chroot "$root" dpkg-query -W -f="$dpkg_format" | LC_ALL=C sort >"$package_list"
  grep -qx "$DEBIAN_VERSION" "$root/etc/debian_version"

  local main_inrelease="$PROVENANCE/debian-main-InRelease"
  local security_inrelease="$PROVENANCE/debian-security-InRelease"
  curl --fail --location --proto '=https' --tlsv1.2 \
    --output "$main_inrelease" "${DEBIAN_REPOSITORY}dists/${DEBIAN_SUITE}/InRelease"
  curl --fail --location --proto '=https' --tlsv1.2 \
    --output "$security_inrelease" "${DEBIAN_SECURITY_REPOSITORY}dists/${DEBIAN_SUITE}-security/InRelease"

  local rootfs="$OUTPUT/debian-${DEBIAN_VERSION}-rootfs.ext4"
  rm -f -- "$rootfs"
  truncate --size 4G "$rootfs"
  mkfs.ext4 -F -L SMP_ROOT -U "$ROOTFS_UUID" -d "$root" "$rootfs"
  e2fsck -fn "$rootfs"

  local seed_root="$BUILD/seed-template-directory"
  safe_remove "$seed_root"
  install -d -m 0700 "$seed_root"
  printf '{"schemaVersion":1,"template":true}\n' >"$seed_root/manifest.json"
  printf 'smp\n' >"$seed_root/hostname"
  printf '{"schemaVersion":1,"mac":"06:00:00:00:00:01","address":"172.31.1.2","prefixLength":30,"gateway":"172.31.1.1","dns":["1.1.1.1","9.9.9.9"]}\n' >"$seed_root/network.json"
  : >"$seed_root/authorized_keys"
  local seed="$OUTPUT/seed-template.ext4"
  rm -f -- "$seed"
  truncate --size 16M "$seed"
  mkfs.ext4 -F -L SMP_SEED -U "$SEED_TEMPLATE_UUID" -d "$seed_root" "$seed"
  e2fsck -fn "$seed"

  local rootfs_digest
  local seed_digest
  local helper_digest
  local package_digest
  rootfs_digest="$(sha256sum "$rootfs" | awk '{print $1}')"
  seed_digest="$(sha256sum "$seed" | awk '{print $1}')"
  helper_digest="$(sha256sum "$release_binary" | awk '{print $1}')"
  package_digest="$(sha256sum "$package_list" | awk '{print $1}')"
  chmod 0444 "$rootfs" "$seed"
  jq -n -S \
    --arg version "$DEBIAN_VERSION" \
    --arg suite "$DEBIAN_SUITE" \
    --arg architecture "$DEBIAN_ARCH" \
    --arg snapshotTimestamp "$DEBIAN_SNAPSHOT" \
    --arg repository "$DEBIAN_REPOSITORY" \
    --arg securityRepository "$DEBIAN_SECURITY_REPOSITORY" \
    --arg keyringVersion "$DEBIAN_KEYRING_VERSION" \
    --arg keyringUrl "$DEBIAN_KEYRING_URL" \
    --arg keyringPackageSha256 "$DEBIAN_KEYRING_SHA256" \
    --arg keyringSha256 "$keyring_digest" \
    --arg mainInReleaseSha256 "$(sha256sum "$main_inrelease" | awk '{print $1}')" \
    --arg securityInReleaseSha256 "$(sha256sum "$security_inrelease" | awk '{print $1}')" \
    --arg packageListSha256 "$package_digest" \
    --arg filesystemUuid "$ROOTFS_UUID" \
    --argjson filesystemSize "$(stat -c %s "$rootfs")" \
    --arg rootfsSha256 "$rootfs_digest" \
    --arg seedTemplateUuid "$SEED_TEMPLATE_UUID" \
    --arg seedTemplateSha256 "$seed_digest" \
    --arg guestHelperSha256 "$helper_digest" \
    --arg moduleTreeSha256 "$(jq -r .moduleTreeSha256 "$PROVENANCE/kernel.json")" \
    --arg buildTimestamp "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    '{
      version:$version,
      suite:$suite,
      architecture:$architecture,
      snapshotTimestamp:$snapshotTimestamp,
      repositories:[$repository,$securityRepository],
      archiveKeyring:{
        version:$keyringVersion,
        sourceUrl:$keyringUrl,
        packageSha256:$keyringPackageSha256,
        keyringSha256:$keyringSha256
      },
      inReleaseSha256:[$mainInReleaseSha256,$securityInReleaseSha256],
      packageListPath:"provenance/debian-packages.txt",
      packageListSha256:$packageListSha256,
      filesystemUuid:$filesystemUuid,
      filesystemSize:$filesystemSize,
      rootfsPath:"debian-13.6-rootfs.ext4",
      rootfsSha256:$rootfsSha256,
      seedTemplatePath:"seed-template.ext4",
      seedTemplateUuid:$seedTemplateUuid,
      seedTemplateSha256:$seedTemplateSha256,
      guestHelperSha256:$guestHelperSha256,
      moduleTreeSha256:$moduleTreeSha256,
      buildTimestamp:$buildTimestamp
    }' >"$PROVENANCE/debian.json"
}

write_manifest() {
  [[ -f "$PROVENANCE/firecracker.json" ]]
  [[ -f "$PROVENANCE/kernel.json" ]]
  [[ -f "$PROVENANCE/debian.json" ]]
  jq -n -S \
    --slurpfile firecracker "$PROVENANCE/firecracker.json" \
    --slurpfile kernel "$PROVENANCE/kernel.json" \
    --slurpfile debian "$PROVENANCE/debian.json" \
    '{
      schemaVersion:1,
      product:"SMP",
      architecture:"x86_64",
      firecracker:$firecracker[0],
      kernel:$kernel[0],
      debian:$debian[0]
    }' >"$MANIFEST.tmp"
  mv -f -- "$MANIFEST.tmp" "$MANIFEST"
  sha256sum "$MANIFEST" >"$OUTPUT/manifest.sha256"
  chmod 0444 "$MANIFEST" "$OUTPUT/manifest.sha256"
}

case "$stage" in
  firecracker)
    build_firecracker
    ;;
  kernel)
    build_kernel
    ;;
  rootfs)
    build_rootfs
    write_manifest
    ;;
  all)
    build_firecracker
    build_kernel
    build_rootfs
    write_manifest
    ;;
esac
