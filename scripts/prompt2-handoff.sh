#!/bin/bash
set -euo pipefail
umask 077

INSTALL=/etc/smp/install.json
ASSETS=/var/lib/smp/assets/manifest.json
[[ -r $INSTALL ]] || { printf 'SMP install metadata is missing: %s\n' "$INSTALL" >&2; exit 66; }
[[ -x /usr/local/bin/smp ]] || { printf 'SMP binary is not installed\n' >&2; exit 66; }

COMMIT="$(jq -r .commit "$INSTALL")"
VERSION="$(jq -r .smpVersion "$INSTALL")"
BINARY_SHA="$(sha256sum /usr/local/bin/smp | cut -d' ' -f1)"
RECORDED_SHA="$(jq -r .binarySha256 "$INSTALL")"
[[ $BINARY_SHA == "$RECORDED_SHA" ]] || { printf 'installed SMP binary digest mismatch\n' >&2; exit 65; }

if [[ -r $ASSETS ]]; then
    FIRECRACKER="$(jq -c .firecracker "$ASSETS")"
    KERNEL="$(jq -c .kernel "$ASSETS")"
    ROOTFS="$(jq -c .rootfs "$ASSETS")"
else
    FIRECRACKER=null
    KERNEL=null
    ROOTFS=null
fi

SMP_STATUS="$(systemctl is-active smp.service 2>/dev/null || true)"
TUNNEL_STATUS="$(systemctl is-active smp-tunnel.service 2>/dev/null || true)"
TUNNEL_IDENTITY="$(systemctl show smp-tunnel.service -p FragmentPath -p MainPID -p ActiveState --value 2>/dev/null | paste -sd, - || true)"
MACHINES="$(/usr/local/bin/smp --json describe --machines 2>/dev/null | jq -c .machines || printf '[]')"
OPERATIONS="$(/usr/local/bin/smp --json describe 2>/dev/null | jq -c '[.operations[].name]' || printf '[]')"

jq -n \
  --arg result "PROMPT_2_BOOTSTRAPPED" \
  --arg repository "StealthEyeLLC/smp" \
  --arg branch "build/smp-firecracker-god-mode-v1" \
  --arg commit "$COMMIT" \
  --arg version "$VERSION" \
  --arg binarySha256 "$BINARY_SHA" \
  --arg localMcpEndpoint "http://127.0.0.1:7745/mcp" \
  --arg pluginDisplayName "SMP" \
  --arg callableIdentity "smp.go" \
  --arg smpService "$SMP_STATUS" \
  --arg tunnelService "$TUNNEL_STATUS" \
  --arg tunnelIdentity "$TUNNEL_IDENTITY" \
  --argjson firecracker "$FIRECRACKER" \
  --argjson kernel "$KERNEL" \
  --argjson rootfs "$ROOTFS" \
  --argjson operations "$OPERATIONS" \
  --argjson machines "$MACHINES" \
  '{result:$result,repository:$repository,branch:$branch,commit:$commit,
    installedSmpVersion:$version,installedBinarySha256:$binarySha256,
    requestSchemaVersion:1,responseSchemaVersion:1,
    localMcpEndpoint:$localMcpEndpoint,pluginDisplayName:$pluginDisplayName,
    onlyCallableTool:$callableIdentity,smpServiceStatus:$smpService,
    tunnelServiceStatus:$tunnelService,tunnelIdentityWithoutSecret:$tunnelIdentity,
    firecracker:$firecracker,kernel:$kernel,rootfs:$rootfs,
    operations:$operations,machines:$machines,
    limits:{inlineOutputBytes:1048576,capturedOutputBytes:67108864,
      maximumTimeoutSeconds:86400,requestRetentionSeconds:604800,resultRetentionSeconds:604800},
    nextAction:"Use a fresh ChatGPT tab with Prompt 2 after the private SMP custom MCP connection is enabled."}'
