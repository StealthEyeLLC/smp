#!/bin/bash
set -euo pipefail

STATE=/var/lib/smp-seed
MOUNT=/run/smp-seed
STATUS="$STATE/status"

mkdir -p "$STATE" "$MOUNT"
if [[ -f "$STATE/complete" ]]; then
    exit 0
fi

fail() {
    local message=$*
    printf 'failed: %s\n' "$message" | tee "$STATUS" >&2
    exit 1
}
trap 'fail "line $LINENO"' ERR

DEVICE="$(blkid -L SMP_SEED || true)"
[[ -n "$DEVICE" ]] || fail 'SMP_SEED filesystem not found'
mount -o ro,nosuid,nodev,noexec "$DEVICE" "$MOUNT"
trap 'umount "$MOUNT" >/dev/null 2>&1 || true' EXIT

for required in hostname authorized_keys network.env; do
    [[ -f "$MOUNT/$required" ]] || fail "missing $required"
done

HOSTNAME_VALUE="$(tr -d '\r\n' < "$MOUNT/hostname")"
[[ "$HOSTNAME_VALUE" =~ ^[a-z][a-z0-9-]{0,62}$ ]] || fail 'invalid hostname'
printf '%s\n' "$HOSTNAME_VALUE" > /etc/hostname
if [[ -w /proc/sys/kernel/hostname ]]; then
    printf '%s\n' "$HOSTNAME_VALUE" > /proc/sys/kernel/hostname || true
fi

if [[ ! -s /etc/machine-id ]]; then
    systemd-machine-id-setup
fi
rm -f /etc/ssh/ssh_host_*
ssh-keygen -A

install -d -m 0700 /root/.ssh
install -m 0600 "$MOUNT/authorized_keys" /root/.ssh/authorized_keys

# shellcheck disable=SC1090
source "$MOUNT/network.env"
[[ "$ADDRESS" =~ ^[0-9.]+/[0-9]+$ ]] || fail 'invalid ADDRESS'
[[ "$GATEWAY" =~ ^[0-9.]+$ ]] || fail 'invalid GATEWAY'
[[ "$DNS" =~ ^[0-9.,]+$ ]] || fail 'invalid DNS'
[[ "$MAC" =~ ^([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}$ ]] || fail 'invalid MAC'
IFS=',' read -r -a DNS_SERVERS <<<"$DNS"
(( ${#DNS_SERVERS[@]} > 0 )) || fail 'empty DNS list'
for server in "${DNS_SERVERS[@]}"; do
    [[ "$server" =~ ^[0-9.]+$ ]] || fail "invalid DNS server $server"
done

cat > /etc/systemd/network/10-smp.network <<NETWORK
[Match]
MACAddress=$MAC

[Network]
Address=$ADDRESS
Gateway=$GATEWAY
DNS=${DNS//,/ }
IPv6AcceptRA=no
NETWORK

INTERFACE=
for candidate in /sys/class/net/*; do
    [[ -r "$candidate/address" ]] || continue
    if [[ "$(tr '[:upper:]' '[:lower:]' < "$candidate/address")" == "${MAC,,}" ]]; then
        INTERFACE="${candidate##*/}"
        break
    fi
done
[[ -n "$INTERFACE" ]] || fail "network interface for MAC $MAC not found"
ip link set dev "$INTERFACE" up
ip address replace "$ADDRESS" dev "$INTERFACE"
ip route replace default via "$GATEWAY" dev "$INTERFACE"

rm -f /etc/resolv.conf
{
    for server in "${DNS_SERVERS[@]}"; do
        printf 'nameserver %s\n' "$server"
    done
    printf 'options timeout:2 attempts:3\n'
} > /etc/resolv.conf
chmod 0644 /etc/resolv.conf

if [[ -f "$MOUNT/files.tar" ]]; then
    tar --extract --file "$MOUNT/files.tar" --directory / --no-same-owner --numeric-owner
fi
if [[ -f "$MOUNT/init.sh" ]]; then
    install -m 0700 "$MOUNT/init.sh" "$STATE/init.sh"
    "$STATE/init.sh"
fi

printf 'complete\n' > "$STATUS"
touch "$STATE/complete"
sync
