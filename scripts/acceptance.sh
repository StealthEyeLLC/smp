#!/bin/bash
set -euo pipefail
umask 077

RESUME_PRIMARY=0
while (($#)); do
    case "$1" in
        --resume-primary) RESUME_PRIMARY=1; shift ;;
        *) printf 'unknown acceptance argument: %s\n' "$1" >&2; exit 64 ;;
    esac
done

[[ $(id -u) -eq 0 ]] || { printf 'acceptance.sh requires root\n' >&2; exit 77; }
command -v smp >/dev/null || { printf 'smp is not installed\n' >&2; exit 69; }

PRIMARY=smp-cert-persistent
SECONDARY=smp-cert-isolated
DISPOSABLE=smp-cert-disposable
FAILURE=smp-cert-no-fallback
HOST_PORT=18080
RESULT_ROOT=/var/lib/smp/results/acceptance
PUBLISH_UNIT=
mkdir -p "$RESULT_ROOT"
if [[ $RESUME_PRIMARY -eq 1 ]]; then
    exec > >(tee -a "$RESULT_ROOT/stdout.log") 2> >(tee -a "$RESULT_ROOT/stderr.log" >&2)
else
    exec > >(tee "$RESULT_ROOT/stdout.log") 2> >(tee "$RESULT_ROOT/stderr.log" >&2)
fi

stage() {
    printf '\n=== %s ===\n' "$1"
}

cleanup_machine() {
    local name=$1
    smp destroy "$name" --force >/dev/null 2>&1 || true
}

cleanup_publish() {
    if [[ -n $PUBLISH_UNIT ]]; then
        smp exec "$PRIMARY" -- systemctl stop "${PUBLISH_UNIT}.service" >/dev/null 2>&1 || true
        smp exec "$PRIMARY" -- rm -f "/etc/systemd/system/${PUBLISH_UNIT}.service" >/dev/null 2>&1 || true
        smp exec "$PRIMARY" -- systemctl daemon-reload >/dev/null 2>&1 || true
        PUBLISH_UNIT=
    fi
}

cleanup() {
    cleanup_publish
    cleanup_machine "$DISPOSABLE"
    cleanup_machine "$FAILURE"
    cleanup_machine "$SECONDARY"
}
trap cleanup EXIT

assert_network_absent() {
    local name=$1
    local status=$2
    local tap suffix legacy_table
    tap="$(jq -er '.network.tapName' <<<"$status")"
    suffix="$(printf '%s' "$name" | sha256sum | cut -c1-10)"
    legacy_table="smp_$(printf '%s' "$name" | sha256sum | cut -c1-12)"
    if ip link show dev "$tap" >/dev/null 2>&1; then
        printf 'machine network cleanup left TAP %s for %s\n' "$tap" "$name" >&2
        return 1
    fi
    if iptables-save -t filter | grep -F "$suffix" >/dev/null; then
        printf 'machine network cleanup left filter state for %s\n' "$name" >&2
        return 1
    fi
    if iptables-save -t nat | grep -F "$suffix" >/dev/null; then
        printf 'machine network cleanup left NAT state for %s\n' "$name" >&2
        return 1
    fi
    if nft list tables | grep -Fx "table ip $legacy_table" >/dev/null; then
        printf 'machine network cleanup left legacy nftables state for %s\n' "$name" >&2
        return 1
    fi
}

probe_host_http() {
    local label=$1
    local url=$2
    local body=
    local attempt
    for attempt in $(seq 1 20); do
        body="$(curl --fail --silent --show-error --max-time 2 "$url" 2>/dev/null || true)"
        if [[ $body == published-ok ]]; then
            printf '%s verified: published-ok\n' "$label"
            return 0
        fi
        sleep 0.25
    done
    printf '%s failed for %s; last body=%q\n' "$label" "$url" "$body" >&2
    return 1
}

