#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets

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
