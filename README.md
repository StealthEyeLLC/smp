# SMP

**Smallest Maximum Power** is a standalone Rust controller for genuine UID 0 inside Firecracker microVMs.

This branch contains the Prompt-1 repository implementation. It does not claim that the binary, assets, services, tunnel, microVM, or ChatGPT connection have been executed in this repository-only tab. Direct host bootstrap and end-to-end certification belong to Prompt 2.

## Canonical lane

- x86_64 host and guest
- official Firecracker `v1.15.1`
- PCI VirtIO by default; explicit MMIO mode
- Firecracker host-side seccomp retained
- Debian `13.6` `trixie` userspace
- uncompressed Linux `6.1.177` ELF `vmlinux` and matching modules
- immutable ext4 base image
- persistent or disposable writable machine disks
- direct key-based root SSH
- exact-argv guest execution without an implicit shell
- bidirectional guest file transfer
- one executable: `smp`
- one MCP namespace: `smp`
- one callable tool: `go`
- canonical callable identity: `smp.go`

SMP does not fall back to containers, QEMU, chroots, namespaces, host execution, or another private control plane when Firecracker fails.

## Repository layout

- `src/` — Rust CLI, lifecycle, state, Firecracker, networking, guest, request, result, and MCP implementation
- `assets/guest/` — one-shot read-only seed initialization
- `assets/guest-tools/` — exact-argv and bounded file-transfer guest helpers
- `scripts/build-assets.sh` — pinned Firecracker, Linux, modules, and Debian ext4 construction
- `scripts/create-seed.sh` — read-only `SMP_SEED` ext4 creation
- `scripts/bootstrap.sh` — direct standalone installation entrypoint
- `scripts/test-repository.sh` — repository build and correctness checks
- `scripts/acceptance.sh` — real Firecracker acceptance run for Prompt 2
- `packaging/systemd/` — `smp.service` and `smp-tunnel.service`
- `plugin/` — private `SMP` app metadata and the stable one-tool schema
- `evidence/smp-firecracker-god-mode-v1/` — minimal Prompt-1 result and Prompt-2 handoff

## Local CLI

```text
smp up
smp create
smp start
smp ssh
smp exec
smp cp
smp logs
smp console
smp status
smp inspect
smp wait
smp stop
smp kill
smp reboot
smp destroy
smp reconcile
smp doctor
smp api
smp describe
smp version
smp serve
```

The zero-friction path is:

```bash
sudo smp up
```

With no name, SMP uses `default`, prepares or reuses verified assets, creates or reuses the persistent machine, starts it, waits for guest initialization and direct root SSH, and opens a root shell.

Exact guest argv is passed after `--`:

```bash
sudo smp exec default -- /usr/bin/id -u
sudo smp exec default -- bash -lc 'apt-get update && apt-get install -y hello'
```

One side of `smp cp` must use an absolute `guest:` path:

```bash
sudo smp cp default ./local.bin guest:/root/local.bin
sudo smp cp default guest:/root/result.bin ./result.bin
```

## Firecracker lifecycle truth

SMP records the executable path and digest, PID and process start time, API socket, generated configuration, disks, networking, and machine directory. It validates the full process identity before signaling or using the selected machine's API socket.

`reboot` is host-mediated. SMP requests guest shutdown or reboot, verifies that the old Firecracker process exits, preserves persistent disks and the machine definition, launches a new Firecracker process, waits for readiness, and reports both identities. It does not claim an in-place Firecracker reboot.

Generated launch configuration uses `--enable-pci` for the default PCI path. MMIO is an explicit alternate. SMP never disables Firecracker seccomp.

## State and installation

```text
/usr/local/bin/smp
/usr/lib/smp/
/etc/smp/
/etc/smp/credentials/
/var/lib/smp/
/var/lib/smp/machines/
/var/lib/smp/assets/
/var/lib/smp/requests/
/var/lib/smp/results/
/run/smp/
```

`/var/lib/smp` is runtime state only. Source is never installed there. Persistent disks are not silently removed by service shutdown or ordinary uninstall.

## Remote control

`smp serve` listens on loopback by default at:

```text
http://127.0.0.1:7745/mcp
```

Health and readiness endpoints are:

```text
http://127.0.0.1:7745/healthz
http://127.0.0.1:7745/readyz
```

The private ChatGPT app display name is exactly `SMP`. Its MCP server identifier is `smp`, and its only callable tool is `go`, yielding `smp.go`.

Every remote request uses one stable envelope:

```json
{
  "schemaVersion": 1,
  "requestId": "unique-id",
  "operation": "exec",
  "machine": "default",
  "argv": ["bash", "-lc", "id -u"],
  "stdin": null,
  "timeoutSeconds": 300,
  "outputLimitBytes": 1048576,
  "detach": false,
  "options": {}
}
```

Call `describe` first. Runtime operations include machine lifecycle, exact guest execution, file transfer, logs, raw SMP argv, selected-machine Firecracker API access, and retained-result get/read/wait/cancel. The operation string remains open so capabilities can grow without adding a second MCP tool.

The same request ID with the same deterministic digest returns the existing terminal result or active handle. Reusing the ID with different content fails. Detached operations retain a strong process identity and bounded stdout/stderr so `smp serve` can adopt them without starting duplicates.

## Dedicated tunnel and app

`packaging/systemd/smp-tunnel.service` runs a dedicated token-managed Cloudflare Tunnel client. Its secret is read from `/etc/smp/credentials/tunnel-token`, never placed in Git or command arguments. The tunnel routes a dedicated private hostname to `http://127.0.0.1:7745`; it must not reuse another product's identity or credential.

`plugin/SMP.json` and `plugin/smp.go.schema.json` define the private app. Replace the tunnel-hostname placeholder only after the dedicated tunnel route exists, then add the custom MCP connection in the ChatGPT workspace. The platform action is intentionally not claimed by Prompt 1.

## Verification order

After direct bootstrap on the authorized host:

```bash
sudo /usr/lib/smp/test-repository.sh
sudo smp doctor --fix
sudo smp assets
sudo /usr/lib/smp/acceptance.sh
```

The repository scripts acquire and verify assets at first use, build the Rust binary, run unit/static checks, launch real Firecracker machines, and exercise root authority, filesystems, networking, persistence, isolation, reboot, raw API access, and no-fallback behavior.

## Standalone law

SMP must not use Baby, Fix, Horsey, Quirt, or another private StealthEye operator for source implementation, build, installation, testing, tunnel setup, operation, recovery, upgrade, cleanup, or removal. GitHub repository management is allowed, but GitHub is not an SMP runtime dependency.

See [docs/04-LEAST-THEATER-LAW.md](docs/04-LEAST-THEATER-LAW.md).
