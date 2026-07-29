#!/usr/bin/env bash
set -euo pipefail

repository_url=https://github.com/StealthEyeLLC/smp.git
commit=
start_service=false

usage() {
  printf 'usage: sudo %s --commit 40_HEX_SHA [--start-service]\n' "$0" >&2
  exit 2
}

while (($#)); do
  case "$1" in
    --commit)
      (($# >= 2)) || usage
      commit="$2"
      shift 2
      ;;
    --start-service)
      start_service=true
      shift
      ;;
    *)
      usage
      ;;
  esac
done

[[ "$(id -u)" -eq 0 ]] || {
  printf 'SMP bootstrap requires root\n' >&2
  exit 1
}
[[ "$commit" =~ ^[a-f0-9]{40}$ ]] || usage
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]]
[[ -c /dev/kvm && -c /dev/net/tun ]]

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  build-essential ca-certificates curl debootstrap e2fsprogs file git iproute2 jq \
  libseccomp-dev make nftables openssh-client pkg-config rsync shellcheck systemd \
  tar xz-utils

workspace="$(mktemp -d /var/tmp/smp-bootstrap.XXXXXX)"
cleanup() {
  local rc=$?
  trap - EXIT
  [[ "$workspace" == /var/tmp/smp-bootstrap.* && -d "$workspace" && ! -L "$workspace" ]]
  find "$workspace" -xdev -mindepth 1 -delete
  rmdir -- "$workspace"
  exit "$rc"
}
trap cleanup EXIT

git clone --filter=blob:none --no-checkout "$repository_url" "$workspace/source"
git -C "$workspace/source" fetch --depth 1 origin "$commit"
git -C "$workspace/source" checkout --detach "$commit"
[[ "$(git -C "$workspace/source" rev-parse HEAD)" == "$commit" ]]
tree="$(git -C "$workspace/source" rev-parse 'HEAD^{tree}')"
[[ -z "$(git -C "$workspace/source" status --porcelain=v1 --untracked-files=all)" ]]

toolchain="$(awk -F'"' '/^channel/ {print $2}' "$workspace/source/rust-toolchain.toml")"
[[ -n "$toolchain" ]]
if ! command -v rustup >/dev/null; then
  rustup_init="$workspace/rustup-init"
  curl --fail --location --proto '=https' --tlsv1.2 \
    --output "$rustup_init" https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init
  chmod 0755 "$rustup_init"
  "$rustup_init" -y --profile minimal --default-toolchain "$toolchain"
fi
export PATH="${CARGO_HOME:-/root/.cargo}/bin:$PATH"
rustup toolchain install "$toolchain" --profile minimal --component rustfmt --component clippy

(
  cd "$workspace/source"
  bash scripts/test-repository.sh
  cargo build --locked --release
  bash scripts/build-assets.sh \
    --stage all \
    --workspace "$workspace/assets" \
    --release-binary "$workspace/source/target/release/smp"
)

install_args=(
  --source "$workspace/source"
  --binary "$workspace/source/target/release/smp"
  --assets "$workspace/assets/output"
  --expected-commit "$commit"
  --expected-tree "$tree"
)
if [[ "$start_service" == true ]]; then
  install_args+=(--start-service)
fi
bash "$workspace/source/scripts/install.sh" "${install_args[@]}"
