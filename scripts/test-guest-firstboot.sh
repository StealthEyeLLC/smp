#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "$0")/.." && pwd -P)"
firstboot="$repository_root/assets/guest/smp-firstboot.sh"

bash -n "$firstboot"
python3 - "$firstboot" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")

forbidden = '''if [[ -f "$SUCCESS_FILE" ]]; then
  exit 0
fi'''
if forbidden in text:
    raise SystemExit("successful first boot still bypasses all later boot configuration")

markers = {
    "initialized": '''already_initialized=0
if [[ -f "$SUCCESS_FILE" ]]; then
  already_initialized=1
fi''',
    "identity_guard": '''if [[ "$already_initialized" -eq 0 ]]; then
  rm -f /etc/machine-id''',
    "network": '''ip route replace default via "$gateway" dev "$interface"''',
    "repeat_exit": '''if [[ "$already_initialized" -eq 1 ]]; then
  rm -f "$FAILURE_FILE"
  exit 0
fi''',
    "init": '''if [[ -f "$SEED_MOUNT/init.sh" ]]; then''',
    "success": '''>"$SUCCESS_FILE.tmp"''',
}
positions = {}
for name, marker in markers.items():
    position = text.find(marker)
    if position < 0:
        raise SystemExit(f"missing first-boot regression marker: {name}")
    positions[name] = position

expected = [
    positions["initialized"],
    positions["identity_guard"],
    positions["network"],
    positions["repeat_exit"],
    positions["init"],
    positions["success"],
]
if expected != sorted(expected):
    raise SystemExit(f"first-boot ordering regression: {positions}")

print("guest first-boot restart-network regression passed")
PY
