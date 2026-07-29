#!/usr/bin/env bash
set -euo pipefail

commit=
usage() {
  printf 'usage: sudo %s 40_HEX_COMMIT\n' "$0" >&2
  exit 2
}
[[ $# -eq 1 ]] || usage
commit="$1"
[[ "$commit" =~ ^[a-f0-9]{40}$ ]] || usage
[[ "$(id -u)" -eq 0 ]] || {
  printf 'SMP upgrade requires root\n' >&2
  exit 1
}
[[ -f /var/lib/smp/provenance/install.json ]] || {
  printf 'installed SMP provenance is missing\n' >&2
  exit 1
}
old_commit="$(jq -er .sourceCommit /var/lib/smp/provenance/install.json)"
printf 'upgrading SMP from %s to %s\n' "$old_commit" "$commit"
exec bash "$(cd -- "$(dirname -- "$0")" && pwd -P)/bootstrap.sh" \
  --commit "$commit" \
  --start-service
