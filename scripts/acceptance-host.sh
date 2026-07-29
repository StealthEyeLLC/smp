#!/usr/bin/env bash
set -euo pipefail

commit=
tree=
durable_log=
usage() {
  printf 'usage: sudo %s --commit SHA --tree SHA --log ABS\n' "$0" >&2
  exit 2
}
while (($#)); do
  case "$1" in
    --commit)
      (($# >= 2)) || usage
      commit="$2"
      shift 2
      ;;
    --tree)
      (($# >= 2)) || usage
      tree="$2"
      shift 2
      ;;
    --log)
      (($# >= 2)) || usage
      durable_log="$2"
      shift 2
      ;;
    *)
      usage
      ;;
  esac
done
[[ "$(id -u)" -eq 0 ]]
[[ "$commit" =~ ^[a-f0-9]{40}$ && "$tree" =~ ^[a-f0-9]{40}$ && "$durable_log" == /* ]]
[[ "$(uname -m)" == x86_64 && -c /dev/kvm && -c /dev/net/tun ]]

readonly pci_machine=p2-pci
readonly mmio_machine=p2-mmio
readonly disposable_machine=p2-disposable
readonly invalid_machine=p2-invalid
readonly upload_file=/var/lib/smp/provenance/prompt2-upload.bin
readonly download_file=/var/lib/smp/provenance/prompt2-download.bin
cleanup() {
  local rc=$?
  trap - EXIT
  for machine in "$pci_machine" "$mmio_machine" "$disposable_machine" "$invalid_machine"; do
    smp kill "$machine" >/dev/null 2>&1 || true
    smp destroy "$machine" --delete-disk >/dev/null 2>&1 || true
  done
  rm -f -- "$upload_file" "$download_file"
  exit "$rc"
}
trap cleanup EXIT

smp version --json | jq -e --arg commit "$commit" '.buildCommit == $commit'
smp assets --json | jq -e '
  .firecracker.version == "1.15.1" and
  .kernel.version == "6.1.178" and
  .debian.version == "13.6" and
  .debian.suite == "trixie"
'
smp doctor --json | jq -e '.report.healthy == true'
systemctl is-active --quiet smp.service
curl --fail --silent --unix-socket /run/smp/mcp.sock http://localhost/healthz | jq -e '.status == "ok"'
curl --fail --silent --unix-socket /run/smp/mcp.sock http://localhost/readyz | jq -e '.status == "ready"'

smp create "$pci_machine" --publish tcp:127.0.0.1:18080:8080
smp start "$pci_machine" --timeout 300
smp inspect "$pci_machine" | jq -e '.transport == "pci" and .state == "ready"'
[[ "$(smp exec --machine "$pci_machine" -- id -u)" == 0 ]]
# shellcheck disable=SC2016
smp exec --machine "$pci_machine" -- bash -lc \
  'set -euo pipefail
   grep -qx "13.6" /etc/debian_version
   test -s /etc/machine-id
   test -s /etc/ssh/ssh_host_ed25519_key
   ip route show default | grep -q default
   getent ahostsv4 deb.debian.org >/dev/null
   curl --fail --silent https://deb.debian.org/ >/dev/null
   apt-get update >/dev/null
   apt-get install -y --no-install-recommends hello >/dev/null
   hello >/dev/null
   truncate -s 32M /root/loop.img
   loop=$(losetup --find --show /root/loop.img)
   mkfs.ext4 -F "$loop" >/dev/null
   mkdir -p /mnt/ext4 /mnt/tmpfs /mnt/lower /mnt/upper /mnt/work /mnt/overlay
   mount "$loop" /mnt/ext4
   mount -t tmpfs tmpfs /mnt/tmpfs
   mount -t overlay overlay -o lowerdir=/mnt/lower,upperdir=/mnt/upper,workdir=/mnt/work /mnt/overlay
   unshare --mount true
   unshare --pid --fork true
   test "$(stat -fc %T /sys/fs/cgroup)" = cgroup2fs
   nft add table inet smp_acceptance
   ip tuntap add dev tunp2 mode tun
   ip tuntap add dev tapp2 mode tap
   ip link add vethp2a type veth peer name vethp2b
   ip link add brp2 type bridge
   ip link add dummyp2 type dummy
   printf "[Unit]\nDescription=p2\n[Service]\nType=oneshot\nExecStart=/bin/true\n[Install]\nWantedBy=multi-user.target\n" >/etc/systemd/system/smp-p2.service
   systemctl daemon-reload
   systemctl start smp-p2.service
   useradd -r smpp2
   groupadd smpp2group
   printf "int main(void){return 0;}\n" >/root/p2.c
   cc /root/p2.c -o /root/p2
   /root/p2
   systemctl disable --now smp-p2.service >/dev/null 2>&1 || true
   rm -f /etc/systemd/system/smp-p2.service
   nft delete table inet smp_acceptance
   ip link delete tunp2
   ip link delete tapp2
   ip link delete vethp2a
   ip link delete brp2
   ip link delete dummyp2
   umount /mnt/overlay /mnt/tmpfs /mnt/ext4
   losetup -d "$loop"'

# shellcheck disable=SC2016
literal='literal;$(touch /root/should-not-exist)*'
[[ "$(smp exec --machine "$pci_machine" -- printf %s "$literal")" == "$literal" ]]
smp exec --machine "$pci_machine" -- test ! -e /root/should-not-exist
set +e
smp exec --machine "$pci_machine" -- bash -c 'exit 37'
remote_rc=$?
set -e
[[ "$remote_rc" -eq 37 ]]

head -c 65536 /dev/urandom >"$upload_file"
smp cp "$upload_file" guest:/root/roundtrip.bin --machine "$pci_machine"
smp cp guest:/root/roundtrip.bin "$download_file" --machine "$pci_machine"
cmp --silent "$upload_file" "$download_file"

smp exec --machine "$pci_machine" -- bash -c 'printf persistent >/root/persistent-marker'
old_pid="$(smp inspect "$pci_machine" | jq -er .firecrackerProcess.pid)"
smp stop "$pci_machine"
smp start "$pci_machine" --timeout 300
[[ "$(smp exec --machine "$pci_machine" -- cat /root/persistent-marker)" == persistent ]]
smp reboot "$pci_machine" --timeout 300
new_pid="$(smp inspect "$pci_machine" | jq -er .firecrackerProcess.pid)"
[[ "$old_pid" != "$new_pid" ]]
smp api "$pci_machine" --method GET /machine-config --json | jq -e '.statusCode == 200'

smp create "$mmio_machine" --mmio
smp start "$mmio_machine" --timeout 300
smp inspect "$mmio_machine" | jq -e '.transport == "mmio" and .state == "ready"'
[[ "$(smp exec --machine "$mmio_machine" -- id -u)" == 0 ]]

smp create "$disposable_machine" --disposable
smp start "$disposable_machine" --timeout 300
disposable_root="$(smp inspect "$disposable_machine" | jq -er .rootDisk.path)"
smp stop "$disposable_machine"
smp destroy "$disposable_machine"
[[ ! -e "$disposable_root" ]]

smp create "$invalid_machine" --firecracker /bin/false
set +e
smp start "$invalid_machine" --timeout 10
invalid_rc=$?
set -e
[[ "$invalid_rc" -ne 0 ]]
[[ "$(smp status "$invalid_machine" --json | jq -r .state)" != ready ]]

pci_inspect="$(smp inspect "$pci_machine")"
pci_tap="$(jq -er .network.tap <<<"$pci_inspect")"
pci_api_socket="$(jq -er .apiSocket <<<"$pci_inspect")"
mmio_tap="$(smp inspect "$mmio_machine" | jq -er .network.tap)"
smp stop "$pci_machine"
smp stop "$mmio_machine"
smp destroy "$pci_machine" --delete-disk
smp destroy "$mmio_machine" --delete-disk
[[ ! -e "/sys/class/net/$pci_tap" && ! -e "/sys/class/net/$mmio_tap" ]]
[[ ! -S "$pci_api_socket" ]]
if nft list table inet smp 2>/dev/null | grep -Fq "smp:$pci_machine"; then
  exit 1
fi

jq -n -S \
  --arg commit "$commit" \
  --arg tree "$tree" \
  --arg completedAt "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
  '{schemaVersion:1,status:"passed",commit:$commit,tree:$tree,completedAt:$completedAt,plugin:{displayName:"SMP",namespace:"smp",onlyTool:"go",callableIdentity:"smp.go"}}' \
  >/var/lib/smp/provenance/prompt2-acceptance.json
printf 'real-host acceptance passed: %s %s\n' "$commit" "$tree" | tee -a "$durable_log"
