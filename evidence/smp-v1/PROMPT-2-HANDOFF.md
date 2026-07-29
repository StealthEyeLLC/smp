# SMP v1 Prompt-2 handoff

- Repository: `StealthEyeLLC/smp`
- Branch: `smp-v1`
- Exact stable implementation commit: `3e304549f0eaf69b6562ce326320ff3a9348febc`
- Exact stable implementation tree: `b9bad10fed897322887f39b5ba0ddc650181d54f`
- Source commit: `6b8eb5c11adf131606a58122d7ebf7933a8fc7c0`
- Source tree: `82fc8ca80449c7e5643bb941319fd33f560ce7eb`
- Build command: `cargo build --locked --release && sudo bash scripts/build-assets.sh --stage all --workspace /var/lib/smp/provenance/prompt2/assets-3e304549f0eaf69b6562ce326320ff3a9348febc --release-binary "$PWD/target/release/smp"`
- Repository test command: `bash scripts/test-repository.sh`
- Installer: `scripts/install.sh`
- Replacement mode: `scripts/replace.sh`
- Detached launcher: `scripts/prompt2-launch.sh`
- Acceptance harness: `scripts/acceptance-host.sh`
- Expected installed paths: `/usr/local/bin/smp`, `/usr/lib/smp`, `/etc/smp`, `/var/lib/smp`, `/run/smp`.
- Firecracker: `1.15.1`, binary SHA-256 `7e8b57e88c459396d4680d83dcdd8c7f72305447cb55b11f4ac98ad70a3f7825`.
- Kernel: `6.1.178`, vmlinux SHA-256 `962f2a873c9c1fcc5a00b5b446f80781311b9019d0ad65ff6d99fa94d0f3d28b`.
- Rootfs: unavailable; required Debian `13.6` conflicts with snapshot-reported `13.5`.
- Asset manifest digest: unavailable because rootfs construction did not complete.
- Service files: `packaging/systemd/smp.service`, `packaging/systemd/smp-tunnel.service`.
- Health endpoint: `GET /healthz` through `/run/smp/mcp.sock`.
- Readiness endpoint: `GET /readyz` through `/run/smp/mcp.sock`.
- Plugin: display name `SMP`; namespace `smp`; only tool `go`; callable identity `smp.go`.
- Request schema: `plugin/schemas/go-request.schema.json`, schema version 1.
- Operation catalog: `describe`, `doctor`, `machine.create`, `machine.start`, `machine.wait`, `machine.status`, `machine.inspect`, `machine.stop`, `machine.kill`, `machine.reboot`, `machine.destroy`, `machine.reconcile`, `exec`, `file.upload`, `file.download`, `logs.read`, `raw.smp`, `raw.firecracker`, `result.get`, `result.read`, `result.wait`, `result.cancel`.
- Limits: 1 MiB inline output, 64 MiB total capture, 1 MiB result chunks, 86,400-second maximum timeout.
- Retention: requests 86,400 seconds; results 86,400 seconds.
- Durable Prompt-2 log: `/var/lib/smp/provenance/prompt2/prompt2-3e304549f0eaf69b6562ce326320ff3a9348febc.log`
- Durable Prompt-2 status: `/var/lib/smp/provenance/prompt2/prompt2-3e304549f0eaf69b6562ce326320ff3a9348febc.status`
- Prompt-2 command template: `sudo /usr/lib/smp/scripts/prompt2-launch.sh 3e304549f0eaf69b6562ce326320ff3a9348febc`
- Cleanup rules: verify recorded PID/start-time/executable/boot identity; remove only owned TAP/nft/socket/runtime resources; preserve credentials, provenance, and persistent disks unless explicit destructive flags are supplied.
- Old SMP code was not read or reused.
- Baby2 is not required by SMP after Prompt 1.

Prompt 2 must not run until the Debian version/snapshot requirement is reconciled.