publish_diagnostics() {
    local guest_ip=$1
    printf '\n--- Published-port diagnostics ---\n' >&2
    if [[ -n $PUBLISH_UNIT ]]; then
        smp exec "$PRIMARY" -- systemctl status "${PUBLISH_UNIT}.service" --no-pager -l >&2 || true
        smp exec "$PRIMARY" -- journalctl -u "${PUBLISH_UNIT}.service" --no-pager -n 50 >&2 || true
    fi
    smp exec "$PRIMARY" -- ss -ltnp >&2 || true
    ip route get "$guest_ip" >&2 || true
    iptables -S INPUT >&2 || true
    iptables -S OUTPUT >&2 || true
    iptables -S FORWARD >&2 || true
    iptables -t nat -S OUTPUT >&2 || true
    iptables -t nat -S POSTROUTING >&2 || true
    printf '%s\n' '--- End published-port diagnostics ---' >&2
}

stage 'Host and asset verification'
smp doctor --fix
if [[ $RESUME_PRIMARY -eq 0 ]]; then
    smp assets
fi
MANIFEST=/var/lib/smp/assets/manifest.json
BASE_PATH="$(jq -r .rootfs.path "$MANIFEST")"
BASE_BEFORE="$(sha256sum "$BASE_PATH" | cut -d' ' -f1)"

cleanup
if [[ $RESUME_PRIMARY -eq 1 ]]; then
    stage 'Resuming existing ready persistent VM'
    PRIMARY_STATUS="$(smp status "$PRIMARY" --json)"
    jq -e '.state == "ready" and (.process.pid | type == "number")' <<<"$PRIMARY_STATUS" >/dev/null || {
        printf 'existing persistent VM is not ready for acceptance resume\n' >&2
        exit 65
    }
else
    cleanup_machine "$PRIMARY"
    stage 'Creating persistent PCI VM'
    smp create "$PRIMARY" --mode persistent --transport pci --vcpus 2 --memory-mib 2048 --publish "tcp:${HOST_PORT}:8080"
    smp start "$PRIMARY"
    smp wait "$PRIMARY" --timeout-seconds 180
fi

if [[ -x /usr/lib/smp/repair-host-network.sh ]]; then
    stage 'Host forwarding reconciliation'
    /usr/lib/smp/repair-host-network.sh "$PRIMARY"
fi

stage 'Guest identity, routing, DNS, and HTTPS'
[[ "$(smp exec "$PRIMARY" -- id -u)" == 0 ]]
smp exec "$PRIMARY" -- systemctl is-system-running --wait
smp exec "$PRIMARY" -- ip route get 1.1.1.1
RESOLV_CONF="$(smp exec "$PRIMARY" -- cat /etc/resolv.conf)"
printf '%s\n' "$RESOLV_CONF"
grep -Eq '^nameserver [0-9]+(\.[0-9]+){3}$' <<<"$RESOLV_CONF"
smp exec "$PRIMARY" -- getent ahostsv4 debian.org
smp exec "$PRIMARY" -- curl --fail --silent --show-error https://deb.debian.org/ >/dev/null

