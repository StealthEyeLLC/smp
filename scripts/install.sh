#!/usr/bin/env bash
set -euo pipefail

root=/
source_root=
binary=
asset_dir=
expected_commit=
expected_tree=
start_service=false
simulate_readiness_failure=false

usage() {
  printf 'usage: %s --source ABS --binary ABS --assets ABS --expected-commit SHA --expected-tree SHA [--root ABS] [--start-service] [--simulate-readiness-failure]\n' "$0" >&2
  exit 2
}

while (($#)); do
  case "$1" in
    --root)
      (($# >= 2)) || usage
      root="$2"
      shift 2
      ;;
    --source)
      (($# >= 2)) || usage
      source_root="$2"
      shift 2
      ;;
    --binary)
      (($# >= 2)) || usage
      binary="$2"
      shift 2
      ;;
    --assets)
      (($# >= 2)) || usage
      asset_dir="$2"
      shift 2
      ;;
    --expected-commit)
      (($# >= 2)) || usage
      expected_commit="$2"
      shift 2
      ;;
    --expected-tree)
      (($# >= 2)) || usage
      expected_tree="$2"
      shift 2
      ;;
    --start-service)
      start_service=true
      shift
      ;;
    --simulate-readiness-failure)
      simulate_readiness_failure=true
      shift
      ;;
    *)
      usage
      ;;
  esac
done

[[ "$(id -u)" -eq 0 ]] || {
  printf 'SMP installation requires root\n' >&2
  exit 1
}
[[ "$root" == /* && "$root" != *"/../"* && "$root" != */.. ]] || usage
[[ "$source_root" == /* && "$binary" == /* && "$asset_dir" == /* ]] || usage
[[ "$expected_commit" =~ ^[a-f0-9]{40}$ && "$expected_tree" =~ ^[a-f0-9]{40}$ ]] || usage
[[ -d "$source_root/.git" && -x "$binary" && -f "$asset_dir/manifest.json" ]] || usage

root="$(realpath -m -- "$root")"
source_root="$(realpath -e -- "$source_root")"
binary="$(realpath -e -- "$binary")"
asset_dir="$(realpath -e -- "$asset_dir")"
[[ ! -L "$root" ]]

actual_commit="$(git -C "$source_root" rev-parse HEAD)"
actual_tree="$(git -C "$source_root" rev-parse 'HEAD^{tree}')"
[[ "$actual_commit" == "$expected_commit" ]] || {
  printf 'source commit mismatch: expected %s, got %s\n' "$expected_commit" "$actual_commit" >&2
  exit 1
}
[[ "$actual_tree" == "$expected_tree" ]] || {
  printf 'source tree mismatch: expected %s, got %s\n' "$expected_tree" "$actual_tree" >&2
  exit 1
}
[[ -z "$(git -C "$source_root" status --porcelain=v1 --untracked-files=all)" ]] || {
  printf 'source checkout is dirty\n' >&2
  exit 1
}

if [[ "$root" == / ]]; then
  [[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]]
  [[ -c /dev/kvm && -c /dev/net/tun ]]
  for tool in git jq sha256sum install systemctl curl nft ip ssh; do
    command -v "$tool" >/dev/null
  done
fi

target() {
  local absolute="$1"
  [[ "$absolute" == /* ]]
  if [[ "$root" == / ]]; then
    printf '%s\n' "$absolute"
  else
    printf '%s%s\n' "$root" "$absolute"
  fi
}

safe_clear_directory() {
  local directory="$1"
  local allowed_parent="$2"
  [[ "$directory" == "$allowed_parent"/* && "$directory" != "$allowed_parent" && ! -L "$directory" ]]
  if [[ -d "$directory" ]]; then
    find "$directory" -xdev -mindepth 1 -delete
    rmdir -- "$directory"
  fi
}

umask 077
lib="$(target /usr/lib/smp)"
etc="$(target /etc/smp)"
credentials="$(target /etc/smp/credentials)"
state="$(target /var/lib/smp)"
assets="$(target /var/lib/smp/assets)"
machines="$(target /var/lib/smp/machines)"
requests="$(target /var/lib/smp/requests)"
results="$(target /var/lib/smp/results)"
provenance="$(target /var/lib/smp/provenance)"
runtime="$(target /run/smp)"
bin_target="$(target /usr/local/bin/smp)"
unit_dir="$(target /etc/systemd/system)"

install -d -m 0755 "$(dirname -- "$bin_target")" "$lib" "$etc" "$assets" "$unit_dir"
install -d -m 0700 "$credentials" "$state" "$machines" "$requests" "$results" "$provenance"
install -d -m 0755 "$runtime"
for owned_path in "$lib" "$etc" "$credentials" "$state" "$assets" "$machines" \
  "$requests" "$results" "$provenance" "$runtime"; do
  [[ ! -L "$owned_path" ]] || {
    printf 'refusing symlinked installed path: %s\n' "$owned_path" >&2
    exit 1
  }
done

stage="$(mktemp -d "$state/.install.XXXXXX")"
backup="$stage/backup"
install -d -m 0700 "$backup"
rollback_needed=true
cleanup() {
  local rc=$?
  trap - EXIT
  if [[ "$rollback_needed" == true ]]; then
    if [[ -f "$backup/smp" ]]; then
      install -m 0755 "$backup/smp" "$bin_target"
    else
      rm -f -- "$bin_target"
    fi
    for unit in smp.service smp-tunnel.service; do
      if [[ -f "$backup/$unit" ]]; then
        install -m 0644 "$backup/$unit" "$unit_dir/$unit"
      else
        rm -f -- "$unit_dir/$unit"
      fi
    done
  fi
  safe_clear_directory "$stage" "$state"
  exit "$rc"
}
trap cleanup EXIT

[[ ! -f "$bin_target" ]] || install -m 0755 "$bin_target" "$backup/smp"
for unit in smp.service smp-tunnel.service; do
  [[ ! -f "$unit_dir/$unit" ]] || install -m 0644 "$unit_dir/$unit" "$backup/$unit"
done

install -m 0755 "$binary" "$stage/smp.new"
binary_digest="$(sha256sum "$stage/smp.new" | awk '{print $1}')"
[[ "$binary_digest" == "$(sha256sum "$binary" | awk '{print $1}')" ]]
mv -f -- "$stage/smp.new" "$bin_target"

install -d -m 0755 "$lib/scripts" "$lib/plugin" "$lib/tunnel"
find "$source_root/scripts" -maxdepth 1 -type f -name '*.sh' -exec install -m 0755 {} "$lib/scripts/" \;
cp -a -- "$source_root/plugin/." "$lib/plugin/"
find "$lib/plugin" -type d -exec chmod 0755 {} +
find "$lib/plugin" -type f -exec chmod 0644 {} +
install -m 0644 "$source_root/packaging/tunnel/smp-tunnel.yml.example" "$lib/tunnel/"
install -m 0644 "$source_root/packaging/systemd/smp.service" "$unit_dir/smp.service"
install -m 0644 "$source_root/packaging/systemd/smp-tunnel.service" "$unit_dir/smp-tunnel.service"

find "$assets" -xdev -mindepth 1 -delete
cp -a -- "$asset_dir/." "$assets/"
find "$assets" -type d -exec chmod 0755 {} +
find "$assets" -type f ! -name firecracker -exec chmod 0444 {} +
[[ ! -f "$assets/firecracker" ]] || chmod 0555 "$assets/firecracker"
asset_manifest_digest="$(sha256sum "$assets/manifest.json" | awk '{print $1}')"

jq -n -S \
  --arg commit "$actual_commit" \
  --arg tree "$actual_tree" \
  --arg binarySha256 "$binary_digest" \
  --arg assetManifestSha256 "$asset_manifest_digest" \
  --arg installedAt "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
  '{
    schemaVersion:1,
    product:"SMP",
    sourceCommit:$commit,
    sourceTree:$tree,
    binarySha256:$binarySha256,
    assetManifestSha256:$assetManifestSha256,
    installedAt:$installedAt
  }' >"$stage/install.json"
install -m 0600 "$stage/install.json" "$provenance/install.json"

[[ "$(stat -c %a "$credentials")" == 700 ]]
[[ "$(stat -c %a "$machines")" == 700 ]]
[[ "$(sha256sum "$bin_target" | awk '{print $1}')" == "$binary_digest" ]]
[[ "$(sha256sum "$assets/manifest.json" | awk '{print $1}')" == "$asset_manifest_digest" ]]

if [[ "$simulate_readiness_failure" == true ]]; then
  printf 'simulated readiness failure\n' >&2
  exit 70
fi

if [[ "$root" == / ]]; then
  systemctl daemon-reload
  if [[ "$start_service" == true ]]; then
    systemctl enable smp.service
    if systemctl is-active --quiet smp.service; then
      systemctl restart smp.service
    else
      systemctl start smp.service
    fi
    for _ in {1..30}; do
      if curl --fail --silent --unix-socket /run/smp/mcp.sock http://localhost/readyz >/dev/null; then
        break
      fi
      sleep 1
    done
    curl --fail --silent --unix-socket /run/smp/mcp.sock http://localhost/readyz >/dev/null
  fi
fi

rollback_needed=false
printf 'installed SMP commit=%s tree=%s binary=%s assets=%s\n' \
  "$actual_commit" "$actual_tree" "$binary_digest" "$asset_manifest_digest"
