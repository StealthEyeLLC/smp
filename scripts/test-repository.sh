#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "$0")/.." && pwd -P)"
cd "$repository_root"
test_workspace="${SMP_TEST_WORKSPACE:-$repository_root/target/smp-test-state}"
[[ "$test_workspace" == /* && "$test_workspace" != / && "$test_workspace" != /tmp ]]
install -d -m 0700 "$test_workspace"
export CARGO_TERM_COLOR=never

expected_toolchain="$(awk -F'"' '/^channel/ {print $2}' rust-toolchain.toml)"
[[ "$expected_toolchain" == 1.88.0 ]]
rustc --version | grep -Fq 'rustc 1.88.0 '
cargo --version

cargo metadata --locked --format-version 1 >"$test_workspace/cargo-metadata.json"
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features

mapfile -d '' shell_files < <(find scripts assets/guest -type f -name '*.sh' -print0 | sort -z)
((${#shell_files[@]} > 0))
for shell_file in "${shell_files[@]}"; do
  bash -n "$shell_file"
done
shellcheck -x "${shell_files[@]}"

mapfile -d '' json_files < <(find config plugin -type f -name '*.json' -print0 | sort -z)
((${#json_files[@]} > 0))
for json_file in "${json_files[@]}"; do
  jq empty "$json_file"
done
jq -e '
  .displayName == "SMP" and
  .namespace == "smp" and
  (.tools | length) == 1 and
  .tools[0].name == "go" and
  .tools[0].callableIdentity == "smp.go"
' plugin/plugin.json >/dev/null
jq -e '
  .properties.operation.type == "string" and
  (.properties.operation | has("enum") | not)
' plugin/schemas/go-request.schema.json >/dev/null

private_pattern="$(
  printf '%b' \
    '\142\141\142\171\062|\142\141\142\171\055\161\165\151\162\164|\150\157\162\163\145\171|\161\165\151\162\164|'
  printf '%b' \
    '\057\157\160\164\057\142\141\142\171\055\161\165\151\162\164|\057\166\141\162\057\154\151\142\057\142\141\142\171\055\161\165\151\162\164|'
  printf '%b' \
    '\142\141\142\171\134\056\152\157\142\134\056|\142\141\142\171\134\056\162\145\154\145\141\163\145\134\056'
)"
if rg --line-number --ignore-case "$private_pattern" \
  src scripts assets packaging plugin config Cargo.toml build.rs rust-toolchain.toml; then
  printf 'forbidden private runtime dependency found\n' >&2
  exit 1
fi

bash scripts/test-installer-static.sh
bash scripts/test-installer-isolated.sh --workspace "$test_workspace"
bash scripts/test-replacement-safety.sh --workspace "$test_workspace"
cargo test network::
cargo test machine::
cargo test remote::

if rg --line-number \
  'BEGIN (RSA |OPENSSH |EC )?PRIVATE KEY|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]+' \
  src scripts assets packaging plugin config; then
  printf 'tracked secret material found\n' >&2
  exit 1
fi
printf 'repository gate passed\n'