stage 'Package installation and guest capabilities'
smp exec "$PRIMARY" -- bash -lc 'apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends hello'
smp exec "$PRIMARY" -- hello
smp exec "$PRIMARY" -- bash -lc 'set -e; truncate -s 64M /root/fs.img; mkfs.ext4 -F /root/fs.img >/dev/null; L=$(losetup --find --show /root/fs.img); mkdir -p /mnt/fs; mount "$L" /mnt/fs; touch /mnt/fs/ok; umount /mnt/fs; losetup -d "$L"'
smp exec "$PRIMARY" -- bash -lc 'set -e; mkdir -p /mnt/tmpfs; mount -t tmpfs tmpfs /mnt/tmpfs; touch /mnt/tmpfs/ok; umount /mnt/tmpfs'
smp exec "$PRIMARY" -- bash -lc 'set -e; mkdir -p /root/ovl/{lower,upper,work,merged}; echo lower >/root/ovl/lower/value; mount -t overlay overlay -o lowerdir=/root/ovl/lower,upperdir=/root/ovl/upper,workdir=/root/ovl/work /root/ovl/merged; test "$(cat /root/ovl/merged/value)" = lower; umount /root/ovl/merged'
smp exec "$PRIMARY" -- unshare --mount --pid --fork --mount-proc true
smp exec "$PRIMARY" -- bash -lc 'test "$(stat -fc %T /sys/fs/cgroup)" = cgroup2fs'
smp exec "$PRIMARY" -- bash -lc 'nft add table inet smp_test; nft list table inet smp_test >/dev/null; nft delete table inet smp_test'
smp exec "$PRIMARY" -- bash -lc 'ip tuntap add dev smptun0 mode tap; ip link set smptun0 up; ip link delete smptun0'
smp exec "$PRIMARY" -- bash -lc 'ip link add smpveth0 type veth peer name smpveth1; ip link add smpbr0 type bridge; ip link set smpveth0 master smpbr0; ip link delete smpveth0; ip link delete smpbr0'
smp exec "$PRIMARY" -- bash -lc 'ip link add smpdummy0 type dummy; ip link set smpdummy0 up; ip link delete smpdummy0'
smp exec "$PRIMARY" -- bash -lc 'cat >/etc/systemd/system/smp-accept.service <<EOF_SERVICE
[Service]
Type=oneshot
ExecStart=/bin/true
RemainAfterExit=yes
EOF_SERVICE
systemctl daemon-reload; systemctl start smp-accept.service; systemctl is-active --quiet smp-accept.service; systemctl disable --now smp-accept.service >/dev/null 2>&1 || true; rm -f /etc/systemd/system/smp-accept.service; systemctl daemon-reload'
smp exec "$PRIMARY" -- bash -lc 'groupadd -f smpaccept; id -u smpaccept >/dev/null 2>&1 || useradd -g smpaccept smpaccept; id smpaccept'
smp exec "$PRIMARY" -- bash -lc 'cat >/root/native.c <<EOF_C
#include <stdio.h>
int main(void){puts("native-ok");return 0;}
EOF_C
gcc -O2 -o /root/native /root/native.c; test "$(/root/native)" = native-ok'

stage 'Published port and exact argv behavior'
PUBLISH_UNIT=smp-publish-acceptance
smp exec "$PRIMARY" -- systemctl stop "${PUBLISH_UNIT}.service" >/dev/null 2>&1 || true
smp exec "$PRIMARY" -- rm -f "/etc/systemd/system/${PUBLISH_UNIT}.service" >/dev/null 2>&1 || true
smp exec "$PRIMARY" -- systemctl daemon-reload
smp exec "$PRIMARY" -- bash -lc 'cat >/root/smp-http.c <<EOF_HTTP
#include <arpa/inet.h>
#include <errno.h>
#include <signal.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>
int main(void) {
    int listener = socket(AF_INET, SOCK_STREAM, 0);
    if (listener < 0) { perror("socket"); return 1; }
    int one = 1;
    if (setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one)) < 0) { perror("setsockopt"); return 1; }
    struct sockaddr_in address;
    memset(&address, 0, sizeof(address));
    address.sin_family = AF_INET;
    address.sin_port = htons(8080);
    address.sin_addr.s_addr = htonl(INADDR_ANY);
    if (bind(listener, (struct sockaddr *)&address, sizeof(address)) < 0) { perror("bind"); return 1; }
    if (listen(listener, 16) < 0) { perror("listen"); return 1; }
    signal(SIGPIPE, SIG_IGN);
    static const char response[] = "HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\npublished-ok";
    for (;;) {
        int client = accept(listener, NULL, NULL);
        if (client < 0) { if (errno == EINTR) continue; perror("accept"); return 1; }
        char request[1024];
        (void)read(client, request, sizeof(request));
        const char *cursor = response;
        size_t remaining = sizeof(response) - 1;
        while (remaining > 0) {
            ssize_t sent = send(client, cursor, remaining, MSG_NOSIGNAL);
            if (sent < 0) { if (errno == EINTR) continue; break; }
            cursor += sent;
            remaining -= (size_t)sent;
        }
        close(client);
    }
}
EOF_HTTP
gcc -O2 -Wall -Wextra -Werror -o /root/smp-http /root/smp-http.c
cat >/etc/systemd/system/smp-publish-acceptance.service <<EOF_UNIT
[Unit]
Description=SMP published-port acceptance listener
After=network.target

