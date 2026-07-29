#!/usr/bin/env bash
set -euo pipefail

readonly KERNEL_VERSION=6.1.178
readonly SOURCE_URL="https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${KERNEL_VERSION}.tar.xz"
readonly CHECKSUMS_URL="https://cdn.kernel.org/pub/linux/kernel/v6.x/sha256sums.asc"

usage() {
  printf 'usage: %s WORK_DIRECTORY OUTPUT_CONFIG\n' "$0" >&2
  exit 2
}

[[ $# -eq 2 ]] || usage
work_directory="$1"
output_config="$2"
[[ "$work_directory" == /* && "$output_config" == /* ]] || usage

umask 077
mkdir -p "$work_directory"
archive="$work_directory/linux-${KERNEL_VERSION}.tar.xz"
checksums="$work_directory/sha256sums.asc"
source_directory="$work_directory/linux-${KERNEL_VERSION}"
fragment="$(cd -- "$(dirname -- "$0")/.." && pwd -P)/config/kernel/smp-${KERNEL_VERSION}.fragment"

curl --fail --location --proto '=https' --tlsv1.2 --output "$checksums.part" "$CHECKSUMS_URL"
mv -f "$checksums.part" "$checksums"
expected_digest="$(awk -v name="linux-${KERNEL_VERSION}.tar.xz" '$2 == name {print $1}' "$checksums")"
[[ "$expected_digest" =~ ^[a-f0-9]{64}$ ]]

if [[ -f "$archive" ]]; then
  actual_digest="$(sha256sum "$archive" | awk '{print $1}')"
  if [[ "$actual_digest" != "$expected_digest" ]]; then
    rm -f "$archive"
  fi
fi
if [[ ! -f "$archive" ]]; then
  curl --fail --location --proto '=https' --tlsv1.2 --output "$archive.part" "$SOURCE_URL"
  printf '%s  %s\n' "$expected_digest" "$archive.part" | sha256sum --check -
  mv -f "$archive.part" "$archive"
fi

if [[ ! -d "$source_directory" ]]; then
  tar --extract --xz --file "$archive" --directory "$work_directory"
fi
cd "$source_directory"
make mrproper
make x86_64_defconfig
while IFS= read -r setting; do
  [[ "$setting" =~ ^CONFIG_([A-Za-z0-9_]+)=(y|m|n)$ ]] || continue
  symbol="${BASH_REMATCH[1]}"
  value="${BASH_REMATCH[2]}"
  case "$value" in
    y) scripts/config --enable "$symbol" ;;
    m) scripts/config --module "$symbol" ;;
    n) scripts/config --disable "$symbol" ;;
  esac
done <"$fragment"
make olddefconfig
install -D -m 0644 .config "$output_config"
printf 'kernel_version=%s\nsource_sha256=%s\nconfig_sha256=%s\n' \
  "$KERNEL_VERSION" "$expected_digest" "$(sha256sum "$output_config" | awk '{print $1}')"
