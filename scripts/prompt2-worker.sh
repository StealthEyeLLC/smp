#!/usr/bin/env bash
set -euo pipefail

[[ $# -eq 5 ]] || exit 2
commit="$1"
tree="$2"
source_root="$3"
status_file="$4"
durable_log="$5"
[[ "$commit" =~ ^[a-f0-9]{40}$ && "$tree" =~ ^[a-f0-9]{40}$ ]]
[[ "$source_root" == /* && "$status_file" == /* && "$durable_log" == /* ]]
[[ "$(id -u)" -eq 0 ]]

finish() {
  local rc=$?
  trap - EXIT
  printf '%s\n' "$rc" >"$status_file.tmp"
  chmod 0600 "$status_file.tmp"
  mv -f -- "$status_file.tmp" "$status_file"
  exit "$rc"
}
trap finish EXIT

[[ "$(git -C "$source_root" rev-parse HEAD)" == "$commit" ]]
[[ "$(git -C "$source_root" rev-parse 'HEAD^{tree}')" == "$tree" ]]
[[ -z "$(git -C "$source_root" status --porcelain=v1 --untracked-files=all)" ]]

export CARGO_TERM_COLOR=never
bash "$source_root/scripts/test-repository.sh"
cargo build --locked --release --manifest-path "$source_root/Cargo.toml"
asset_workspace="/var/lib/smp/provenance/prompt2/assets-$commit"
bash "$source_root/scripts/build-assets.sh" \
  --stage all \
  --workspace "$asset_workspace" \
  --release-binary "$source_root/target/release/smp"

archive="/var/backups/smp/replacement-$(date --utc +%Y%m%dT%H%M%SZ)-${commit:0:12}"
bash "$source_root/scripts/replace.sh" \
  --archive "$archive" \
  --installer "$source_root/scripts/install.sh" \
  --source "$source_root" \
  --binary "$source_root/target/release/smp" \
  --assets "$asset_workspace/output" \
  --expected-commit "$commit" \
  --expected-tree "$tree" \
  --destroy-state \
  --start-service

bash "$source_root/scripts/acceptance-host.sh" \
  --commit "$commit" \
  --tree "$tree" \
  --log "$durable_log"
