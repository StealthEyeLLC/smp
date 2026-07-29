#!/bin/bash
set -euo pipefail
umask 077

SOURCE=
COMMIT=
SKIP_PACKAGES=0
SKIP_TUNNEL_PROMPT=0
CONTROL_PLANE_ONLY=0
RUSTUP_VERSION=1.29.0
RUST_TOOLCHAIN=1.97.1
RUST_HOST=x86_64-unknown-linux-gnu
RUSTUP_INIT_URL="https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${RUST_HOST}/rustup-init"
RUSTUP_INIT_SHA_URL="${RUSTUP_INIT_URL}.sha256"
CLOUDFLARED_VERSION=2026.5.2
CLOUDFLARED_SHA256=5286698547f03df745adb2355f04c12dde52ef425491e81f433642d695521886
CLOUDFLARED_URL="https://github.com/cloudflare/cloudflared/releases/download/${CLOUDFLARED_VERSION}/cloudflared-linux-amd64"
MIN_ASSET_BUILD_FREE_BYTES=$((10 * 1024 * 1024 * 1024))

while (($#)); do
    case "$1" in
        --source) SOURCE=$2; shift 2 ;;
        --commit) COMMIT=$2; shift 2 ;;
        --skip-packages) SKIP_PACKAGES=1; shift ;;
        --skip-tunnel-prompt) SKIP_TUNNEL_PROMPT=1; shift ;;
        --control-plane-only) CONTROL_PLANE_ONLY=1; shift ;;
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
      ca-certificates curl git build-essential pkg-config libssl-dev jq \
      debootstrap e2fsprogs util-linux iproute2 nftables iptables openssh-client \
      xz-utils zstd bison flex libelf-dev bc dwarves rsync file kmod procps shellcheck
fi

for tool in awk curl debugfs df git install iptables iptables-save jq losetup sha256sum shellcheck systemctl tar; do
    command -v "$tool" >/dev/null || { printf 'missing bootstrap tool: %s\n' "$tool" >&2; exit 69; }
done

BUILD_ROOT="$(mktemp -d /var/tmp/smp-bootstrap.XXXXXX)"
TMP_CLOUDFLARED=
cleanup() {
    [[ -z $TMP_CLOUDFLARED ]] || rm -f "$TMP_CLOUDFLARED"
    rm -rf "$BUILD_ROOT"
}
trap cleanup EXIT

install -d -m 0700 /var/lib/smp /var/lib/smp/toolchain /var/lib/smp/toolchain/rustup /var/lib/smp/toolchain/cargo
export RUSTUP_HOME=/var/lib/smp/toolchain/rustup
export CARGO_HOME=/var/lib/smp/toolchain/cargo
export RUSTUP_INIT_SKIP_PATH_CHECK=yes
RUSTUP_BIN="$CARGO_HOME/bin/rustup"
if [[ ! -x $RUSTUP_BIN ]]; then
    printf 'Installing official rustup %s into %s\n' "$RUSTUP_VERSION" "$CARGO_HOME"
    curl --fail --location --proto '=https' --tlsv1.2 --retry 4 --output "$BUILD_ROOT/rustup-init" "$RUSTUP_INIT_URL"
    curl --fail --location --proto '=https' --tlsv1.2 --retry 4 --output "$BUILD_ROOT/rustup-init.sha256" "$RUSTUP_INIT_SHA_URL"
    RUSTUP_INIT_SHA="$(awk 'NR == 1 {print $1}' "$BUILD_ROOT/rustup-init.sha256")"
    [[ $RUSTUP_INIT_SHA =~ ^[0-9a-f]{64}$ ]] || { printf 'invalid official rustup-init checksum\n' >&2; exit 65; }
    printf '%s  %s\n' "$RUSTUP_INIT_SHA" "$BUILD_ROOT/rustup-init" | sha256sum --check --strict -
    chmod 0755 "$BUILD_ROOT/rustup-init"
    "$BUILD_ROOT/rustup-init" -y --no-modify-path --default-toolchain none --profile minimal
