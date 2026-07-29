#!/usr/bin/env bash
set -euo pipefail

root=/
archive=
destroy_state=false
installer=
source_root=
binary=
assets=
expected_commit=
expected_tree=
start_service=false

usage() {
  printf 'usage: %s --archive ABS --installer ABS --source ABS --binary ABS --assets ABS --expected-commit SHA --expected-tree SHA [--root ABS] [--destroy-state] [--start-service]\n' "$0" >&2
  exit 2
}

while (($#)); do
  case "$1" in
    --root)
      (($# >= 2)) || usage
      root="$2"
      shift 2
      ;;
    --archive)
      (($# >= 2)) || usage
      archive="$2"
      shift 2
      ;;
    --installer)
      (($# >= 2)) || usage
      installer="$2"
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
      assets="$2"
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
    --destroy-state)
      destroy_state=true
      shift
      ;;
    --start-service)
      start_service=true
      shift
      ;;
    *)
      usage
      ;;
  esac
done

[[ "$(id -u)" -eq 0 ]] || {
  printf 'SMP replacement requires root\n' >&2
  exit 1
}
[[ "$root" == /* && "$archive" == /* && "$installer" == /* ]] || usage
[[ "$source_root" == /* && "$binary" == /* && "$assets" == /* ]] || usage
[[ "$expected_commit" =~ ^[a-f0-9]{40}$ && "$expected_tree" =~ ^[a-f0-9]{40}$ ]] || usage
[[ -f "$installer" ]]
root="$(realpath -m -- "$root")"
archive="$(realpath -m -- "$archive")"
[[ "$archive" != "$root" ]]
if [[ "$root" == / ]]; then
  [[ "$archive" != /usr/lib/smp/* && "$archive" != /etc/smp/* && "$archive" != /run/smp/* ]]
else
  [[ "$archive" != "$root/usr/lib/smp"/* && "$archive" != "$root/etc/smp"/* ]]
fi
install -d -m 0700 "$archive"
[[ ! -L "$archive" ]]

target() {
  local absolute="$1"
  if [[ "$root" == / ]]; then
    printf '%s\n' "$absolute"
  else
    printf '%s%s\n' "$root" "$absolute"
  fi
}

bin_target="$(target /usr/local/bin/smp)"
lib="$(target /usr/lib/smp)"
etc="$(target /etc/smp)"
state="$(target /var/lib/smp)"
runtime="$(target /run/smp)"
unit_dir="$(target /etc/systemd/system)"

for path in "$bin_target" "$lib" "$etc" "$state" "$runtime" \
  "$unit_dir/smp.service" "$unit_dir/smp-tunnel.service"; do
  [[ ! -L "$path" ]] || {
    printf 'refusing symlinked SMP-owned path: %s\n' "$path" >&2
    exit 1
  }
done

old_binary_digest=
[[ ! -f "$bin_target" ]] || old_binary_digest="$(sha256sum "$bin_target" | awk '{print $1}')"
service_state=synthetic
tunnel_state=synthetic
if [[ "$root" == / ]]; then
  service_state="$(systemctl is-active smp.service 2>/dev/null || true)"
  tunnel_state="$(systemctl is-active smp-tunnel.service 2>/dev/null || true)"
fi
jq -n -S \
  --arg recordedAt "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
  --arg binarySha256 "$old_binary_digest" \
  --arg serviceState "$service_state" \
  --arg tunnelState "$tunnel_state" \
  --argjson destroyState "$destroy_state" \
  '{recordedAt:$recordedAt,binarySha256:$binarySha256,serviceState:$serviceState,tunnelState:$tunnelState,destroyState:$destroyState}' \
  >"$archive/pre-replacement.json"

archive_paths=()
for relative in usr/local/bin/smp usr/lib/smp etc/smp etc/systemd/system/smp.service \
  etc/systemd/system/smp-tunnel.service run/smp; do
  [[ ! -e "$root/$relative" ]] || archive_paths+=("$relative")
done
if ((${#archive_paths[@]})); then
  tar --create --preserve-permissions --file "$archive/owned-files.tar" \
    --directory "$root" "${archive_paths[@]}"
fi
if [[ -d "$state" ]]; then
  (
    cd "$root"
    if [[ "$root" == / ]]; then
      state_relative="${state#/}"
    else
      state_relative="${state#"$root"/}"
    fi
    find "$state_relative" -xdev -type f \
      \( -name machine.json -o -name install.json -o -name '*.status' -o -name '*.log' \) \
      -size -16M -print0 |
      tar --null --files-from=- --create --file "$archive/state-diagnostics.tar"
  )
fi
if [[ "$root" == / ]]; then
  {
    ip -details link show
    nft list ruleset
  } >"$archive/network-before.txt" 2>&1
fi

verify_live_processes() {
  [[ -d "$state/machines" ]] || return 0
  while IFS= read -r record; do
    pid="$(jq -er '.firecrackerProcess.pid // empty' "$record" 2>/dev/null || true)"
    [[ -n "$pid" && -e "/proc/$pid" ]] || continue
    expected_exe="$(jq -er '.firecrackerProcess.executablePath' "$record")"
    expected_start="$(jq -er '.firecrackerProcess.processStartTime' "$record")"
    expected_digest="$(jq -r '.firecrackerProcess.executableDigest // empty' "$record")"
    actual_exe="$(readlink -f -- "/proc/$pid/exe")"
    stat_tail="$(sed -E 's/^[0-9]+ \\([^)]*\\) //' "/proc/$pid/stat")"
    actual_start="$(awk '{print $20}' <<<"$stat_tail")"
    [[ "$actual_exe" == "$expected_exe" && "$actual_start" == "$expected_start" ]] || {
      printf 'ambiguous live process in %s\n' "$record" >&2
      exit 1
    }
    if [[ -n "$expected_digest" ]]; then
      [[ "$(sha256sum "$actual_exe" | awk '{print $1}')" == "$expected_digest" ]] || {
        printf 'live process executable digest mismatch in %s\n' "$record" >&2
        exit 1
      }
    fi
  done < <(find "$state/machines" -mindepth 2 -maxdepth 2 -type f -name machine.json -print)
}

stop_verified_processes() {
  [[ -d "$state/machines" ]] || return 0
  while IFS= read -r record; do
    pid="$(jq -er '.firecrackerProcess.pid // empty' "$record" 2>/dev/null || true)"
    [[ -n "$pid" && -e "/proc/$pid" ]] || continue
    kill -TERM "$pid"
    for _ in {1..100}; do
      [[ -e "/proc/$pid" ]] || break
      sleep 0.1
    done
    if [[ -e "/proc/$pid" ]]; then
      kill -KILL "$pid"
    fi
  done < <(find "$state/machines" -mindepth 2 -maxdepth 2 -type f -name machine.json -print)
}

cleanup_verified_network() {
  [[ "$root" == / && -d "$state/machines" ]] || return 0
  while IFS= read -r record; do
    machine="$(jq -er .machineId "$record")"
    tap="$(jq -er .network.tap "$record")"
    if ip link show dev "$tap" >/dev/null 2>&1; then
      ip -d link show dev "$tap" | grep -Fq "alias smp:$machine:" || {
        printf 'ambiguous TAP ownership: %s\n' "$tap" >&2
        exit 1
      }
      ip link delete dev "$tap"
    fi
  done < <(find "$state/machines" -mindepth 2 -maxdepth 2 -type f -name machine.json -print)
  if nft list table inet smp >/dev/null 2>&1; then
    rules="$(nft list table inet smp)"
    if grep -E 'comment "' <<<"$rules" | grep -Ev 'comment "smp:' >/dev/null; then
      printf 'ambiguous rules in inet smp table\n' >&2
      exit 1
    fi
    nft delete table inet smp
  fi
}

verify_live_processes
if [[ "$root" == / ]]; then
  systemctl stop smp-tunnel.service 2>/dev/null || true
  systemctl stop smp.service 2>/dev/null || true
fi
stop_verified_processes
cleanup_verified_network

rm -f -- "$bin_target" "$unit_dir/smp.service" "$unit_dir/smp-tunnel.service"
for directory in "$lib" "$etc" "$runtime"; do
  if [[ -d "$directory" ]]; then
    find "$directory" -xdev -mindepth 1 -delete
    rmdir -- "$directory"
  fi
done
prompt2_recovery=
if [[ "$destroy_state" == true && -d "$state/provenance/prompt2" ]]; then
  prompt2_recovery="$archive/prompt2-runtime"
  mv -- "$state/provenance/prompt2" "$prompt2_recovery"
fi
if [[ "$destroy_state" == true && -d "$state" ]]; then
  find "$state" -xdev -mindepth 1 -delete
  rmdir -- "$state"
fi
if [[ -n "$prompt2_recovery" ]]; then
  install -d -m 0700 "$state/provenance"
  mv -- "$prompt2_recovery" "$state/provenance/prompt2"
fi

install_args=(
  --root "$root"
  --source "$source_root"
  --binary "$binary"
  --assets "$assets"
  --expected-commit "$expected_commit"
  --expected-tree "$expected_tree"
)
if [[ "$start_service" == true ]]; then
  install_args+=(--start-service)
fi
bash "$installer" "${install_args[@]}"
printf 'replacement archive: %s\n' "$archive"
