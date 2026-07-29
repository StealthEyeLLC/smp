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
test_root="$(mktemp -d "$workspace/installer.XXXXXX")"
cleanup() {
  local rc=$?
  trap - EXIT
  [[ "$test_root" == "$workspace"/installer.* && -d "$test_root" && ! -L "$test_root" ]]
  find "$test_root" -xdev -mindepth 1 -delete
  rmdir -- "$test_root"
  exit "$rc"
}
trap cleanup EXIT

repository_root="$(cd -- "$(dirname -- "$0")/.." && pwd -P)"
[[ -d "$repository_root/.git" ]]
[[ -z "$(git -C "$repository_root" status --porcelain=v1 --untracked-files=all)" ]]
commit="$(git -C "$repository_root" rev-parse HEAD)"
tree="$(git -C "$repository_root" rev-parse 'HEAD^{tree}')"
synthetic="$test_root/root"
inputs="$test_root/inputs"
assets="$inputs/assets"
install -d -m 0700 "$synthetic" "$inputs" "$assets"

printf '#!/usr/bin/env bash\nprintf "synthetic-smp-v1\\n"\n' >"$inputs/smp-v1"
printf '#!/usr/bin/env bash\nprintf "synthetic-smp-v2\\n"\n' >"$inputs/smp-v2"
chmod 0755 "$inputs/smp-v1" "$inputs/smp-v2"
printf 'firecracker\n' >"$assets/firecracker"
printf 'kernel\n' >"$assets/vmlinux"
printf 'rootfs\n' >"$assets/debian-13.6-rootfs.ext4"
printf 'seed\n' >"$assets/seed-template.ext4"
chmod 0755 "$assets/firecracker"
jq -n -S '{schemaVersion:1,product:"SMP",architecture:"x86_64"}' >"$assets/manifest.json"

global_before=absent
if [[ -e /usr/local/bin/smp ]]; then
  global_before="$(stat -Lc '%d:%i:%s:%Y' /usr/local/bin/smp)"
fi
args=(
  --root "$synthetic"
  --source "$repository_root"
  --binary "$inputs/smp-v1"
  --assets "$assets"
  --expected-commit "$commit"
  --expected-tree "$tree"
)
bash "$repository_root/scripts/install.sh" "${args[@]}"
first_digest="$(sha256sum "$synthetic/usr/local/bin/smp" | awk '{print $1}')"
[[ "$first_digest" == "$(sha256sum "$inputs/smp-v1" | awk '{print $1}')" ]]
[[ "$(stat -c %a "$synthetic/etc/smp/credentials")" == 700 ]]
[[ "$(stat -c %a "$synthetic/var/lib/smp/machines")" == 700 ]]
cmp --silent "$synthetic/etc/systemd/system/smp.service" \
  "$repository_root/packaging/systemd/smp.service"
cmp --silent "$synthetic/var/lib/smp/assets/manifest.json" "$assets/manifest.json"
jq -e --arg commit "$commit" --arg tree "$tree" \
  '.sourceCommit == $commit and .sourceTree == $tree' \
  "$synthetic/var/lib/smp/provenance/install.json"

bash "$repository_root/scripts/install.sh" "${args[@]}"
[[ "$(sha256sum "$synthetic/usr/local/bin/smp" | awk '{print $1}')" == "$first_digest" ]]

set +e
bash "$repository_root/scripts/install.sh" \
  --root "$synthetic" \
  --source "$repository_root" \
  --binary "$inputs/smp-v2" \
  --assets "$assets" \
  --expected-commit "$commit" \
  --expected-tree "$tree" \
  --simulate-readiness-failure
failure_rc=$?
set -e
[[ "$failure_rc" -eq 70 ]]
[[ "$(sha256sum "$synthetic/usr/local/bin/smp" | awk '{print $1}')" == "$first_digest" ]]
cmp --silent "$synthetic/etc/systemd/system/smp.service" \
  "$repository_root/packaging/systemd/smp.service"

global_after=absent
if [[ -e /usr/local/bin/smp ]]; then
  global_after="$(stat -Lc '%d:%i:%s:%Y' /usr/local/bin/smp)"
fi
[[ "$global_before" == "$global_after" ]]
printf 'isolated installer passed: root=%s commit=%s tree=%s\n' "$synthetic" "$commit" "$tree"
