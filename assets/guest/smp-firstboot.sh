#!/usr/bin/env bash
set -euo pipefail

readonly STATE_DIR=/var/lib/smp-init
readonly SUCCESS_FILE="$STATE_DIR/success.json"
readonly FAILURE_FILE="$STATE_DIR/failure.txt"
readonly SEED_MOUNT=/run/smp-seed

umask 077
mkdir -p "$STATE_DIR" "$SEED_MOUNT"

fail() {
  local rc=$?
  trap - ERR
  printf 'smp first boot failed at line %s with status %s\n' "${BASH_LINENO[0]:-unknown}" "$rc" >"$FAILURE_FILE"
  exit "$rc"
}
trap fail ERR

already_initialized=0
if [[ -f "$SUCCESS_FILE" ]]; then
  already_initialized=1
fi

seed_device="$(findfs LABEL=SMP_SEED)"
mount -o ro,nosuid,nodev,noexec "$seed_device" "$SEED_MOUNT"
cleanup() {
  mountpoint -q "$SEED_MOUNT" && umount "$SEED_MOUNT"
}
trap cleanup EXIT

for required in manifest.json hostname network.json authorized_keys; do
  [[ -f "$SEED_MOUNT/$required" ]]
done
jq -e '.schemaVersion == 1' "$SEED_MOUNT/manifest.json" >/dev/null

if [[ "$already_initialized" -eq 0 ]]; then
  rm -f /etc/machine-id /var/lib/dbus/machine-id
  systemd-machine-id-setup
  ln -sf /etc/machine-id /var/lib/dbus/machine-id
  rm -f /etc/ssh/ssh_host_*_key /etc/ssh/ssh_host_*_key.pub
  ssh-keygen -A

  install -d -m 0700 /root/.ssh
  install -m 0600 "$SEED_MOUNT/authorized_keys" /root/.ssh/authorized_keys
  hostname="$(tr -d '\r\n' <"$SEED_MOUNT/hostname")"
  [[ "$hostname" =~ ^[A-Za-z0-9][A-Za-z0-9-]{0,62}$ ]]
  printf '%s\n' "$hostname" >/etc/hostname
  hostnamectl --static set-hostname "$hostname"
fi

mac="$(jq -er '.mac' "$SEED_MOUNT/network.json" | tr '[:upper:]' '[:lower:]')"
address="$(jq -er '.address' "$SEED_MOUNT/network.json")"
prefix="$(jq -er '.prefixLength' "$SEED_MOUNT/network.json")"
gateway="$(jq -er '.gateway' "$SEED_MOUNT/network.json")"
mapfile -t dns_servers < <(jq -er '.dns[]' "$SEED_MOUNT/network.json")

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

install -d -m 0755 /etc/systemd/network
network_file=/etc/systemd/network/10-smp.network
{
  printf '[Match]\nMACAddress=%s\n\n[Network]\nAddress=%s/%s\nGateway=%s\n' "$mac" "$address" "$prefix" "$gateway"
  for dns in "${dns_servers[@]}"; do
    printf 'DNS=%s\n' "$dns"
  done
} >"$network_file"
chmod 0644 "$network_file"

ip link set dev "$interface" up
ip address flush dev "$interface"
ip address add "$address/$prefix" dev "$interface"
ip route replace default via "$gateway" dev "$interface"
{
  for dns in "${dns_servers[@]}"; do
    printf 'nameserver %s\n' "$dns"
  done
  printf 'options timeout:2 attempts:3\n'
} >/etc/resolv.conf

if [[ "$already_initialized" -eq 1 ]]; then
  rm -f "$FAILURE_FILE"
  exit 0
fi

if [[ -f "$SEED_MOUNT/files.json" ]]; then
  jq -e 'type == "array"' "$SEED_MOUNT/files.json" >/dev/null
  while IFS=$'\t' read -r source destination mode; do
    [[ "$source" =~ ^[A-Za-z0-9._/-]+$ ]]
    [[ "$destination" == /* && "$destination" != *"/../"* && "$destination" != */.. ]]
    [[ "$mode" =~ ^0[0-7]{3}$ ]]
    install -D -m "$mode" "$SEED_MOUNT/files/$source" "$destination"
  done < <(jq -r '.[] | [.source, .destination, .mode] | @tsv' "$SEED_MOUNT/files.json")
fi

if [[ -f "$SEED_MOUNT/init.sh" ]]; then
  install -m 0700 "$SEED_MOUNT/init.sh" "$STATE_DIR/init.sh"
  /bin/bash "$STATE_DIR/init.sh"
fi

hostname="$(cat /etc/hostname)"
printf '{"schemaVersion":1,"status":"ready","machineId":"%s","hostname":"%s","interface":"%s"}\n' \
  "$(cat /etc/machine-id)" "$hostname" "$interface" >"$SUCCESS_FILE.tmp"
chmod 0600 "$SUCCESS_FILE.tmp"
mv -f "$SUCCESS_FILE.tmp" "$SUCCESS_FILE"
rm -f "$FAILURE_FILE"