fi
RUSTUP_OBSERVED="$($RUSTUP_BIN --version)"
[[ $RUSTUP_OBSERVED == "rustup ${RUSTUP_VERSION}"* ]] || { printf 'unexpected rustup: %s\n' "$RUSTUP_OBSERVED" >&2; exit 69; }
"$RUSTUP_BIN" toolchain install "$RUST_TOOLCHAIN" --profile minimal
"$RUSTUP_BIN" component add --toolchain "$RUST_TOOLCHAIN" rustfmt clippy
RUSTC_BIN="$($RUSTUP_BIN which --toolchain "$RUST_TOOLCHAIN" rustc)"
RUSTDOC_BIN="$($RUSTUP_BIN which --toolchain "$RUST_TOOLCHAIN" rustdoc)"
CARGO_BIN="$($RUSTUP_BIN which --toolchain "$RUST_TOOLCHAIN" cargo)"
for tool in "$RUSTC_BIN" "$RUSTDOC_BIN" "$CARGO_BIN"; do
    [[ -x $tool ]] || { printf 'pinned Rust tool is missing: %s\n' "$tool" >&2; exit 69; }
done
RUSTC_VERSION="$($RUSTC_BIN --version)"
CARGO_VERSION="$($CARGO_BIN --version)"
[[ $RUSTC_VERSION == "rustc ${RUST_TOOLCHAIN} "* ]] || { printf 'unexpected Rust compiler: %s\n' "$RUSTC_VERSION" >&2; exit 69; }
[[ $CARGO_VERSION == "cargo ${RUST_TOOLCHAIN} "* ]] || { printf 'unexpected Cargo: %s\n' "$CARGO_VERSION" >&2; exit 69; }
export RUSTC="$RUSTC_BIN"
export RUSTDOC="$RUSTDOC_BIN"
export PATH="$(dirname "$CARGO_BIN"):$PATH"
printf 'Using %s\n' "$RUSTUP_OBSERVED"
printf 'Using %s\n' "$RUSTC_VERSION"
printf 'Using %s\n' "$CARGO_VERSION"

mkdir -p "$BUILD_ROOT/source"
git -C "$SOURCE" archive --format=tar "$COMMIT" | tar -xf - -C "$BUILD_ROOT/source"
BUILD_SOURCE="$BUILD_ROOT/source"

printf 'Resolving the locked SMP dependency graph\n'
(
    cd "$BUILD_SOURCE"
    "$CARGO_BIN" generate-lockfile
    "$CARGO_BIN" fetch --locked
)
LOCK_SHA="$(sha256sum "$BUILD_SOURCE/Cargo.lock" | cut -d' ' -f1)"

printf 'Checking SMP repository commit %s\n' "$COMMIT"
SMP_CARGO_BIN="$CARGO_BIN" SMP_CARGO_LOCKED=1 bash "$BUILD_SOURCE/scripts/test-repository.sh"
printf 'Building SMP commit %s\n' "$COMMIT"
(
    cd "$BUILD_SOURCE"
    SMP_BUILD_COMMIT="$COMMIT" "$CARGO_BIN" build --release --locked
)
CANDIDATE="$BUILD_SOURCE/target/release/smp"
[[ -x $CANDIDATE ]] || { printf 'SMP release binary was not produced\n' >&2; exit 70; }
CANDIDATE_SHA="$(sha256sum "$CANDIDATE" | cut -d' ' -f1)"

install -d -m 0755 /usr/local/bin /usr/lib/smp /usr/lib/smp/assets /etc/smp /etc/smp/credentials
install -d -m 0700 /var/lib/smp /var/lib/smp/machines /var/lib/smp/assets /var/lib/smp/requests /var/lib/smp/results /var/lib/smp/provenance /run/smp
install -m 0755 "$CANDIDATE" /usr/local/bin/smp.new
mv -f /usr/local/bin/smp.new /usr/local/bin/smp
for script in build-assets.sh create-seed.sh repair-rootfs.sh repair-host-network.sh test-repository.sh acceptance.sh prompt2-handoff.sh recover-firecracker-acceptance.sh uninstall.sh; do
    install -m 0755 "$BUILD_SOURCE/scripts/$script" "/usr/lib/smp/$script"
