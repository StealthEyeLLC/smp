#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if cargo fmt --version >/dev/null 2>&1; then
    cargo fmt --all -- --check
fi
cargo test --all-targets
if cargo clippy --version >/dev/null 2>&1; then
    cargo clippy --all-targets
fi

for script in scripts/*.sh assets/guest/*.sh; do
    bash -n "$script"
done
if command -v shellcheck >/dev/null; then
    shellcheck scripts/*.sh assets/guest/*.sh
fi

jq -e '.displayName == "SMP" and .toolContract.onlyTool == "go" and .toolContract.canonicalCallableIdentity == "smp.go"' plugin/SMP.json >/dev/null
jq -e '.properties.schemaVersion.const == 1 and (.required | index("operation")) != null' plugin/smp.go.schema.json >/dev/null

TOOLS_LIST_COUNT="$(grep -R --include='*.rs' -F '"name": "go"' src/server.rs | wc -l)"
[[ "$TOOLS_LIST_COUNT" -eq 1 ]] || {
    printf 'expected one MCP tool registration, found %s\n' "$TOOLS_LIST_COUNT" >&2
    exit 1
}
! grep -R -nE '(Baby|Fix|Horsey|Quirt).*(socket|service|credential|state|endpoint)' src scripts packaging plugin --exclude='test-repository.sh'

printf 'repository tests passed\n'
