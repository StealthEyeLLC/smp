# SMP Prompt-2 Handoff

## Repository identity

- Repository: `StealthEyeLLC/smp`
- Branch: `build/smp-firecracker-god-mode-v1`
- Authorized source commit: `0994877ca12e9bd0d375b8af9f748e674e602d82`
- Repository implementation checkpoint before evidence: `d069870f2d61a9c83cd4cc99b0c333a239df78ae`
- Resolve the exact final evidence-head commit from the immutable bootstrap command supplied with Prompt-1 completion.
- Resolve its exact tree after checkout with `git rev-parse HEAD^{tree}`; the current GitHub connector does not expose commit tree identity.

## Expected installation

- SMP version: `0.1.0`
- Installed binary: `/usr/local/bin/smp`
- Installed binary digest: generated and verified by `scripts/bootstrap.sh`, then recorded in `/etc/smp/install.json`
- Local MCP endpoint: `http://127.0.0.1:7745/mcp`
- Health: `http://127.0.0.1:7745/healthz`
- Readiness: `http://127.0.0.1:7745/readyz`
- Request schema version: `1`
- Response schema version: `1`
- Plugin display name: `SMP`
- MCP namespace: `smp`
- Only tool: `go`
- Expected callable identity: `smp.go`

## Pinned external lane

- Firecracker: official `v1.15.1` x86_64 archive, verified before extraction
- Kernel: Linux `6.1.177`, uncompressed x86_64 ELF `vmlinux`, matching modules
- Rootfs: Debian `13.6` `trixie`, ext4, systemd, direct key-based root SSH
- Default transport: PCI VirtIO
- Explicit alternate: MMIO
- Base rootfs: immutable during ordinary machine use

## Available operations

- `describe`
- `doctor`
- `machine.create`
- `machine.start`
- `machine.wait`
- `machine.status`
- `machine.inspect`
- `machine.stop`
- `machine.kill`
- `machine.reboot`
- `machine.destroy`
- `machine.reconcile`
- `exec`
- `file.upload`
- `file.download`
- `logs.read`
- `raw.smp`
- `raw.firecracker`
- `result.get`
- `result.read`
- `result.wait`
- `result.cancel`

## Limits and retention

- Inline output: `1,048,576` bytes
- Total captured output: `67,108,864` bytes
- Maximum request timeout: `86,400` seconds
- Request retention: `604,800` seconds
- Result retention: `604,800` seconds

## Prompt-2 execution order

1. Run the exact immutable one-line Termius bootstrap command from the Prompt-1 completion report.
2. Supply the dedicated SMP-only tunnel token only through the bootstrap's hidden interactive prompt. Press Enter to leave the tunnel disabled until the dedicated tunnel exists.
3. Run `/usr/lib/smp/prompt2-handoff.sh` and preserve its JSON output.
4. Run `sudo smp doctor --fix`.
5. Run `sudo smp assets` to acquire, verify, build, and record the pinned assets at first use.
6. Run `sudo /usr/lib/smp/acceptance.sh` and correct actual failures on this branch with fast-forward commits.
7. Configure the dedicated private tunnel hostname to the local origin and confirm `smp-tunnel.service` is active.
8. Perform the one external platform action in `plugin/REGISTER.md`: add the private custom MCP app named exactly `SMP` at `https://<dedicated-smp-hostname>/mcp` and confirm discovery shows only `go`.
9. Open a fresh ChatGPT tab, provide Prompt 2, and call `smp.go` with `operation: "describe"` first.

## Retained test machine

The acceptance script preserves the stopped persistent machine `smp-cert-persistent` only after all acceptance checks pass. Disposable, failure, and secondary test machines are removed.

## Exact remaining external actions

1. Execute the one-line direct bootstrap on the authorized VPS through Termius.
2. Provide a dedicated SMP-only Cloudflare Tunnel token and private hostname.
3. Add the private ChatGPT custom MCP app named `SMP` using that hostname.

Fresh-tab invocation belongs to Prompt 2. Prompt 1 has not claimed it.