done
rm -rf /usr/lib/smp/assets.new
install -d -m 0755 /usr/lib/smp/assets.new
cp -a "$BUILD_SOURCE/assets/." /usr/lib/smp/assets.new/
rm -rf /usr/lib/smp/assets
mv /usr/lib/smp/assets.new /usr/lib/smp/assets
install -m 0644 "$BUILD_SOURCE/packaging/systemd/smp.service" /etc/systemd/system/smp.service
install -m 0644 "$BUILD_SOURCE/packaging/systemd/smp-tunnel.service" /etc/systemd/system/smp-tunnel.service
install -m 0644 "$BUILD_SOURCE/plugin/SMP.json" /etc/smp/SMP.plugin.json
install -m 0644 "$BUILD_SOURCE/plugin/smp.go.schema.json" /etc/smp/smp.go.schema.json
install -m 0600 "$BUILD_SOURCE/Cargo.lock" /var/lib/smp/provenance/Cargo.lock.new
mv -f /var/lib/smp/provenance/Cargo.lock.new /var/lib/smp/provenance/Cargo.lock

if [[ ! -x /usr/local/bin/cloudflared ]] || ! printf '%s  %s\n' "$CLOUDFLARED_SHA256" /usr/local/bin/cloudflared | sha256sum --check --strict - >/dev/null 2>&1; then
    TMP_CLOUDFLARED="$(mktemp)"
    curl --fail --location --proto '=https' --tlsv1.2 --retry 4 --output "$TMP_CLOUDFLARED" "$CLOUDFLARED_URL"
    printf '%s  %s\n' "$CLOUDFLARED_SHA256" "$TMP_CLOUDFLARED" | sha256sum --check --strict -
    install -m 0755 "$TMP_CLOUDFLARED" /usr/local/bin/cloudflared.new
    mv -f /usr/local/bin/cloudflared.new /usr/local/bin/cloudflared
    rm -f "$TMP_CLOUDFLARED"
    TMP_CLOUDFLARED=
fi

INSTALLED_VERSION="$(/usr/local/bin/smp --json version | jq -r .version)"
cat > /etc/smp/install.json.new <<INSTALL
{
  "schemaVersion": 1,
  "repository": "StealthEyeLLC/smp",
  "branch": "build/smp-firecracker-god-mode-v1",
  "commit": "$COMMIT",
  "smpVersion": "$INSTALLED_VERSION",
  "binarySha256": "$CANDIDATE_SHA",
  "rustupVersion": "$RUSTUP_VERSION",
  "rustToolchain": "$RUST_TOOLCHAIN",
  "cargoLockSha256": "$LOCK_SHA",
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
systemctl enable smp.service >/dev/null
systemctl restart smp.service
curl --fail --silent --show-error http://127.0.0.1:7745/healthz >/dev/null
curl --fail --silent --show-error http://127.0.0.1:7745/readyz >/dev/null
SERVICE_REQUEST_ID="bootstrap-${COMMIT:0:20}"
SERVICE_RESPONSE="$(
    jq -nc --arg requestId "$SERVICE_REQUEST_ID" \
      '{jsonrpc:"2.0",id:1,method:"tools/call",params:{name:"go",arguments:{schemaVersion:1,requestId:$requestId,operation:"describe",argv:[],detach:false,options:{}}}}' |
    curl --fail --silent --show-error \
      -H 'Content-Type: application/json' \
      --data-binary @- \
      http://127.0.0.1:7745/mcp
)"
jq -e --arg commit "$COMMIT" '.result.structuredContent.data.buildCommit == $commit' <<<"$SERVICE_RESPONSE" >/dev/null || {
    printf 'live SMP service does not report expected build commit %s\n' "$COMMIT" >&2
    exit 65
}
if [[ -s /etc/smp/credentials/tunnel-token ]]; then
    systemctl enable smp-tunnel.service >/dev/null
    systemctl restart smp-tunnel.service
else
    systemctl disable smp-tunnel.service >/dev/null 2>&1 || true
fi

/usr/local/bin/smp --json describe >/dev/null

