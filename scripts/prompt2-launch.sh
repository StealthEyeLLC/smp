#!/usr/bin/env bash
set -euo pipefail

[[ $# -eq 1 ]] || {
  printf 'usage: sudo %s 40_HEX_COMMIT\n' "$0" >&2
  exit 2
}
commit="$1"
[[ "$commit" =~ ^[a-f0-9]{40}$ ]] || {
  printf 'commit must be exactly 40 lowercase hexadecimal characters\n' >&2
  exit 2
}
[[ "$(id -u)" -eq 0 ]] || {
  printf 'Prompt-2 launcher requires root\n' >&2
  exit 1
}

repository=https://github.com/StealthEyeLLC/smp.git
run_root="/var/lib/smp/provenance/prompt2"
source_root="$run_root/source-$commit"
log="$run_root/prompt2-$commit.log"
status="$run_root/prompt2-$commit.status"
identity="$run_root/prompt2-$commit.process.json"
unit="smp-prompt2-${commit:0:12}"
install -d -m 0700 "$run_root"
[[ ! -L "$run_root" ]]

if [[ -e "$log" || -e "$status" || -e "$identity" ]]; then
  archive="$run_root/archive-$(date --utc +%Y%m%dT%H%M%SZ)-${commit:0:12}"
  install -d -m 0700 "$archive"
  for old in "$log" "$status" "$identity"; do
    [[ ! -e "$old" ]] || mv -- "$old" "$archive/"
  done
fi
if [[ -d "$source_root" ]]; then
  [[ "$source_root" == "$run_root"/source-* && ! -L "$source_root" ]]
  find "$source_root" -xdev -mindepth 1 -delete
  rmdir -- "$source_root"
fi

git clone --filter=blob:none --no-checkout "$repository" "$source_root"
git -C "$source_root" fetch --depth 1 origin "$commit"
git -C "$source_root" checkout --detach "$commit"
[[ "$(git -C "$source_root" rev-parse HEAD)" == "$commit" ]]
tree="$(git -C "$source_root" rev-parse 'HEAD^{tree}')"
[[ -z "$(git -C "$source_root" status --porcelain=v1 --untracked-files=all)" ]]

: >"$log"
chmod 0600 "$log"
printf 'running\n' >"$status"
chmod 0600 "$status"
systemd-run \
  --unit "$unit" \
  --description "SMP Prompt-2 replacement $commit" \
  --collect \
  --property Type=exec \
  --property WorkingDirectory="$source_root" \
  --property StandardInput=null \
  --property StandardOutput="append:$log" \
  --property StandardError="append:$log" \
  /bin/bash "$source_root/scripts/prompt2-worker.sh" \
  "$commit" "$tree" "$source_root" "$status" "$log"

main_pid=
for _ in {1..50}; do
  main_pid="$(systemctl show --property MainPID --value "$unit")"
  [[ "$main_pid" =~ ^[1-9][0-9]*$ ]] && break
  sleep 0.1
done
[[ "$main_pid" =~ ^[1-9][0-9]*$ ]]
jq -n -S \
  --arg unit "$unit" \
  --argjson pid "$main_pid" \
  --arg commit "$commit" \
  --arg tree "$tree" \
  --arg source "$source_root" \
  --arg log "$log" \
  --arg status "$status" \
  '{schemaVersion:1,unit:$unit,pid:$pid,commit:$commit,tree:$tree,source:$source,log:$log,status:$status}' \
  >"$identity"
chmod 0600 "$identity"
printf 'Prompt-2 detached: unit=%s pid=%s log=%s status=%s\n' "$unit" "$main_pid" "$log" "$status"
