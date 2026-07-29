#!/bin/bash
set -euo pipefail
umask 077

SOURCE=
COMMIT=
SKIP_PACKAGES=0
SKIP_TUNNEL_PROMPT=0
CLOUDFLARED_VERSION=2026.5.2
CLOUDFLARED_SHA256=5286698547f03df745adb2355f04c12dde52ef425491e81f433642d695521886
CLOUDFLARED_URL="https://github.com/cloudflare/cloudflared/releases/download/${CLOUDFLARED_VERSION}/cloudflared-linux-amd64"

while (($#)); do
    case "$1" in
        --source) SOURCE=$2; shift 2 ;;
        --commit) COMMIT=$2; shift 2 ;;
        --skip-packages) SKIP_PACKAGES=1; shift ;;
        --skip-tunnel-prompt) SKIP_TUNNEL_PROMPT=1; shift ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 64 ;;
    esac
done

[[ $(id -u) -eq 0 ]] || { printf 'bootstrap.sh must run as root\n' >&2; exit 77; }
[[ $(uname -m) == x86_64 ]] || { printf 'SMP canonical bootstrap requires x86_64\n' >&2; exit 69; }
[[ -n $SOURCE && -d $SOURCE/.git ]] || { printf -- '--source must name a Git checkout\n' >&2; exit 66; }
[[ $COMMIT =~ ^[0-9a-f]{40}$ ]] || { printf -- '--commit must be a full Git SHA\n' >&2; exit 64; }
SOURCE="$(cd "$SOURCE" && pwd)"
OBSERVED_COMMIT="$(git -C "$SOURCE" rev-parse HEAD)"
[[ $OBSERVED_COMMIT == "$COMMIT" ]] || { printf 'source commit mismatch: expected %s, found %s\n' "$COMMIT" "$OBSERVED_COMMIT" >&2; exit 65; }
[[ -z "$(git -C "$SOURCE" status --porcelain)" ]] || { printf 'source checkout has uncommitted work\n' >&2; exit 65; }
[[ "$(git -C "$SOURCE" remote get-url origin)" == *StealthEyeLLC/smp* ]] || { printf 'source repository is not StealthEyeLLC/smp\n' >&2; exit 65; }

if [[ $SKIP_PACKAGES -eq 0 ]]; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y --no-install-recommends \
      ca-certificates curl git build-essential cargo rustc pkg-config libssl-dev jq \
      debootstrap e2fsprogs util-linux iproute2 nftables openssh-client \
      xz-utils zstd bison flex libelf-dev bc dwarves rsync file kmod procps shellcheck
fi

for tool in cargo rustc curl git install systemctl sha256sum jq; do
    command -v "$tool" >/dev/null || { printf 'missing bootstrap tool: %s\n' "$tool" >&2; exit 69; }
done

printf 'Checking SMP repository commit %s\n' "$COMMIT"
"$SOURCE/scripts/test-repository.sh"
printf 'Building SMP commit %s\n' "$COMMIT"
(
    cd "$SOURCE"
    SMP_BUILD_COMMIT="$COMMIT" cargo build --release
)
CANDIDATE="$SOURCE/target/release/smp"
[[ -x $CANDIDATE ]] || { printf 'SMP release binary was not produced\n' >&2; exit 70; }
CANDIDATE_SHA="$(sha256sum "$CANDIDATE" | cut -d' ' -f1)"

install -d -m 0755 /usr/local/bin /usr/lib/smp /usr/lib/smp/assets /etc/smp /etc/smp/credentials
install -d -m 0700 /var/lib/smp /var/lib/smp/machines /var/lib/smp/assets /var/lib/smp/requests /var/lib/smp/results /run/smp
install -m 0755 "$CANDIDATE" /usr/local/bin/smp.new
mv -f /usr/local/bin/smp.new /usr/local/bin/smp
for script in build-assets.sh create-seed.sh test-repository.sh acceptance.sh prompt2-handoff.sh uninstall.sh; do
    install -m 0755 "$SOURCE/scripts/$script" "/usr/lib/smp/$script"