[Service]
Type=simple
ExecStart=/root/smp-http
Restart=always
RestartSec=1
EOF_UNIT
systemctl daemon-reload
systemctl start smp-publish-acceptance.service'
smp exec "$PRIMARY" -- systemctl is-active --quiet "${PUBLISH_UNIT}.service"

GUEST_HTTP=
for _ in $(seq 1 20); do
    GUEST_HTTP="$(smp exec "$PRIMARY" -- curl --fail --silent --show-error --max-time 2 http://127.0.0.1:8080/ 2>/dev/null || true)"
    [[ $GUEST_HTTP == published-ok ]] && break
    sleep 0.25
done
if [[ $GUEST_HTTP != published-ok ]]; then
    printf 'guest listener verification failed; last body=%q\n' "$GUEST_HTTP" >&2
    publish_diagnostics "$(smp status "$PRIMARY" --json | jq -r .network.guestAddress)"
    exit 1
fi
printf 'Guest listener verified: published-ok\n'

GUEST_IP="$(smp status "$PRIMARY" --json | jq -er .network.guestAddress)"
if ! probe_host_http 'Host direct guest-port path' "http://${GUEST_IP}:8080/"; then
    publish_diagnostics "$GUEST_IP"
    exit 1
fi
if ! probe_host_http 'Published localhost port' "http://127.0.0.1:${HOST_PORT}/"; then
    publish_diagnostics "$GUEST_IP"
    exit 1
fi

EXACT_ARG='$(touch /root/argv-was-shell); spaced ; *'
EXACT_OBSERVED="$(smp exec "$PRIMARY" -- /usr/bin/printf '%s' "$EXACT_ARG")"
[[ $EXACT_OBSERVED == "$EXACT_ARG" ]] || {
    printf 'exact argv mismatch: expected=%q observed=%q\n' "$EXACT_ARG" "$EXACT_OBSERVED" >&2
    exit 1
}
if smp exec "$PRIMARY" -- test -e /root/argv-was-shell; then
    printf 'exact argv was interpreted by a shell\n' >&2
    exit 1
fi
set +e
smp exec "$PRIMARY" -- sh -c 'exit 37'
NONZERO=$?
set -e
[[ $NONZERO -eq 37 ]] || {
    printf 'remote exit-code propagation mismatch: expected=37 observed=%s\n' "$NONZERO" >&2
    exit 1
}
printf 'Exact argv and remote exit-code propagation verified\n'
cleanup_publish

stage 'File transfer and persistence'
printf 'file-transfer-ok\n' > "$RESULT_ROOT/upload.txt"
smp cp "$PRIMARY" "$RESULT_ROOT/upload.txt" guest:/root/upload.txt
smp exec "$PRIMARY" -- grep -Fx file-transfer-ok /root/upload.txt
smp cp "$PRIMARY" guest:/root/upload.txt "$RESULT_ROOT/download.txt"
cmp "$RESULT_ROOT/upload.txt" "$RESULT_ROOT/download.txt"

smp exec "$PRIMARY" -- sh -c 'printf persistent-ok >/root/persistent-value'
smp stop "$PRIMARY"
smp start "$PRIMARY"
smp wait "$PRIMARY" --timeout-seconds 180
if [[ -x /usr/lib/smp/repair-host-network.sh ]]; then
    /usr/lib/smp/repair-host-network.sh "$PRIMARY"
