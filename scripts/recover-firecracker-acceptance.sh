#!/bin/bash
set -euo pipefail
umask 077

SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXPECTED_COMMIT=${1:-}
RESULT_ROOT=/var/lib/smp/results
ACCEPTANCE_ROOT=$RESULT_ROOT/acceptance
RECOVERY_ROOT=/var/lib/smp/recovery
FINAL_STATUS=$RESULT_ROOT/final-recovery.status.json
PRIMARY=smp-cert-persistent
CERT_MACHINES=(smp-cert-disposable smp-cert-no-fallback smp-cert-isolated smp-cert-persistent)
STARTED_AT="$(date --utc +%Y-%m-%dT%H:%M:%SZ)"
ARCHIVE="$RESULT_ROOT/archive/final-recovery-$(date --utc +%Y%m%dT%H%M%SZ)"

[[ $(id -u) -eq 0 ]] || { printf 'recover-firecracker-acceptance.sh requires root\n' >&2; exit 77; }
[[ $EXPECTED_COMMIT =~ ^[0-9a-f]{40}$ ]] || { printf 'expected full recovery commit SHA\n' >&2; exit 64; }
OBSERVED_COMMIT="$(git -C "$SOURCE_ROOT" rev-parse HEAD)"
OBSERVED_TREE="$(git -C "$SOURCE_ROOT" rev-parse 'HEAD^{tree}')"
[[ $OBSERVED_COMMIT == "$EXPECTED_COMMIT" ]] || {
    printf 'recovery source mismatch: expected %s, observed %s\n' "$EXPECTED_COMMIT" "$OBSERVED_COMMIT" >&2
    exit 65
}
[[ -z "$(git -C "$SOURCE_ROOT" status --porcelain)" ]] || { printf 'recovery source has uncommitted work\n' >&2; exit 65; }

install -d -m 0700 "$RESULT_ROOT" "$ACCEPTANCE_ROOT" "$RECOVERY_ROOT" "$ARCHIVE"
rm -f "$FINAL_STATUS"

capture_command() {
    local name=$1
    shift
    { "$@"; } >"$ARCHIVE/${name}.stdout" 2>"$ARCHIVE/${name}.stderr" || true
}

finalize() {
    local status=$?
    set +e
    local completed
    completed="$(date --utc +%Y-%m-%dT%H:%M:%SZ)"
    for machine in "${CERT_MACHINES[@]}"; do
        capture_command "final-${machine}-status" /usr/local/bin/smp status "$machine" --json
    done
    capture_command final-processes ps -eo pid,ppid,lstart,stat,comm
    capture_command final-links ip -details link show
    capture_command final-addresses ip -details address show
    capture_command final-routes ip route show table all
    capture_command final-iptables-filter iptables-save -t filter
    capture_command final-iptables-nat iptables-save -t nat
    capture_command final-nft nft list ruleset
    jq -n \
      --arg result "$([[ $status -eq 0 ]] && printf PASS || printf FAIL)" \
      --argjson exitStatus "$status" \
      --arg startedAt "$STARTED_AT" \
      --arg completedAt "$completed" \
      --arg commit "$EXPECTED_COMMIT" \
      --arg tree "$OBSERVED_TREE" \
      --arg log "/var/lib/smp/results/final-recovery.log" \
      --arg archive "$ARCHIVE" \
      '{result:$result,exitStatus:$exitStatus,startedAt:$startedAt,completedAt:$completedAt,commit:$commit,tree:$tree,log:$log,archive:$archive}' \
      > "$FINAL_STATUS.tmp"
    chmod 0600 "$FINAL_STATUS.tmp"
    mv -f "$FINAL_STATUS.tmp" "$FINAL_STATUS"
    printf 'final_exit_status=%s\n' "$status"
    printf 'final_status=%s\n' "$FINAL_STATUS"
    printf 'failure_archive=%s\n' "$ARCHIVE"
    exit "$status"
}
trap finalize EXIT

printf 'SMP targeted recovery source verified\n'
printf 'recovery_commit=%s\n' "$EXPECTED_COMMIT"
printf 'recovery_tree=%s\n' "$OBSERVED_TREE"

for path in \
  /etc/smp/install.json \
  /var/lib/smp/assets/manifest.json \
  /var/lib/smp/assets/provenance/kernel-capabilities-revision.json \
  "$ACCEPTANCE_ROOT/result.json" \
  "$ACCEPTANCE_ROOT/stdout.log" \
  "$ACCEPTANCE_ROOT/stderr.log" \
  /var/lib/smp/provenance/prompt2-handoff.json; do
    [[ ! -e $path ]] || cp -a "$path" "$ARCHIVE/$(basename "$path").before"
