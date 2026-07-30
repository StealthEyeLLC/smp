#!/usr/bin/env bash
set -euo pipefail

readonly STATE_FILE=/var/lib/smp-init/network.json
readonly TIMEOUT_SECONDS=30

[[ -f "$STATE_FILE" ]]
jq -e '.schemaVersion == 1' "$STATE_FILE" >/dev/null

mac="$(jq -er '.mac' "$STATE_FILE" | tr '[:upper:]' '[:lower:]')"
address="$(jq -er '.address' "$STATE_FILE")"
prefix="$(jq -er '.prefixLength' "$STATE_FILE")"
gateway="$(jq -er '.gateway' "$STATE_FILE")"

interface=
for candidate in /sys/class/net/*; do
  [[ -f "$candidate/address" ]] || continue
  candidate_mac="$(tr '[:upper:]' '[:lower:]' <"$candidate/address")"
  if [[ "$candidate_mac" == "$mac" ]]; then
    interface="${candidate##*/}"
    break
  fi
done
[[ -n "$interface" ]]

link_is_up() {
  ip -o link show dev "$interface" | awk -F'[<>]' '$2 ~ /(^|,)UP(,|$)/ { found = 1 } END { exit(found ? 0 : 1) }'
}

address_is_ready() {
  ip -4 -o address show dev "$interface" scope global | awk -v expected="$address/$prefix" '$4 == expected { found = 1 } END { exit(found ? 0 : 1) }'
}

route_is_ready() {
  ip -4 route show default | awk -v expected_gateway="$gateway" -v expected_interface="$interface" '
    $1 == "default" {
      for (field = 2; field <= NF; field += 1) {
        if ($field == "via" && $(field + 1) == expected_gateway) {
          via = 1
        }
        if ($field == "dev" && $(field + 1) == expected_interface) {
          dev = 1
        }
      }
    }
    END { exit(via && dev ? 0 : 1) }
  '
}

deadline=$((SECONDS + TIMEOUT_SECONDS))
while (( SECONDS <= deadline )); do
  if link_is_up && address_is_ready && route_is_ready; then
    exit 0
  fi
  sleep 1
done

printf 'SMP network readiness timed out after %s seconds for %s (%s)\n' "$TIMEOUT_SECONDS" "$interface" "$mac" >&2
ip -d link show dev "$interface" >&2 || true
ip -4 address show dev "$interface" >&2 || true
ip -4 route show >&2 || true
exit 1