fi
smp exec "$PRIMARY" -- grep -Fx persistent-ok /root/persistent-value
OLD_PROCESS="$(smp inspect "$PRIMARY" --json | jq -c .process)"
REBOOT_JSON="$(smp reboot "$PRIMARY" --json)"
NEW_PROCESS="$(jq -c .newProcess <<<"$REBOOT_JSON")"
[[ "$OLD_PROCESS" != "$NEW_PROCESS" ]]
if [[ -x /usr/lib/smp/repair-host-network.sh ]]; then
    /usr/lib/smp/repair-host-network.sh "$PRIMARY"
fi

stage 'MMIO isolation and disposable lifecycle'
smp create "$SECONDARY" --mode persistent --transport mmio --memory-mib 1024
smp start "$SECONDARY"
smp wait "$SECONDARY" --timeout-seconds 180
smp exec "$SECONDARY" -- sh -c 'printf isolated >/root/secondary-only'
if smp exec "$PRIMARY" -- test -e /root/secondary-only; then
    printf 'machine storage isolation failed\n' >&2
    exit 1
fi

smp create "$DISPOSABLE" --mode disposable --transport pci --memory-mib 1024
smp start "$DISPOSABLE"
smp wait "$DISPOSABLE" --timeout-seconds 180
smp exec "$DISPOSABLE" -- sh -c 'printf disposable >/root/value'
DISPOSABLE_STATUS="$(smp status "$DISPOSABLE" --json)"
smp destroy "$DISPOSABLE" --force
[[ ! -e "/var/lib/smp/machines/$DISPOSABLE" ]]
assert_network_absent "$DISPOSABLE" "$DISPOSABLE_STATUS"

stage 'Firecracker API and no-fallback failure behavior'
smp api "$PRIMARY" --method GET --path /machine-config --json | jq -e '.httpStatus == 200' >/dev/null
cp /bin/false "$RESULT_ROOT/not-firecracker"
chmod 0755 "$RESULT_ROOT/not-firecracker"
smp create "$FAILURE" --firecracker "$RESULT_ROOT/not-firecracker" --memory-mib 512
set +e
smp start "$FAILURE"
START_FAILURE=$?
set -e
[[ $START_FAILURE -ne 0 ]]
FAILURE_STATUS="$(smp status "$FAILURE" --json)"
jq -e '.state != "ready" and .process == null' <<<"$FAILURE_STATUS" >/dev/null
assert_network_absent "$FAILURE" "$FAILURE_STATUS"

stage 'Base image immutability and cleanup'
BASE_AFTER="$(sha256sum "$BASE_PATH" | cut -d' ' -f1)"
[[ "$BASE_BEFORE" == "$BASE_AFTER" ]]
smp stop "$SECONDARY"
SECONDARY_STATUS="$(smp status "$SECONDARY" --json)"
assert_network_absent "$SECONDARY" "$SECONDARY_STATUS"
smp destroy "$SECONDARY"
smp destroy "$FAILURE" --force
smp stop "$PRIMARY"
PRIMARY_STATUS="$(smp status "$PRIMARY" --json)"
jq -e '.state == "stopped" and .process == null' <<<"$PRIMARY_STATUS" >/dev/null
assert_network_absent "$PRIMARY" "$PRIMARY_STATUS"
for machine in "$DISPOSABLE" "$FAILURE" "$SECONDARY"; do
    [[ ! -e "/var/lib/smp/machines/$machine" ]]
done
rm -f "$RESULT_ROOT/not-firecracker"
trap - EXIT

jq -n \
  --arg result PASS \
  --arg baseSha256 "$BASE_AFTER" \
  --arg primary "$PRIMARY" \
  --arg completedAt "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
  '{result:$result,baseImageSha256:$baseSha256,persistentMachine:$primary,completedAt:$completedAt}' \
  > "$RESULT_ROOT/result.json"
printf 'SMP real Firecracker acceptance passed\n'