done
for machine in "${CERT_MACHINES[@]}"; do
    [[ ! -r /var/lib/smp/machines/$machine/machine.json ]] || cp -a "/var/lib/smp/machines/$machine/machine.json" "$ARCHIVE/${machine}.machine.before.json"
    capture_command "before-${machine}-status" /usr/local/bin/smp status "$machine" --json
done
capture_command before-processes ps -eo pid,ppid,lstart,stat,comm
capture_command before-links ip -details link show
capture_command before-addresses ip -details address show
capture_command before-routes ip route show table all
capture_command before-sockets ss -xlpn
capture_command before-iptables-filter iptables-save -t filter
capture_command before-iptables-nat iptables-save -t nat
capture_command before-nft nft list ruleset

printf 'Building, testing, and atomically installing only the corrected SMP control plane\n'
bash "$SOURCE_ROOT/scripts/bootstrap.sh" \
  --source "$SOURCE_ROOT" \
  --commit "$EXPECTED_COMMIT" \
  --skip-packages \
  --skip-tunnel-prompt \
  --control-plane-only

[[ -x /usr/local/bin/smp ]] || { printf 'installed SMP binary is unavailable\n' >&2; exit 69; }
[[ -r /etc/smp/install.json ]] || { printf 'installed SMP metadata is unavailable\n' >&2; exit 66; }
jq -e --arg commit "$EXPECTED_COMMIT" '.commit == $commit' /etc/smp/install.json >/dev/null
INSTALLED_SHA="$(sha256sum /usr/local/bin/smp | cut -d' ' -f1)"
RECORDED_SHA="$(jq -r .binarySha256 /etc/smp/install.json)"
[[ $INSTALLED_SHA == "$RECORDED_SHA" ]] || { printf 'installed SMP binary digest mismatch\n' >&2; exit 65; }

ASSETS_ROOT=/var/lib/smp/assets
MANIFEST=$ASSETS_ROOT/manifest.json
REVISION=$ASSETS_ROOT/provenance/kernel-capabilities-revision.json
FIRECRACKER_ARCHIVE=$ASSETS_ROOT/downloads/firecracker-v1.15.1-x86_64.tgz
KERNEL_MODULES=$ASSETS_ROOT/kernel/modules-6.1.177
KERNEL_CONFIG_PROVENANCE=$ASSETS_ROOT/provenance/kernel-config.sha256
MODULE_PROVENANCE=$ASSETS_ROOT/provenance/module-tree.sha256
[[ -r $MANIFEST && -r $REVISION && -f $FIRECRACKER_ARCHIVE &&
   -d $KERNEL_MODULES && -r $KERNEL_CONFIG_PROVENANCE && -r $MODULE_PROVENANCE ]] || {
    printf 'corrected canonical asset metadata or retained assets are unavailable\n' >&2
    exit 66
}
hash_tree() {
    local directory=$1
    find "$directory" -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1
}
jq -e '
  .schemaVersion == 1 and
  .architecture == "x86_64" and
  .firecracker.version == "1.15.1" and
  .firecracker.sha256 == "7e8b57e88c459396d4680d83dcdd8c7f72305447cb55b11f4ac98ad70a3f7825" and
  .firecrackerReleaseAsset == "firecracker-v1.15.1-x86_64.tgz" and
  .firecrackerReleaseSha256 == "d4a32ab2322d887ca1bc4a4e7afa9cc35393e6362dfc2b3becb389d362e4275a" and
  .kernel.version == "6.1.177" and
  .kernel.sha256 == "d1134da8fddbebbb0212f193398e64e60533a9d5d2363cc62e361c2e4bae95fb" and
  .kernelConfigSha256 == "f40b6317e62eb55f9b019e1d0359350c505688b5cf362cc32d8b541cc9844df4" and
  .moduleTreeSha256 == "ac0f97629f17612332ebe6b469a46195dda39bf7ff0725192908886acbe59eb4" and
  .rootfs.sha256 == "501d447bcaf180a50a834f438e88d2733aa309d656d19bb9f2a0433536f99da5" and
  .debianVersion == "13.6" and .debianSuite == "trixie"' "$MANIFEST" >/dev/null
