# SMP Prompt-1 Test Results

## Repository result

- Source creation through the GitHub App: `PASS`
- Branch fast-forward relation to authorized source commit: `PASS`
- Repository-only standalone boundary: `PASS`
- Static test and real acceptance scripts prepared: `PASS`

## Not executed in Prompt 1

The current tab had no authorized direct host execution path and therefore did not claim any of the following:

- `cargo build`
- Rust unit or integration tests
- shell static checks
- pinned asset downloads and digest checks
- kernel or module build
- Debian ext4 image build
- real Firecracker launch
- guest systemd, SSH, UID 0, filesystem, networking, module, compiler, persistence, isolation, reboot, raw API, or no-fallback acceptance
- `smp.service` or `smp-tunnel.service` installation or status
- live tunnel reachability
- custom ChatGPT app registration or fresh-tab `smp.go` invocation

## Prepared checks

`scripts/test-repository.sh` performs the repository build/tests, optional formatting and clippy checks, shell syntax and optional ShellCheck, plugin metadata validation, single-tool validation, and private-runtime dependency scan.

`scripts/acceptance.sh` performs the real Firecracker acceptance lane after direct bootstrap, including root authority, package installation, filesystems, loop devices, mounts, tmpfs, overlayfs, namespaces, cgroup v2, nftables, TUN, veth, bridge, module load/unload, systemd control, users/groups, native compilation, DNS, outbound network, published ports, exact argv, nonzero status, file transfer, persistence, host-mediated reboot, disposable cleanup, machine isolation, raw API, immutable base verification, and explicit no-fallback failure.

## Required Prompt-2 disposition

Prompt 2 must run the direct bootstrap, correct any observed compile or runtime failures in the same branch, execute the prepared checks, produce the installed binary and asset digests, configure the dedicated tunnel, enable the private app, and certify `smp.go` from a fresh tab.
