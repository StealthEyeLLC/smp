#!/usr/bin/env bash
# shellcheck disable=SC2016
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "$0")/.." && pwd -P)"
installer="$repository_root/scripts/install.sh"
bootstrap="$repository_root/scripts/bootstrap.sh"
replacement="$repository_root/scripts/replace.sh"

grep -Fq '[[ "$(id -u)" -eq 0 ]]' "$installer"
grep -Fq 'expected_commit' "$installer"
grep -Fq 'expected_tree' "$installer"
grep -Fq 'status --porcelain=v1 --untracked-files=all' "$installer"
grep -Fq 'mv -f -- "$stage/smp.new" "$bin_target"' "$installer"
grep -Fq 'simulate_readiness_failure' "$installer"
grep -Fq 'systemctl daemon-reload' "$installer"
grep -Fq 'git -C "$workspace/source" checkout --detach "$commit"' "$bootstrap"
grep -Fq 'verify_live_processes' "$replacement"
grep -Fq 'ambiguous live process' "$replacement"
eval_pattern='(^|[;&|])[[:space:]]*e''val([[:space:]]|$)'
pipe_pattern='cu''rl[^[:cntrl:]]*\|[[:space:]]*(ba)?sh'
if grep -REn "$eval_pattern" \
  "$repository_root/scripts" "$repository_root/assets/guest"; then
  exit 1
fi
if grep -REn "$pipe_pattern" "$repository_root/scripts"; then
  exit 1
fi
printf 'installer static assertions passed\n'