jq -e '
  .schemaVersion == 1 and
  .kernelSha256 == "d1134da8fddbebbb0212f193398e64e60533a9d5d2363cc62e361c2e4bae95fb" and
  .moduleTreeSha256 == "ac0f97629f17612332ebe6b469a46195dda39bf7ff0725192908886acbe59eb4" and
  .rootfsSha256 == "501d447bcaf180a50a834f438e88d2733aa309d656d19bb9f2a0433536f99da5" and
  .kernelConfigSha256 == "f40b6317e62eb55f9b019e1d0359350c505688b5cf362cc32d8b541cc9844df4" and
  (.enabledCapabilities | index("CONFIG_NF_TABLES") != null) and
  (.enabledCapabilities | index("CONFIG_NF_TABLES_IPV4") != null) and
  (.enabledCapabilities | index("CONFIG_NF_TABLES_IPV6") != null) and
  (.enabledCapabilities | index("CONFIG_NF_TABLES_INET") != null) and
  (.enabledCapabilities | index("CONFIG_DUMMY") != null)' "$REVISION" >/dev/null
FIRECRACKER_ARCHIVE_SHA="$(sha256sum "$FIRECRACKER_ARCHIVE" | cut -d' ' -f1)"
[[ $FIRECRACKER_ARCHIVE_SHA == d4a32ab2322d887ca1bc4a4e7afa9cc35393e6362dfc2b3becb389d362e4275a ]] || {
    printf 'Firecracker archive digest mismatch: %s\n' "$FIRECRACKER_ARCHIVE_SHA" >&2
    exit 65
}
MODULE_TREE_SHA="$(hash_tree "$KERNEL_MODULES")"
[[ $MODULE_TREE_SHA == ac0f97629f17612332ebe6b469a46195dda39bf7ff0725192908886acbe59eb4 ]] || {
    printf 'kernel module-tree digest mismatch: %s\n' "$MODULE_TREE_SHA" >&2
    exit 65
}
[[ "$(awk 'NR == 1 {print $1}' "$KERNEL_CONFIG_PROVENANCE")" == f40b6317e62eb55f9b019e1d0359350c505688b5cf362cc32d8b541cc9844df4 ]] || {
    printf 'kernel configuration provenance digest mismatch\n' >&2
    exit 65
}
[[ "$(awk 'NR == 1 {print $1}' "$MODULE_PROVENANCE")" == "$MODULE_TREE_SHA" ]] || {
    printf 'kernel module-tree provenance digest mismatch\n' >&2
    exit 65
}
for spec in \
  'firecracker 7e8b57e88c459396d4680d83dcdd8c7f72305447cb55b11f4ac98ad70a3f7825' \
  'kernel d1134da8fddbebbb0212f193398e64e60533a9d5d2363cc62e361c2e4bae95fb' \
  'rootfs 501d447bcaf180a50a834f438e88d2733aa309d656d19bb9f2a0433536f99da5'; do
    set -- $spec
    path="$(jq -r ".${1}.path" "$MANIFEST")"
    observed="$(sha256sum "$path" | cut -d' ' -f1)"
    [[ $observed == "$2" ]] || { printf '%s asset digest mismatch: %s\n' "$1" "$observed" >&2; exit 65; }
done
printf 'Corrected canonical assets verified without rebuild\n'

delete_iptables_rule_all() {
    local table=$1 chain=$2
    shift 2
    local prefix=(iptables -w 5)
    [[ $table == filter ]] || prefix+=(-t "$table")
    while "${prefix[@]}" -C "$chain" "$@" >/dev/null 2>&1; do
        "${prefix[@]}" -D "$chain" "$@"
    done
}

