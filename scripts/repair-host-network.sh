#!/bin/bash
set -euo pipefail
umask 077

MACHINE=${1:-smp-cert-persistent}
[[ $(id -u) -eq 0 ]] || { printf 'repair-host-network.sh requires root\n' >&2; exit 77; }
for tool in ip iptables jq smp sysctl; do
    command -v "$tool" >/dev/null || { printf 'missing host network repair tool: %s\n' "$tool" >&2; exit 69; }
done

STATUS="$(smp status "$MACHINE" --json)"
jq -e '.state == "ready" and (.process.pid | type == "number")' <<<"$STATUS" >/dev/null || {
    printf 'machine is not ready: %s\n' "$MACHINE" >&2
    exit 75
}
TAP="$(jq -er '.network.tapName' <<<"$STATUS")"
GUEST="$(jq -er '.network.guestAddress' <<<"$STATUS")"
PREFIX="$(jq -er '.network.prefixLength' <<<"$STATUS")"
GATEWAY="$(jq -er '.network.gatewayAddress' <<<"$STATUS")"
OUTBOUND="$(ip -o route show default | awk 'NR == 1 {for (i=1; i<=NF; i++) if ($i == "dev") {print $(i+1); exit}}')"
[[ -n $OUTBOUND ]] || { printf 'host default route has no interface\n' >&2; exit 69; }

IFS=. read -r A B C _ <<<"$GATEWAY"
SUBNET="$A.$B.$C.0/$PREFIX"
sysctl -q -w net.ipv4.ip_forward=1

ensure_rule() {
    local table=$1 chain=$2
    shift 2
    if [[ $table == filter ]]; then
        iptables -C "$chain" "$@" >/dev/null 2>&1 || iptables -I "$chain" 1 "$@"
    else
        iptables -t "$table" -C "$chain" "$@" >/dev/null 2>&1 || iptables -t "$table" -I "$chain" 1 "$@"
    fi
}

# Insert into the host's canonical iptables-nft chains so these rules precede
# a distribution firewall's default FORWARD drop.
ensure_rule filter FORWARD -i "$TAP" -j ACCEPT
ensure_rule filter FORWARD -o "$TAP" -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
ensure_rule nat POSTROUTING -s "$SUBNET" -o "$OUTBOUND" -j MASQUERADE

# Locally generated traffic does not traverse PREROUTING. Mirror every SMP
# published TCP/UDP port into nat OUTPUT so localhost publication is real.
while IFS=$'\t' read -r protocol host_port guest_port; do
    [[ -n $protocol ]] || continue
    ensure_rule nat OUTPUT -p "$protocol" -d 127.0.0.1 --dport "$host_port" -j DNAT --to-destination "$GUEST:$guest_port"
done < <(jq -r '.network.publishedPorts[]? | [.protocol, (.hostPort|tostring), (.guestPort|tostring)] | @tsv' <<<"$STATUS")

printf 'Host forwarding repaired for %s via %s -> %s\n' "$MACHINE" "$TAP" "$OUTBOUND"

# Prove routed IP connectivity before DNS is tested by acceptance.
for _ in $(seq 1 10); do
    if smp exec "$MACHINE" -- ping -c 1 -W 2 1.1.1.1 >/dev/null 2>&1; then
        printf 'Guest direct IPv4 connectivity verified\n'
        exit 0
    fi
    sleep 1
done
printf 'guest still cannot reach 1.1.1.1 after host forwarding repair\n' >&2
exit 70
