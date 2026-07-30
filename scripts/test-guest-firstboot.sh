#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "$0")/.." && pwd -P)"
firstboot="$repository_root/assets/guest/smp-firstboot.sh"
firstboot_unit="$repository_root/assets/guest/smp-firstboot.service"
wait_online="$repository_root/assets/guest/smp-wait-online.sh"
wait_online_dropin="$repository_root/assets/guest/10-smp-wait-online.conf"

bash -n "$firstboot"
bash -n "$wait_online"
python3 - "$firstboot" "$firstboot_unit" "$wait_online" "$wait_online_dropin" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
unit_path = Path(sys.argv[2])
wait_online_path = Path(sys.argv[3])
wait_online_dropin_path = Path(sys.argv[4])
text = path.read_text(encoding="utf-8")
unit_text = unit_path.read_text(encoding="utf-8")
wait_online_text = wait_online_path.read_text(encoding="utf-8")
wait_online_dropin_text = wait_online_dropin_path.read_text(encoding="utf-8")

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

required_unit_ordering = "Before=network-pre.target systemd-networkd.service ssh.service"
if required_unit_ordering not in unit_text:
    raise SystemExit("first-boot unit no longer orders before systemd-networkd")

network_contract = [
    r"[Link]\nRequiredForOnline=routable",
    r"LinkLocalAddressing=no",
    r"IPv6AcceptRA=no",
]
for item in network_contract:
    if item not in text:
        raise SystemExit(f"missing deterministic network-online contract: {item}")

state_contract = [
    'install -m 0600 "$SEED_MOUNT/network.json" "$STATE_DIR/network.json"',
    'jq -e \'.schemaVersion == 1\' "$SEED_MOUNT/network.json"',
]
for item in state_contract:
    if item not in text:
        raise SystemExit(f"missing durable network state contract: {item}")

wait_online_contract = [
    "readonly TIMEOUT_SECONDS=30",
    "readonly STATE_FILE=/var/lib/smp-init/network.json",
    "ip -o link show dev",
    "awk -F'[<>]' '$2 ~ /(^|,)UP(,|$)/",
    "ip -4 -o address show dev",
    "ip -4 route show default",
    "for (field = 2; field <= NF; field += 1)",
    "while (( SECONDS <= deadline ))",
    "exit 1",
]
for item in wait_online_contract:
    if item not in wait_online_text:
        raise SystemExit(f"missing bounded wait-online contract: {item}")

expected_dropin = """[Service]
ExecStart=
ExecStart=/usr/lib/smp-guest/smp-wait-online
TimeoutStartSec=35
"""
if wait_online_dropin_text != expected_dropin:
    raise SystemExit("networkd wait-online override changed unexpectedly")

print("guest first-boot restart-network regression passed")
PY

printf '%s\n' '3: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500' \
  | awk -F'[<>]' '$2 ~ /(^|,)UP(,|$)/ { found = 1 } END { exit(found ? 0 : 1) }'

printf '%s\n' 'default via 172.31.1.1 dev eth0' \
  | awk -v expected_gateway='172.31.1.1' -v expected_interface='eth0' '    $1 == "default" {      for (field = 2; field <= NF; field += 1) {        if ($field == "via" && $(field + 1) == expected_gateway) { via = 1 }        if ($field == "dev" && $(field + 1) == expected_interface) { dev = 1 }      }    }    END { exit(via && dev ? 0 : 1) }  '