cleanup_stale_machine() {
    local machine=$1 status=$2
    local directory="/var/lib/smp/machines/$machine"
    local record="$directory/machine.json"
    [[ -r $record ]] || { printf 'stale machine record is unreadable: %s\n' "$record" >&2; exit 75; }

    local tap guest gateway prefix api_socket config_path suffix legacy_table subnet outbound a b c destination
    tap="$(jq -er '.network.tapName' "$record")"
    guest="$(jq -er '.network.guestAddress' "$record")"
    gateway="$(jq -er '.network.gatewayAddress' "$record")"
    prefix="$(jq -er '.network.prefixLength' "$record")"
    api_socket="$(jq -er '.apiSocket' "$record")"
    config_path="$(jq -er '.configPath' "$record")"
    suffix="$(printf '%s' "$machine" | sha256sum | cut -c1-10)"
    legacy_table="smp_$(printf '%s' "$machine" | sha256sum | cut -c1-12)"
    IFS=. read -r a b c _ <<<"$gateway"
    subnet="$a.$b.$c.0/$prefix"
    outbound="$(ip -o route show default | awk 'NR == 1 {for (i=1; i<=NF; i++) if ($i == "dev") {print $(i+1); exit}}')"

    if ss -xlpn 2>/dev/null | grep -F -- "$api_socket" >/dev/null; then
        printf 'stale machine API socket is still owned by a process: %s\n' "$api_socket" >&2
        exit 75
    fi
    local cmdline pid text
    for cmdline in /proc/[0-9]*/cmdline; do
        [[ -r $cmdline ]] || continue
        pid=${cmdline#/proc/}
        pid=${pid%/cmdline}
        [[ $pid == $$ || $pid == $PPID ]] && continue
        text="$(tr '\0' ' ' < "$cmdline" 2>/dev/null || true)"
        if [[ $text == *"$api_socket"* || $text == *"$config_path"* ]]; then
            printf 'stale machine still has a process referencing its socket or config: pid=%s\n' "$pid" >&2
            exit 75
        fi
    done

    printf '%s\n' "$status" > "$ARCHIVE/${machine}.stale-state.json"

    local table builtin code owned label
    while read -r table builtin code label; do
        owned="SMP_${code}_${suffix}"
        delete_iptables_rule_all "$table" "$builtin" -m comment --comment "smp:${suffix}:jump:${label}" -j "$owned"
        if { [[ $table == filter ]] && iptables -w 5 -L "$owned" -n >/dev/null 2>&1; } || { [[ $table != filter ]] && iptables -w 5 -t "$table" -L "$owned" -n >/dev/null 2>&1; }; then
            if [[ $table == filter ]]; then
                iptables -w 5 -F "$owned"
                iptables -w 5 -X "$owned"
            else
                iptables -w 5 -t "$table" -F "$owned"
                iptables -w 5 -t "$table" -X "$owned"
            fi
        fi
    done <<CHAINS
filter INPUT I input
filter OUTPUT O output
filter FORWARD F forward
nat PREROUTING PR prerouting
nat OUTPUT NO output
nat POSTROUTING PO postrouting
CHAINS

    nft delete table ip "$legacy_table" >/dev/null 2>&1 || true
    delete_iptables_rule_all filter OUTPUT -o "$tap" -j ACCEPT
    delete_iptables_rule_all filter INPUT -i "$tap" -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
    delete_iptables_rule_all filter FORWARD -i "$tap" -j ACCEPT
    delete_iptables_rule_all filter FORWARD -o "$tap" -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
    [[ -z $outbound ]] || delete_iptables_rule_all nat POSTROUTING -s "$subnet" -o "$outbound" -j MASQUERADE
    while IFS=$'\t' read -r protocol host_port guest_port; do
        [[ -n $protocol ]] || continue
        destination="${guest}:${guest_port}"
        delete_iptables_rule_all filter FORWARD -o "$tap" -p "$protocol" -d "$guest" --dport "$guest_port" -m conntrack --ctstate NEW,ESTABLISHED,RELATED -j ACCEPT
        delete_iptables_rule_all nat OUTPUT -p "$protocol" -m addrtype --dst-type LOCAL --dport "$host_port" -j DNAT --to-destination "$destination"
        delete_iptables_rule_all nat POSTROUTING -p "$protocol" -s 127.0.0.0/8 -d "$guest" --dport "$guest_port" -j SNAT --to-source "$gateway"
    done < <(jq -r '.network.publishedPorts[]? | [.protocol, (.hostPort|tostring), (.guestPort|tostring)] | @tsv' "$record")
    ip link show dev "$tap" >/dev/null 2>&1 && ip link delete dev "$tap" || true
    rm -f "$api_socket"
    mv "$directory" "$RECOVERY_ROOT/${machine}.stale.$(date --utc +%Y%m%dT%H%M%SZ)"
    printf 'Archived and cleaned unowned stale certification state for %s after proving no process owned it\n' "$machine"
}

reset_machine() {
    local machine=$1
    local directory="/var/lib/smp/machines/$machine"
    [[ -d $directory ]] || return 0

    local status state pid
    status="$(/usr/local/bin/smp status "$machine" --json 2>/dev/null || true)"
    printf '%s\n' "$status" > "$ARCHIVE/${machine}.status.before-reset.json"
    state="$(jq -r '.state // "unreadable"' <<<"$status" 2>/dev/null || printf unreadable)"
    pid="$(jq -r '.process.pid // empty' <<<"$status" 2>/dev/null || true)"
    printf 'Resetting certification machine %s from state=%s pid=%s\n' "$machine" "$state" "${pid:-none}"

    if [[ $pid =~ ^[0-9]+$ ]]; then
        if ! /usr/local/bin/smp stop "$machine"; then
            /usr/local/bin/smp kill "$machine" || true
        fi
    fi

    status="$(/usr/local/bin/smp status "$machine" --json 2>/dev/null || true)"
    state="$(jq -r '.state // "unreadable"' <<<"$status" 2>/dev/null || printf unreadable)"
    pid="$(jq -r '.process.pid // empty' <<<"$status" 2>/dev/null || true)"
    if [[ $pid =~ ^[0-9]+$ ]]; then
        printf 'certification machine %s still has a recorded process after stop/kill\n' "$machine" >&2
        exit 70
    fi
    if [[ $state == stale ]]; then
        cleanup_stale_machine "$machine" "$status"
        return 0
    fi
    if [[ $state == unreadable ]]; then
        printf 'certification machine %s is unreadable after verified process handling; refusing destruction\n' "$machine" >&2
        exit 75
    fi
    /usr/local/bin/smp destroy "$machine" --force
    [[ ! -e $directory ]] || { printf 'certification machine directory remained after destroy: %s\n' "$directory" >&2; exit 70; }
}

for machine in "${CERT_MACHINES[@]}"; do
    reset_machine "$machine"
done

rm -f "$ACCEPTANCE_ROOT/result.json"
printf 'Running complete real Firecracker acceptance with core-owned host networking\n'
/usr/lib/smp/acceptance.sh
jq -e '.result == "PASS"' "$ACCEPTANCE_ROOT/result.json" >/dev/null
systemctl is-active --quiet smp.service
curl --fail --silent --show-error http://127.0.0.1:7745/healthz >/dev/null
curl --fail --silent --show-error http://127.0.0.1:7745/readyz >/dev/null
/usr/lib/smp/prompt2-handoff.sh > /var/lib/smp/provenance/prompt2-handoff.json.tmp
jq -e '.result == "PROMPT_2_CERTIFIED" and .acceptance.result == "PASS"' /var/lib/smp/provenance/prompt2-handoff.json.tmp >/dev/null
chmod 0600 /var/lib/smp/provenance/prompt2-handoff.json.tmp
mv -f /var/lib/smp/provenance/prompt2-handoff.json.tmp /var/lib/smp/provenance/prompt2-handoff.json

COMPLETED_AT="$(jq -r .completedAt "$ACCEPTANCE_ROOT/result.json")"
printf 'SMP real Firecracker acceptance passed\n'
printf 'SMP targeted recovery complete\n'
printf 'acceptance_result=PASS\n'
printf 'acceptance_completed_at=%s\n' "$COMPLETED_AT"
printf 'repository_commit=%s\n' "$EXPECTED_COMMIT"
printf 'repository_tree=%s\n' "$OBSERVED_TREE"
printf 'installed_binary_sha256=%s\n' "$INSTALLED_SHA"
printf 'installed_binary_provenance_commit=%s\n' "$(jq -r .commit /etc/smp/install.json)"
printf 'firecracker_sha256=%s\n' "$(jq -r .firecracker.sha256 "$MANIFEST")"
printf 'kernel_sha256=%s\n' "$(jq -r .kernel.sha256 "$MANIFEST")"
printf 'kernel_config_sha256=%s\n' "$(jq -r .kernelConfigSha256 "$MANIFEST")"
printf 'module_tree_sha256=%s\n' "$(jq -r .moduleTreeSha256 "$MANIFEST")"
printf 'rootfs_sha256=%s\n' "$(jq -r .rootfs.sha256 "$MANIFEST")"
printf 'acceptance_evidence=%s\n' "$ACCEPTANCE_ROOT/result.json"
printf 'prompt2_handoff=/var/lib/smp/provenance/prompt2-handoff.json\n'
printf 'persistent_machine_state=%s\n' "$(/usr/local/bin/smp status "$PRIMARY" --json | jq -r .state)"
printf 'remaining_unresolved_risks=none_detected_by_prompt1_acceptance\n'