done
rm -rf /usr/lib/smp/assets.new
install -d -m 0755 /usr/lib/smp/assets.new
cp -a "$SOURCE/assets/." /usr/lib/smp/assets.new/
rm -rf /usr/lib/smp/assets
mv /usr/lib/smp/assets.new /usr/lib/smp/assets
install -m 0644 "$SOURCE/packaging/systemd/smp.service" /etc/systemd/system/smp.service
install -m 0644 "$SOURCE/packaging/systemd/smp-tunnel.service" /etc/systemd/system/smp-tunnel.service
install -m 0644 "$SOURCE/plugin/SMP.json" /etc/smp/SMP.plugin.json
install -m 0644 "$SOURCE/plugin/smp.go.schema.json" /etc/smp/smp.go.schema.json

TMP_CLOUDFLARED="$(mktemp)"
trap 'rm -f "$TMP_CLOUDFLARED"' EXIT
curl --fail --location --proto '=https' --tlsv1.2 --retry 4 --output "$TMP_CLOUDFLARED" "$CLOUDFLARED_URL"
printf '%s  %s\n' "$CLOUDFLARED_SHA256" "$TMP_CLOUDFLARED" | sha256sum --check --strict -
install -m 0755 "$TMP_CLOUDFLARED" /usr/local/bin/cloudflared.new
mv -f /usr/local/bin/cloudflared.new /usr/local/bin/cloudflared
rm -f "$TMP_CLOUDFLARED"
trap - EXIT

INSTALLED_VERSION="$(/usr/local/bin/smp --json version | jq -r .version)"
cat > /etc/smp/install.json.new <<INSTALL
{
  "schemaVersion": 1,
  "repository": "StealthEyeLLC/smp",
  "branch": "build/smp-firecracker-god-mode-v1",
  "commit": "$COMMIT",
  "smpVersion": "$INSTALLED_VERSION",
  "binarySha256": "$CANDIDATE_SHA",
  "cloudflaredVersion": "$CLOUDFLARED_VERSION",
  "cloudflaredSha256": "$CLOUDFLARED_SHA256",
  "installedAt": "$(date --utc +%Y-%m-%dT%H:%M:%SZ)"
}
INSTALL
chmod 0600 /etc/smp/install.json.new
mv /etc/smp/install.json.new /etc/smp/install.json

if [[ ! -s /etc/smp/credentials/tunnel-token && $SKIP_TUNNEL_PROMPT -eq 0 && -t 0 ]]; then
    printf 'Paste the dedicated SMP Cloudflare tunnel token, or press Enter to leave the tunnel disabled: ' >&2
    IFS= read -r -s TOKEN
    printf '\n' >&2
    if [[ -n $TOKEN ]]; then
        printf '%s\n' "$TOKEN" > /etc/smp/credentials/tunnel-token
        chmod 0600 /etc/smp/credentials/tunnel-token
        unset TOKEN
    fi
fi

systemctl daemon-reload
systemctl enable --now smp.service
curl --fail --silent --show-error http://127.0.0.1:7745/healthz >/dev/null
curl --fail --silent --show-error http://127.0.0.1:7745/readyz >/dev/null
if [[ -s /etc/smp/credentials/tunnel-token ]]; then
    systemctl enable --now smp-tunnel.service
else
    systemctl disable smp-tunnel.service >/dev/null 2>&1 || true
fi

/usr/local/bin/smp --json describe >/dev/null
printf 'SMP bootstrap complete\n'
printf 'commit=%s\n' "$COMMIT"
printf 'binary_sha256=%s\n' "$CANDIDATE_SHA"
printf 'smp_service=%s\n' "$(systemctl is-active smp.service)"
printf 'tunnel_service=%s\n' "$(systemctl is-active smp-tunnel.service 2>/dev/null || true)"
printf 'prompt2_handoff=/usr/lib/smp/prompt2-handoff.sh\n'
