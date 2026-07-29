#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CARGO_BIN="${SMP_CARGO_BIN:-cargo}"
LOCK_ARGS=()
if [[ ${SMP_CARGO_LOCKED:-0} == 1 ]]; then
    LOCK_ARGS+=(--locked)
fi

[[ -x $CARGO_BIN || $CARGO_BIN == cargo ]] || {
    printf 'Cargo executable is unavailable: %s\n' "$CARGO_BIN" >&2
    exit 1
}

cargo_run() {
    "$CARGO_BIN" "$@"
}

if [[ ${SMP_CARGO_LOCKED:-0} == 1 ]]; then
    [[ -f Cargo.lock ]] || { printf 'Cargo.lock is required for locked repository checks\n' >&2; exit 1; }
    cargo_run metadata --format-version 1 "${LOCK_ARGS[@]}" >/dev/null
fi

cargo_run test --all-targets "${LOCK_ARGS[@]}"

for script in scripts/*.sh assets/guest/*.sh; do
    bash -n "$script"
done
if command -v shellcheck >/dev/null; then
    shellcheck --severity=error scripts/*.sh assets/guest/*.sh
fi

jq -e '.displayName == "SMP" and .toolContract.onlyTool == "go" and .toolContract.canonicalCallableIdentity == "smp.go"' plugin/SMP.json >/dev/null
jq -e '.properties.schemaVersion.const == 1 and (.required | index("operation")) != null' plugin/smp.go.schema.json >/dev/null

grep -Fq '/proc/sys/kernel/hostname' assets/guest/smp-seed-init.sh
! grep -Fq 'hostnamectl set-hostname' assets/guest/smp-seed-init.sh
grep -Fq 'repair-rootfs.sh' scripts/bootstrap.sh
grep -Fq 'MACAddress=$MAC' assets/guest/smp-seed-init.sh

TOOLS_LIST_COUNT="$(grep -R --include='*.rs' -F '"name": "go"' src/server.rs | wc -l)"
[[ "$TOOLS_LIST_COUNT" -eq 1 ]] || {
    printf 'expected one MCP tool registration, found %s\n' "$TOOLS_LIST_COUNT" >&2
    exit 1
}
! grep -R -nE '(Baby|Fix|Horsey|Quirt).*(socket|service|credential|state|endpoint)' src scripts packaging plugin --exclude='test-repository.sh'

printf 'repository tests passed\n'
