#!/usr/bin/env bash
set -euo pipefail

workspace=
usage() {
  printf 'usage: %s --workspace ABS\n' "$0" >&2
  exit 2
}
while (($#)); do
  case "$1" in
    --workspace)
      (($# >= 2)) || usage
      workspace="$2"
      shift 2
      ;;
    *)
      usage
      ;;
  esac
done
[[ "$workspace" == /* && "$workspace" != / && "$workspace" != /tmp ]]
workspace="$(realpath -m -- "$workspace")"
install -d -m 0700 "$workspace"
test_root="$(mktemp -d "$workspace/replacement.XXXXXX")"
cleanup() {
  local rc=$?
  trap - EXIT
  [[ "$test_root" == "$workspace"/replacement.* && -d "$test_root" && ! -L "$test_root" ]]
  find "$test_root" -xdev -mindepth 1 -delete
  rmdir -- "$test_root"
  exit "$rc"
}
trap cleanup EXIT

repository_root="$(cd -- "$(dirname -- "$0")/.." && pwd -P)"
[[ -z "$(git -C "$repository_root" status --porcelain=v1 --untracked-files=all)" ]]
commit="$(git -C "$repository_root" rev-parse HEAD)"
tree="$(git -C "$repository_root" rev-parse 'HEAD^{tree}')"
inputs="$test_root/inputs"
asset_dir="$inputs/assets"
install -d -m 0700 "$asset_dir"
printf '#!/usr/bin/env bash\nexit 0\n' >"$inputs/smp"
chmod 0755 "$inputs/smp"
printf '{}\n' >"$asset_dir/manifest.json"

run_replacement() {
  local synthetic="$1"
  local archive="$2"
  shift 2
  install -d -m 0755 "$synthetic/usr/local/bin" "$synthetic/usr/lib/smp" \
    "$synthetic/etc/smp" "$synthetic/etc/systemd/system" "$synthetic/run/smp"
  install -d -m 0700 "$synthetic/var/lib/smp/machines/persistent"
  printf 'old\n' >"$synthetic/usr/local/bin/smp"
  printf 'persistent\n' >"$synthetic/var/lib/smp/machines/persistent/root.ext4"
  printf 'unrelated service\n' >"$synthetic/etc/systemd/system/unrelated.service"
  install -d -m 0700 "$synthetic/var/lib/unrelated"
  printf 'unrelated state\n' >"$synthetic/var/lib/unrelated/state"
  bash "$repository_root/scripts/replace.sh" \
    --root "$synthetic" \
    --archive "$archive" \
    --installer "$repository_root/scripts/install.sh" \
    --source "$repository_root" \
    --binary "$inputs/smp" \
    --assets "$asset_dir" \
    --expected-commit "$commit" \
    --expected-tree "$tree" \
    "$@"
  [[ -f "$archive/pre-replacement.json" && -f "$archive/owned-files.tar" ]]
  [[ -f "$synthetic/etc/systemd/system/unrelated.service" ]]
  [[ -f "$synthetic/var/lib/unrelated/state" ]]
}

preserve_root="$test_root/preserve-root"
run_replacement "$preserve_root" "$test_root/archive-preserve"
[[ -f "$preserve_root/var/lib/smp/machines/persistent/root.ext4" ]]

destroy_root="$test_root/destroy-root"
run_replacement "$destroy_root" "$test_root/archive-destroy" --destroy-state
[[ ! -e "$destroy_root/var/lib/smp/machines/persistent/root.ext4" ]]

ambiguous_root="$test_root/ambiguous-root"
install -d -m 0755 "$ambiguous_root/usr/local/bin"
install -d -m 0700 "$ambiguous_root/var/lib/smp/machines/ambiguous"
printf 'old\n' >"$ambiguous_root/usr/local/bin/smp"
jq -n -S \
  --argjson pid "$$" \
  '{firecrackerProcess:{pid:$pid,executablePath:"/bin/false",processStartTime:1,executableDigest:null}}' \
  >"$ambiguous_root/var/lib/smp/machines/ambiguous/machine.json"
set +e
bash "$repository_root/scripts/replace.sh" \
  --root "$ambiguous_root" \
  --archive "$test_root/archive-ambiguous" \
  --installer "$repository_root/scripts/install.sh" \
  --source "$repository_root" \
  --binary "$inputs/smp" \
  --assets "$asset_dir" \
  --expected-commit "$commit" \
  --expected-tree "$tree"
ambiguous_rc=$?
set -e
[[ "$ambiguous_rc" -ne 0 ]]
grep -qx old "$ambiguous_root/usr/local/bin/smp"
printf 'replacement safety passed\n'
