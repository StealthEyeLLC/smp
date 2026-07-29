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
cleanup() {
    cleanup_machine "$DISPOSABLE"
    cleanup_machine "$FAILURE"
    cleanup_machine "$SECONDARY"
}
trap cleanup EXIT

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
smp exec "$PRIMARY" -- bash -lc 'modprobe dummy; lsmod | grep -q "^dummy "; modprobe -r dummy'
smp exec "$PRIMARY" -- bash -lc 'cat >/etc/systemd/system/smp-accept.service <<EOF
[Service]
Type=oneshot
ExecStart=/bin/true
RemainAfterExit=yes
EOF
systemctl daemon-reload; systemctl start smp-accept.service; systemctl is-active --quiet smp-accept.service; systemctl disable --now smp-accept.service >/dev/null 2>&1 || true; rm -f /etc/systemd/system/smp-accept.service; systemctl daemon-reload'
smp exec "$PRIMARY" -- bash -lc 'groupadd -f smpaccept; id -u smpaccept >/dev/null 2>&1 || useradd -g smpaccept smpaccept; id smpaccept'
smp exec "$PRIMARY" -- bash -lc 'cat >/root/native.c <<EOF
#include <stdio.h>
int main(void){puts("native-ok");return 0;}
EOF
gcc -O2 -o /root/native /root/native.c; test "$(/root/native)" = native-ok'

stage 'Published port and exact argv behavior'
smp exec "$PRIMARY" -- bash -lc 'pkill -x nc >/dev/null 2>&1 || true; nohup sh -c "while true; do printf \"HTTP/1.1 200 OK\\r\\nContent-Length: 12\\r\\nConnection: close\\r\\n\\r\\npublished-ok\" | nc -l -p 8080 -q 1; done" >/root/listener.log 2>&1 </dev/null &'
for _ in $(seq 1 30); do
    if [[ "$(curl --silent --max-time 1 "http://127.0.0.1:${HOST_PORT}" || true)" == published-ok ]]; then break; fi
    sleep 1
done
[[ "$(curl --silent --max-time 2 "http://127.0.0.1:${HOST_PORT}")" == published-ok ]]

EXACT_ARG='$(touch /root/argv-was-shell); spaced ; *'
smp exec "$PRIMARY" -- /usr/bin/printf '%s' "$EXACT_ARG" | grep -Fqx "$EXACT_ARG"
if smp exec "$PRIMARY" -- test -e /root/argv-was-shell; then
    printf 'exact argv was interpreted by a shell\n' >&2
    exit 1
fi
set +e
smp exec "$PRIMARY" -- sh -c 'exit 37'
NONZERO=$?
set -e
[[ $NONZERO -eq 37 ]]

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
smp exec "$PRIMARY" -- grep -Fx persistent-ok /root/persistent-value
OLD_PROCESS="$(smp inspect "$PRIMARY" --json | jq -c .process)"
REBOOT_JSON="$(smp reboot "$PRIMARY" --json)"
NEW_PROCESS="$(jq -c .newProcess <<<"$REBOOT_JSON")"
[[ "$OLD_PROCESS" != "$NEW_PROCESS" ]]

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
smp destroy "$DISPOSABLE" --force
[[ ! -e "/var/lib/smp/machines/$DISPOSABLE" ]]

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
[[ "$(smp status "$FAILURE" --json | jq -r .state)" != ready ]]

stage 'Base image immutability and cleanup'
BASE_AFTER="$(sha256sum "$BASE_PATH" | cut -d' ' -f1)"
[[ "$BASE_BEFORE" == "$BASE_AFTER" ]]
smp stop "$SECONDARY"
smp destroy "$SECONDARY"
smp stop "$PRIMARY"
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