if [[ $CONTROL_PLANE_ONLY -eq 0 ]]; then
    [[ -c /dev/kvm ]] || { printf 'SMP real Firecracker certification requires /dev/kvm\n' >&2; exit 69; }
    if [[ ! -r /var/lib/smp/assets/manifest.json ]]; then
        AVAILABLE_BYTES="$(df -PB1 /var/lib/smp | awk 'NR == 2 {print $4}')"
        [[ $AVAILABLE_BYTES =~ ^[0-9]+$ ]] || { printf 'could not determine free disk space\n' >&2; exit 69; }
        printf 'Asset-build free space: %s bytes\n' "$AVAILABLE_BYTES"
        if (( AVAILABLE_BYTES < MIN_ASSET_BUILD_FREE_BYTES )); then
            printf 'SMP asset build requires at least %s free bytes; found %s\n' "$MIN_ASSET_BUILD_FREE_BYTES" "$AVAILABLE_BYTES" >&2
            exit 70
        fi
    fi

    for machine in smp-cert-disposable smp-cert-no-fallback smp-cert-isolated smp-cert-persistent; do
        /usr/local/bin/smp destroy "$machine" --force >/dev/null 2>&1 || true
    done
    if [[ -r /var/lib/smp/assets/manifest.json ]]; then
        printf 'Reconciling canonical guest initializer in existing rootfs\n'
        /usr/lib/smp/repair-rootfs.sh \
          --assets-root /var/lib/smp/assets \
          --source-root /usr/lib/smp/assets \
          --build-commit "$COMMIT"
    fi

    printf 'Preparing and verifying canonical Firecracker, Linux, and Debian assets\n'
    /usr/local/bin/smp --json assets | tee /var/lib/smp/provenance/assets.json >/dev/null
    jq -e '.schemaVersion == 1 and .architecture == "x86_64" and .firecracker.version == "1.15.1" and .kernel.version == "6.1.177" and .debianVersion == "13.6"' /var/lib/smp/assets/manifest.json >/dev/null
    for path in \
      "$(jq -r .firecracker.path /var/lib/smp/assets/manifest.json)" \
      "$(jq -r .kernel.path /var/lib/smp/assets/manifest.json)" \
      "$(jq -r .rootfs.path /var/lib/smp/assets/manifest.json)"; do
        [[ -f $path ]] || { printf 'certified asset missing: %s\n' "$path" >&2; exit 65; }
    done

    rm -f /var/lib/smp/results/acceptance/result.json
    printf 'Running complete real Firecracker acceptance\n'
    /usr/lib/smp/acceptance.sh
    jq -e '.result == "PASS"' /var/lib/smp/results/acceptance/result.json >/dev/null
    systemctl is-active --quiet smp.service
    curl --fail --silent --show-error http://127.0.0.1:7745/healthz >/dev/null
    curl --fail --silent --show-error http://127.0.0.1:7745/readyz >/dev/null
    /usr/lib/smp/prompt2-handoff.sh | tee /var/lib/smp/provenance/prompt2-handoff.json
fi

printf 'SMP full bootstrap complete\n'
printf 'commit=%s\n' "$COMMIT"
printf 'rustup_version=%s\n' "$RUSTUP_VERSION"
printf 'rust_toolchain=%s\n' "$RUST_TOOLCHAIN"
printf 'cargo_lock_sha256=%s\n' "$LOCK_SHA"
printf 'binary_sha256=%s\n' "$CANDIDATE_SHA"
printf 'smp_service=%s\n' "$(systemctl is-active smp.service)"
printf 'tunnel_service=%s\n' "$(systemctl is-active smp-tunnel.service 2>/dev/null || true)"
if [[ $CONTROL_PLANE_ONLY -eq 0 ]]; then
    printf 'acceptance_result=%s\n' "$(jq -r .result /var/lib/smp/results/acceptance/result.json)"
    printf 'acceptance_evidence=/var/lib/smp/results/acceptance/result.json\n'
    printf 'prompt2_handoff=/var/lib/smp/provenance/prompt2-handoff.json\n'
else
    printf 'prompt2_handoff=/usr/lib/smp/prompt2-handoff.sh\n'
fi
