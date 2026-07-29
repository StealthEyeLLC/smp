#!/bin/bash
set -euo pipefail
umask 077

MACHINE=${1:-}
[[ $(id -u) -eq 0 ]] || { printf 'repair-host-network.sh requires root\n' >&2; exit 77; }
[[ $MACHINE =~ ^[a-z][a-z0-9-]{0,62}$ ]] || { printf 'expected a canonical machine name\n' >&2; exit 64; }
for tool in ip iptables iptables-save jq sha256sum smp; do
    command -v "$tool" >/dev/null || { printf 'missing host network audit tool: %s\n' "$tool" >&2; exit 69; }
done

STATUS="$(smp status "$MACHINE" --json)"
jq -e '.state == "ready" and (.process.pid | type == "number") and .network.managed == true' <<<"$STATUS" >/dev/null || {
    printf 'machine is not a ready managed-network machine: %s\n' "$MACHINE" >&2
    exit 75
}

TAP="$(jq -er '.network.tapName' <<<"$STATUS")"
GUEST="$(jq -er '.network.guestAddress' <<<"$STATUS")"
GATEWAY="$(jq -er '.network.gatewayAddress' <<<"$STATUS")"
PREFIX="$(jq -er '.network.prefixLength' <<<"$STATUS")"
SUFFIX="$(printf '%s' "$MACHINE" | sha256sum | cut -c1-10)"
IFS=. read -r A B C _ <<<"$GATEWAY"
SUBNET="$A.$B.$C.0/$PREFIX"
OUTBOUND="$(ip -o route show default | awk 'NR == 1 {for (i=1; i<=NF; i++) if ($i == "dev") {print $(i+1); exit}}')"
[[ -n $OUTBOUND ]] || { printf 'host default route has no interface\n' >&2; exit 69; }

ip link show dev "$TAP" >/dev/null
[[ $(cat /proc/sys/net/ipv4/ip_forward) == 1 ]] || { printf 'IPv4 forwarding is disabled\n' >&2; exit 70; }

check_rule() {
    local table=$1
    shift
    if [[ $table == filter ]]; then
        iptables -w 5 "$@"
    else
        iptables -w 5 -t "$table" "$@"
    fi
}

check_jump() {
    local table=$1 builtin=$2 owned=$3 label=$4
    check_rule "$table" -C "$builtin" -m comment --comment "smp:${SUFFIX}:jump:${label}" -j "$owned"
}

check_jump filter INPUT "SMP_I_${SUFFIX}" input
check_jump filter OUTPUT "SMP_O_${SUFFIX}" output
check_jump filter FORWARD "SMP_F_${SUFFIX}" forward
check_jump nat PREROUTING "SMP_PR_${SUFFIX}" prerouting
check_jump nat OUTPUT "SMP_NO_${SUFFIX}" output
check_jump nat POSTROUTING "SMP_PO_${SUFFIX}" postrouting

check_rule filter -C "SMP_O_${SUFFIX}" -o "$TAP" -d "$GUEST" -m comment --comment "smp:${SUFFIX}:host-output" -j ACCEPT
check_rule filter -C "SMP_I_${SUFFIX}" -i "$TAP" -s "$GUEST" -m conntrack --ctstate ESTABLISHED,RELATED -m comment --comment "smp:${SUFFIX}:host-input" -j ACCEPT
check_rule filter -C "SMP_F_${SUFFIX}" -i "$TAP" -s "$SUBNET" -m comment --comment "smp:${SUFFIX}:guest-forward" -j ACCEPT
check_rule filter -C "SMP_F_${SUFFIX}" -o "$TAP" -d "$SUBNET" -m conntrack --ctstate ESTABLISHED,RELATED -m comment --comment "smp:${SUFFIX}:guest-return" -j ACCEPT
check_rule nat -C "SMP_PO_${SUFFIX}" -s "$SUBNET" -o "$OUTBOUND" -m comment --comment "smp:${SUFFIX}:masquerade" -j MASQUERADE

while IFS=$'\t' read -r protocol host_port guest_port; do
    [[ -n $protocol ]] || continue
    TAG="smp:${SUFFIX}:${protocol}:${host_port}:${guest_port}"
    DESTINATION="${GUEST}:${guest_port}"
    check_rule filter -C "SMP_F_${SUFFIX}" -o "$TAP" -p "$protocol" -d "$GUEST" --dport "$guest_port" -m conntrack --ctstate NEW,ESTABLISHED,RELATED -m comment --comment "${TAG}:forward" -j ACCEPT
    check_rule nat -C "SMP_PR_${SUFFIX}" -p "$protocol" --dport "$host_port" -m comment --comment "${TAG}:prerouting" -j DNAT --to-destination "$DESTINATION"
    check_rule nat -C "SMP_NO_${SUFFIX}" -p "$protocol" -m addrtype --dst-type LOCAL --dport "$host_port" -m comment --comment "${TAG}:output" -j DNAT --to-destination "$DESTINATION"
    check_rule nat -C "SMP_PO_${SUFFIX}" -p "$protocol" -s 127.0.0.0/8 -d "$GUEST" --dport "$guest_port" -m comment --comment "${TAG}:hairpin" -j SNAT --to-source "$GATEWAY"
done < <(jq -r '.network.publishedPorts[]? | [.protocol, (.hostPort|tostring), (.guestPort|tostring)] | @tsv' <<<"$STATUS")

printf 'SMP core-owned host networking verified for %s via %s -> %s\n' "$MACHINE" "$TAP" "$OUTBOUND"
