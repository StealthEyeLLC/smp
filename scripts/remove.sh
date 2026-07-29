#!/usr/bin/env bash
set -euo pipefail

root=/
level=
destroy_state=false

usage() {
  printf 'usage: %s --level plugin|tunnel|service|binary|complete [--root ABS] [--destroy-state]\n' "$0" >&2
  exit 2
}
while (($#)); do
  case "$1" in
    --root)
      (($# >= 2)) || usage
      root="$2"
      shift 2
      ;;
    --level)
      (($# >= 2)) || usage
      level="$2"
      shift 2
      ;;
    --destroy-state)
      destroy_state=true
      shift
      ;;
    *)
      usage
      ;;
  esac
done
[[ "$(id -u)" -eq 0 ]] || {
  printf 'SMP removal requires root\n' >&2
  exit 1
}
[[ "$root" == /* && "$level" =~ ^(plugin|tunnel|service|binary|complete)$ ]] || usage
if [[ "$level" == complete && "$destroy_state" != true ]]; then
  printf 'complete removal requires --destroy-state\n' >&2
  exit 2
fi
root="$(realpath -m -- "$root")"
target() {
  if [[ "$root" == / ]]; then
    printf '%s\n' "$1"
  else
    printf '%s%s\n' "$root" "$1"
  fi
}
safe_remove_directory() {
  local directory="$1"
  [[ "$directory" == /* && "$directory" != / && ! -L "$directory" ]]
  if [[ "$root" != / ]]; then
    [[ "$directory" == "$root"/* && "$directory" != "$root" ]]
  fi
  [[ ! -d "$directory" ]] || {
    find "$directory" -xdev -mindepth 1 -delete
    rmdir -- "$directory"
  }
}
unit_dir="$(target /etc/systemd/system)"
lib="$(target /usr/lib/smp)"
etc="$(target /etc/smp)"
state="$(target /var/lib/smp)"
runtime="$(target /run/smp)"
binary="$(target /usr/local/bin/smp)"

if [[ "$root" == / ]]; then
  case "$level" in
    tunnel) systemctl stop smp-tunnel.service 2>/dev/null || true ;;
    service|binary|complete)
      systemctl stop smp-tunnel.service 2>/dev/null || true
      systemctl stop smp.service 2>/dev/null || true
      ;;
    plugin) ;;
  esac
fi
case "$level" in
  plugin)
    safe_remove_directory "$lib/plugin"
    ;;
  tunnel)
    rm -f -- "$unit_dir/smp-tunnel.service" "$etc/tunnel.yml"
    ;;
  service)
    rm -f -- "$unit_dir/smp.service" "$unit_dir/smp-tunnel.service"
    safe_remove_directory "$runtime"
    ;;
  binary)
    rm -f -- "$unit_dir/smp.service" "$unit_dir/smp-tunnel.service" "$binary"
    safe_remove_directory "$lib"
    safe_remove_directory "$runtime"
    ;;
  complete)
    rm -f -- "$unit_dir/smp.service" "$unit_dir/smp-tunnel.service" "$binary"
    safe_remove_directory "$lib"
    safe_remove_directory "$etc"
    safe_remove_directory "$runtime"
    safe_remove_directory "$state"
    ;;
esac
if [[ "$root" == / ]]; then
  systemctl daemon-reload
fi
printf 'SMP removal level %s complete\n' "$level"
