#!/bin/bash
set -euo pipefail
umask 077

SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXPECTED_COMMIT=${1:-}

[[ $(id -u) -eq 0 ]] || { printf 'recover-kernel-and-acceptance.sh requires root\n' >&2; exit 77; }
[[ $EXPECTED_COMMIT =~ ^[0-9a-f]{40}$ ]] || { printf 'expected full recovery commit SHA\n' >&2; exit 64; }
OBSERVED_COMMIT="$(git -C "$SOURCE_ROOT" rev-parse HEAD)"
[[ $OBSERVED_COMMIT == "$EXPECTED_COMMIT" ]] || {
    printf 'recovery source mismatch: expected %s, observed %s\n' "$EXPECTED_COMMIT" "$OBSERVED_COMMIT" >&2
    exit 65
}
[[ -x /usr/local/bin/smp ]] || { printf 'installed SMP binary is unavailable\n' >&2; exit 69; }
[[ -r /var/lib/smp/assets/manifest.json ]] || { printf 'SMP asset manifest is unavailable\n' >&2; exit 66; }

install -d -m 0755 /usr/lib/smp
for script in module-tree-digest.sh repair-kernel-capabilities.sh repair-host-network.sh acceptance.sh prompt2-handoff.sh; do
    install -m 0755 "$SOURCE_ROOT/scripts/$script" "/usr/lib/smp/$script"
done

rm -f /var/lib/smp/results/acceptance/result.json
printf 'Repairing canonical Linux kernel capabilities only\n'
/usr/lib/smp/repair-kernel-capabilities.sh "$EXPECTED_COMMIT"

printf 'Verifying rebound Firecracker, kernel, module, and rootfs assets\n'
/usr/local/bin/smp --json assets | tee /var/lib/smp/provenance/assets.json >/dev/null
jq -e \
  '.schemaVersion == 1 and .kernel.version == "6.1.177" and
   (.kernel.sha256 | test("^[0-9a-f]{64}$")) and
   (.moduleTreeSha256 | test("^[0-9a-f]{64}$")) and
   (.rootfs.sha256 | test("^[0-9a-f]{64}$"))' \
  /var/lib/smp/assets/manifest.json >/dev/null

printf 'Running complete real Firecracker acceptance with corrected kernel\n'
/usr/lib/smp/acceptance.sh
jq -e '.result == "PASS"' /var/lib/smp/results/acceptance/result.json >/dev/null
systemctl is-active --quiet smp.service
curl --fail --silent --show-error http://127.0.0.1:7745/healthz >/dev/null
curl --fail --silent --show-error http://127.0.0.1:7745/readyz >/dev/null
/usr/lib/smp/prompt2-handoff.sh | tee /var/lib/smp/provenance/prompt2-handoff.json

printf 'SMP kernel recovery and acceptance complete\n'
printf 'recovery_commit=%s\n' "$EXPECTED_COMMIT"
printf 'acceptance_result=%s\n' "$(jq -r .result /var/lib/smp/results/acceptance/result.json)"
printf 'kernel_sha256=%s\n' "$(jq -r .kernel.sha256 /var/lib/smp/assets/manifest.json)"
printf 'module_tree_sha256=%s\n' "$(jq -r .moduleTreeSha256 /var/lib/smp/assets/manifest.json)"
printf 'rootfs_sha256=%s\n' "$(jq -r .rootfs.sha256 /var/lib/smp/assets/manifest.json)"
printf 'acceptance_evidence=/var/lib/smp/results/acceptance/result.json\n'
printf 'prompt2_handoff=/var/lib/smp/provenance/prompt2-handoff.json\n'
