#!/bin/bash
set -euo pipefail
umask 077

SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXPECTED_COMMIT=${1:-}

[[ $(id -u) -eq 0 ]] || { printf 'recover-firecracker-acceptance.sh requires root\n' >&2; exit 77; }
[[ $EXPECTED_COMMIT =~ ^[0-9a-f]{40}$ ]] || { printf 'expected full recovery commit SHA\n' >&2; exit 64; }
OBSERVED_COMMIT="$(git -C "$SOURCE_ROOT" rev-parse HEAD)"
[[ $OBSERVED_COMMIT == "$EXPECTED_COMMIT" ]] || {
    printf 'recovery source mismatch: expected %s, observed %s\n' "$EXPECTED_COMMIT" "$OBSERVED_COMMIT" >&2
    exit 65
}
[[ -x /usr/local/bin/smp ]] || { printf 'installed SMP binary is unavailable\n' >&2; exit 69; }
[[ -r /var/lib/smp/assets/manifest.json ]] || { printf 'SMP asset manifest is unavailable\n' >&2; exit 66; }

if ! command -v iptables >/dev/null; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y --no-install-recommends iptables
fi

install -d -m 0755 /usr/lib/smp /usr/lib/smp/assets/guest
install -m 0755 "$SOURCE_ROOT/scripts/repair-rootfs.sh" /usr/lib/smp/repair-rootfs.sh
install -m 0755 "$SOURCE_ROOT/scripts/repair-host-network.sh" /usr/lib/smp/repair-host-network.sh
install -m 0755 "$SOURCE_ROOT/scripts/acceptance.sh" /usr/lib/smp/acceptance.sh
install -m 0755 "$SOURCE_ROOT/scripts/prompt2-handoff.sh" /usr/lib/smp/prompt2-handoff.sh
install -m 0755 "$SOURCE_ROOT/assets/guest/smp-seed-init.sh" /usr/lib/smp/assets/guest/smp-seed-init.sh
install -m 0644 "$SOURCE_ROOT/assets/guest/smp-seed-init.service" /usr/lib/smp/assets/guest/smp-seed-init.service

REVISION=/var/lib/smp/assets/provenance/kernel-capabilities-revision.json
[[ -r $REVISION ]] || { printf 'corrected kernel capability provenance is unavailable\n' >&2; exit 66; }
jq -e '
  .schemaVersion == 1 and
  (.enabledCapabilities | index("CONFIG_NF_TABLES_INET") != null) and
  (.enabledCapabilities | index("CONFIG_NF_TABLES_IPV4") != null) and
  (.enabledCapabilities | index("CONFIG_DUMMY") != null)' "$REVISION" >/dev/null || {
    printf 'corrected kernel capability provenance is incomplete\n' >&2
    exit 65
}
/usr/local/bin/smp --json assets >/dev/null

rm -f /var/lib/smp/results/acceptance/result.json
PRIMARY_STATUS="$(/usr/local/bin/smp status smp-cert-persistent --json 2>/dev/null || true)"
if jq -e '.state == "ready" and (.process.pid | type == "number")' <<<"$PRIMARY_STATUS" >/dev/null 2>&1; then
    printf 'Restarting the existing corrected persistent VM to clear failed-run process state\n'
    /usr/local/bin/smp stop smp-cert-persistent >/dev/null
    /usr/local/bin/smp start smp-cert-persistent >/dev/null
    /usr/local/bin/smp wait smp-cert-persistent --timeout-seconds 180 >/dev/null
    /usr/lib/smp/repair-host-network.sh smp-cert-persistent
    printf 'Resuming complete acceptance on the clean persistent VM\n'
    /usr/lib/smp/acceptance.sh --resume-primary
else
    printf 'Recreating certification machines from the already-corrected canonical assets\n'
    for machine in smp-cert-disposable smp-cert-no-fallback smp-cert-isolated smp-cert-persistent; do
        /usr/local/bin/smp destroy "$machine" --force >/dev/null 2>&1 || true
    done
    /usr/lib/smp/acceptance.sh
fi

jq -e '.result == "PASS"' /var/lib/smp/results/acceptance/result.json >/dev/null
systemctl is-active --quiet smp.service
curl --fail --silent --show-error http://127.0.0.1:7745/healthz >/dev/null
curl --fail --silent --show-error http://127.0.0.1:7745/readyz >/dev/null
/usr/lib/smp/prompt2-handoff.sh | tee /var/lib/smp/provenance/prompt2-handoff.json

printf 'SMP targeted recovery complete\n'
printf 'recovery_commit=%s\n' "$EXPECTED_COMMIT"
printf 'acceptance_result=%s\n' "$(jq -r .result /var/lib/smp/results/acceptance/result.json)"
printf 'acceptance_evidence=/var/lib/smp/results/acceptance/result.json\n'
printf 'prompt2_handoff=/var/lib/smp/provenance/prompt2-handoff.json\n'
