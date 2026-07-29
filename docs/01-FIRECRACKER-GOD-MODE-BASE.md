# Firecracker God-Mode Base

Status: Canonical product specification

Project: SMP — Smallest Maximum Power

Reviewed: 2026-07-29

## 1. Product contract

The first SMP product is a standalone Firecracker microVM system that provides genuine unrestricted UID 0 inside a full Linux guest.

The guest-root authority is literal within the guest boundary. SMP must not impose command allowlists, reduced Linux capabilities, restricted shells, sudo mediation, read-only root, artificial workload modes, package restrictions, outbound network filtering, or a curated operation catalog.

The guest remains bounded by the virtual hardware, kernel, resources, and connectivity actually assigned to the Firecracker microVM. Guest root is not host root. Arbitrary host-shell execution, host escape, undeclared host paths, and automatic host credential exposure are not part of this product contract.

## 2. Exact initial baseline

The first certified implementation lane is:

- host and guest architecture: `x86_64`;
- host: Linux with working hardware virtualization and usable KVM access;
- VMM: official Firecracker `v1.15.1` release binary;
- guest userspace: Debian `13.6` (`trixie`), with exact package provenance;
- guest kernel line: Linux `6.1`, built in a Firecracker-compatible boot format;
- default VirtIO transport: PCI;
- default local control: the `smp` CLI;
- optional ChatGPT control: one MCP tool, `smp.go`.

The implementation must pin the exact Firecracker release asset and record its SHA-256 digest. It must never download an unversioned `latest` binary during ordinary machine creation.

The initial Debian image build must pin the `trixie` suite rather than the moving `stable` alias. It must record the build timestamp, repository `InRelease` identities, installed package names and versions, and final root-filesystem digest. Rebuilding from newer repository contents is an explicit image update, not the same canonical image.

An upgrade to another Firecracker, Debian image, or kernel version is an explicit compatibility change. The old identities must remain in machine state so an existing machine or snapshot is never silently opened under an unknown baseline.

Firecracker requires host and guest to use the same CPU architecture. Cross-architecture emulation is not part of the product.

PCI in this specification means Firecracker's VirtIO PCI transport. It does not promise VFIO, GPU passthrough, arbitrary host PCI devices, USB passthrough, or a general-purpose PC hardware model.

Nested KVM inside the guest is not part of the first product contract unless explicitly added and proven later.

## 3. Smallest complete architecture

The canonical local architecture is:

```text
one SMP binary
one Firecracker process per running microVM
one machine directory
one writable root disk
one small read-only seed disk
one TAP interface in the default network mode
one Firecracker API socket
normal direct root SSH
bounded local logs
no mandatory SMP daemon
no database
no scheduler
no worker pool
no generalized job system
no policy engine
no receipt system
```

The optional ChatGPT path adds one running `smp serve` process or an even smaller equally complete MCP serving mode in the same binary. It must not create a second implementation of SMP lifecycle logic.

The normal launch path uses a generated Firecracker JSON configuration file and retains the API socket for post-boot native control.

## 4. Firecracker launch law

### 4.1 Pinned official binary

SMP uses the pinned official Firecracker binary, not an SMP fork. An operator may explicitly supply a different compatible binary through the raw path.

### 4.2 PCI by default

The canonical machine starts Firecracker with PCI VirtIO transport enabled. The kernel must include the required PCI and VirtIO PCI support.

MMIO remains available through an explicit operator override and the raw configuration path. SMP must not claim that PCI device hotplug is complete while upstream marks it developer preview.

### 4.3 Preserve Firecracker seccomp

Firecracker's default host-side seccomp filters remain enabled. Disabling them creates no additional guest-root power and therefore violates the smallest-mechanism rule by adding host risk without increasing the promised authority domain.

SMP must not add an in-guest seccomp policy.

### 4.4 Jailer scope

The power-first base does not require the Firecracker jailer. Production jailer integration belongs to later production hardening unless real implementation work proves it is required for correctness on the certified host.

Deferring the jailer does not authorize disabling Firecracker's built-in seccomp filters.

## 5. Canonical host lane

The certified host must expose `/dev/kvm` with read and write access and must pass a real KVM API probe. File existence alone is insufficient.

`smp doctor` must report at least:

- CPU architecture and virtualization support;
- host kernel version;
- `/dev/kvm` permissions and KVM API usability;
- available memory and disk space;
- TUN/TAP availability;
- nftables availability;
- IP-forwarding state;
- required filesystem and image-building tools;
- SSH client availability;
- Firecracker binary identity and digest;
- conflicts that prevent the selected machine from starting.

`smp doctor --fix` may install or configure ordinary unambiguous prerequisites for the declared host lane. It must print every change and fail rather than choose a weaker backend.

Local interactive commands may re-execute through `sudo` when host privileges are required. Non-interactive and remote modes must never wait on an unseen sudo prompt; they must already possess the required authority or fail exactly.

## 6. Canonical guest

The guest uses Debian 13.6 `trixie` with systemd, glibc, apt, OpenSSH, nftables, and ordinary Linux administration tools.

The canonical kernel is not assumed to be a distribution's compressed `/boot/vmlinuz` file. For `x86_64`, Firecracker requires an uncompressed ELF kernel image. SMP must build or acquire a Firecracker-compatible Linux 6.1 `vmlinux`, install its matching modules into the root filesystem, and record the exact kernel source identity, configuration digest, image digest, and module-tree identity.

The default boot path should not require an initramfs when the required boot-critical drivers and ext4 support are built into the kernel. Operator-supplied initrd/initramfs remains supported through the raw path.

The kernel configuration must preserve broad guest administration power, including support for:

- ext4 and loop devices;
- loadable modules;
- namespaces;
- cgroup v2;
- overlayfs;
- nftables;
- TUN/TAP, veth, and Linux bridging;
- VirtIO block, network, PCI, RNG, and console devices;
- vsock when selected;
- ordinary container runtimes supported by the kernel.

A VirtIO entropy device is enabled in the canonical machine so first-boot key generation and cryptographic workloads do not depend on weak or stalled entropy initialization.

On `x86_64`, the canonical boot arguments must include the serial console configuration needed for diagnostics and the kernel reboot behavior required to terminate the current Firecracker process cleanly. SMP must still treat reboot as a host-mediated lifecycle operation as defined below.

## 7. Writable storage

The root filesystem is a writable sparse ext4 image with a useful logical size. The canonical base image is never attached writable during ordinary machine operation.

Machine creation uses a reflink clone when the host filesystem supports it and a correct sparse-copy fallback otherwise. A fallback may be slower but must not weaken guest behavior.

The machine definition records the exact backing path, logical size, filesystem identity, writable status, and base-image identity for every disk.

The canonical root is selected by filesystem UUID or another stable identity rather than relying on incidental device enumeration. The seed filesystem uses a fixed label such as `SMP_SEED` and is mounted read-only by that identity.

SMP must support:

- persistent writable root disks;
- disposable writable root disks;
- operator-supplied compatible root filesystems;
- additional read-only or writable Firecracker block devices through the raw path;
- block-device rescan after an explicit backing-file growth operation.

SMP must refuse accidental simultaneous writable attachment of the same ordinary backing file to multiple running machines.

## 8. Minimal initialization

SMP must not require cloud-init or a permanent custom guest agent.

Machine-specific initialization uses a tiny read-only ext4 seed disk and a one-shot systemd unit. The seed may contain:

- hostname;
- root authorized public keys;
- network configuration;
- optional files;
- one optional arbitrary root initialization script.

The one-shot service:

1. finds and mounts the seed by its stable filesystem label;
2. validates the seed structure;
3. creates a unique machine ID when required;
4. generates unique guest SSH host keys;
5. installs root authorized keys;
6. configures networking and DNS;
7. applies optional files;
8. runs the optional root script exactly once;
9. records local completion or failure state;
10. exits permanently.

No private SSH key may be embedded in the base image or seed disk.

## 9. Networking

The zero-configuration path creates one TAP interface per machine, allocates and records deterministic private addressing with collision detection, configures a dedicated SMP nftables table for forwarding and masquerading, and provides working DNS.

SMP must not replace the host's global forwarding policy with an unconditional accept policy. It adds only the rules required for the declared SMP network and removes only rules it owns.

Simple TCP and UDP host-port publication uses nftables DNAT and does not require a proxy process.

SMP must permit operator-supplied networking without narrowing it, including:

- an existing TAP;
- an existing bridge;
- explicit guest and gateway addresses;
- an explicit MAC address;
- multiple supported network interfaces through raw configuration;
- native Firecracker rate-limiter configuration.

First-class multiple-interface convenience commands may follow the base, but raw Firecracker access must not block the capability.

SMP itself does not silently filter guest networking.

## 10. Local control surface

The primary zero-friction command is:

```bash
sudo smp up
```

With no name, it uses the machine name `default`. The command prepares or reuses pinned shared assets, creates or reuses the machine, starts it, waits for successful guest initialization and direct root SSH, and opens the root shell.

Machine identifiers must be validated canonical names and must map to exactly one machine directory. User input must not create path traversal or alias two names onto the same unintended state.

The canonical CLI includes at least:

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
```

`exec` passes an exact argument vector. SMP must not silently insert a shell. A shell is used only when the operator explicitly supplies one, such as `bash -lc`.

`exec` must preserve standard input, standard output, standard error, interactive terminal behavior, terminal resizing, signals, and the guest command's exit status as faithfully as SSH permits.

Persistent mode is the default. Disposable mode changes only writable-state lifetime, not guest power.

## 11. Lifecycle truth

SMP must distinguish at least:

- absent;
- created;
- starting;
- running;
- ready;
- stopped;
- crashed;
- stale.

A PID alone does not prove a running Firecracker instance. SMP must bind the machine to the expected executable, PID start time, API socket, configuration, and machine directory.

`ready` means the expected Firecracker process is running, the guest has completed initialization successfully, and direct root SSH works.

### 11.1 Reboot semantics

Firecracker does not provide a general in-place guest reboot contract. On the canonical `x86_64` lane, guest reboot with the required kernel arguments terminates the current Firecracker process.

Therefore `smp reboot NAME` means:

1. request a graceful guest reboot or shutdown;
2. verify termination of the old Firecracker process;
3. preserve the writable disks and machine definition;
4. start a new Firecracker process for the same machine;
5. wait for the guest to become ready again.

SMP must not claim that the original VMM process survived the reboot.

### 11.2 Stop and kill

`stop` attempts graceful guest shutdown and then handles any architecture-specific Firecracker process behavior explicitly. `kill` forcibly terminates the verified Firecracker process. Neither operation removes persistent writable state.

### 11.3 Serial behavior

Detached machines write serial output to bounded local storage. `smp logs` and `smp console` may follow that output. The base does not promise interactive serial input to a detached process.

When native interactive serial input is required, `smp start --foreground` exposes Firecracker's foreground console directly. This preserves the capability without adding a permanent console broker process.

## 12. Native-power escape hatches

SMP must never become the Firecracker capability ceiling.

It must provide direct access to:

- the Firecracker API socket;
- the exact generated Firecracker configuration;
- an operator-supplied complete Firecracker configuration;
- an operator-supplied Firecracker binary;
- an operator-supplied kernel, initrd, boot arguments, and root filesystem;
- arbitrary supported drives and network interfaces;
- block-device rescan;
- supported native features such as vsock, MMDS, entropy devices, rate limiters, snapshots, PMEM, memory controls, and CPU templates when present in the pinned release.

Full snapshots are stable native functionality in the pinned baseline and may receive first-class commands after the base lifecycle passes. Differential snapshots and PCI device hotplug remain outside the canonical completion claim while upstream marks them developer preview. MMDS remains an optional native feature rather than a dependency of SMP initialization.

A Firecracker snapshot is not a complete machine image. Guest memory and VMM/device state are separate from block-device files. Snapshot restore must bind the compatible Firecracker snapshot-data version, CPU model or template, host-kernel assumptions, disks, TAP devices, and vsock path. SMP must not claim that existing network or vsock connections survive restore.

Features not yet wrapped by a first-class SMP command remain reachable through the raw API or complete raw configuration path.

## 13. Standalone ChatGPT control

SMP includes an optional remote MCP mode so ChatGPT can operate SMP on an authorized VPS without Baby or another StealthEye repository.

The official integration exposes exactly one callable tool with the canonical identity:

```text
smp.go
```

For MCP implementations that namespace a server and its tools, the server/app identifier is `smp` and its only tool is `go`. No second callable tool is permitted.

Canonical path:

```text
ChatGPT -> smp.go -> SMP MCP endpoint on the authorized VPS -> SMP core -> Firecracker
```

Local path:

```text
operator -> smp CLI -> Firecracker
```

The remote endpoint is optional. Local SMP remains fully usable without ChatGPT, MCP, a plugin, or a public listener.

### 13.1 Reachability

The preferred ChatGPT connection uses a supported private secure MCP tunnel so the SMP endpoint does not require an open public inbound port.

When a direct remote endpoint is deliberately selected, it must use authenticated encrypted transport. Anonymous public control is never a valid zero-friction mode.

SMP itself adds no per-operation approval workflow. ChatGPT or workspace policy may still display platform-required confirmations for write actions; SMP must not misrepresent platform behavior.

### 13.2 Capability discovery

The first operation implemented by `smp.go` is `describe`.

`describe` returns:

- SMP version and build identity;
- request and response schema versions;
- supported operation names;
- operation argument schemas;
- Firecracker version and digest;
- Debian image and kernel identities;
- host architecture and certified status;
- limits for inline input, inline output, captured output, result retention, and timeouts;
- current machine names and summary state when requested.

The published MCP tool schema remains stable and broad. New SMP capabilities appear through the runtime `describe` catalog rather than by publishing additional tools.

### 13.3 Request envelope

The canonical envelope is small, strict, and extensible:

```json
{
  "schemaVersion": 1,
  "requestId": "018f6f3d-3e1a-7c20-9ca5-9f826e9349e5",
  "operation": "exec",
  "machine": "default",
  "argv": ["bash", "-lc", "apt update && apt install -y git"],
  "stdin": null,
  "timeoutSeconds": 300,
  "outputLimitBytes": 1048576,
  "detach": false,
  "options": {}
}
```

Only fields required by the selected operation are mandatory. `operation` is a string rather than a frozen enumeration so the one tool can gain capabilities without republishing its callable surface.

All command-bearing operations use exact argument arrays. A raw shell command string must not be silently constructed or reinterpreted.

### 13.4 Retry identity

`requestId` provides minimal network retry correctness. SMP calculates a deterministic digest of the normalized request.

- the same `requestId` with the same request digest returns the existing result or operation handle;
- the same `requestId` with a different request digest fails with a conflict;
- SMP must not create duplicate machines or duplicate long-running operations because ChatGPT retried a timed-out call;
- request records survive `smp serve` restart;
- a request record is never expired while its operation is active;
- terminal request records remain available for at least the retention interval advertised by `describe`.

The record contains the request ID, request digest, redacted operation summary, process identity where applicable, result state, and output paths. It need not retain raw credentials or complete sensitive stdin after execution.

This is a small request record, not a generalized transaction or receipt system.

### 13.5 Response envelope

The response preserves exact truth:

```json
{
  "schemaVersion": 1,
  "requestId": "018f6f3d-3e1a-7c20-9ca5-9f826e9349e5",
  "state": "completed",
  "exitCode": 0,
  "timedOut": false,
  "stdout": "",
  "stderr": "",
  "stdoutComplete": true,
  "stderrComplete": true,
  "resultHandle": null,
  "machineState": "ready",
  "error": null
}
```

Transport success must not be represented as operation success. Guest command failure, SMP failure, timeout, cancellation, transport loss, and output-capture exhaustion remain distinguishable.

### 13.6 Long output and disconnected operations

Inline output is bounded so one command cannot overflow the ChatGPT tool channel. Bounded inline output must not mean silently lost finite output.

When output exceeds the inline limit, SMP stores stdout and stderr in plain bounded-lifetime files and returns a `resultHandle`. The same `smp.go` tool supports `result.get` and `result.read` operations for status and chunked continuation.

Captured output has an explicit configurable maximum advertised by `describe`. Reaching that maximum must be reported exactly. The operator may raise the limit or direct output into a guest file when complete arbitrarily large output is required.

Long operations may run in detached mode and return a result handle. The same tool supports status, wait, read, and cancel operations.

This mechanism must remain smaller than a job system:

- no queue;
- no scheduler;
- no worker service;
- no automatic retry policy;
- no database;
- no dependency graph;
- no generalized workflow model.

A detached operation is one directly spawned process plus a small state file, verified by PID start time and output files. It exists only to survive an MCP call timeout or client disconnect without duplicating work.

After `smp serve` restarts, it must adopt a still-running detached operation from its verified process identity and state file or classify it honestly as terminal, failed, or stale. It must not start a duplicate operation merely because the serving process restarted.

### 13.7 File transfer

The one tool must support explicit guest file upload and download. Small content may travel inline with an explicit encoding. Larger content may use a temporary result handle served by the same SMP endpoint.

File transfer does not justify a second callable plugin tool or a generalized artifact service.

### 13.8 Power preservation

`smp.go` must not become a fixed command allowlist weaker than the local CLI.

A raw SMP operation accepts exact `smp` argument vectors. A raw Firecracker API operation accepts an exact method, path, headers, and body for the selected machine's API socket.

These raw paths expose the full SMP and Firecracker control surfaces. They do not silently become arbitrary host-shell execution, which is outside the declared product authority domain.

## 14. Minimum correctness

SMP must use:

- validated canonical machine identifiers;
- per-machine locking;
- exact Firecracker process identity;
- atomic machine-state replacement;
- explicit partial-failure reporting;
- bounded retained failure logs;
- current host truth rather than stale state-file claims;
- base-image immutability checks;
- deterministic machine and network identity;
- exact external-component version reporting.

A CLI process exiting must not stop a persistent microVM.

`reconcile` may reconstruct only unambiguous missing runtime resources. Ambiguous state must be reported rather than guessed into success.

No generalized receipts, signed evidence, append-only event chain, durable jobs, scheduler, or policy records are required in this phase.

## 15. Completion condition

The base is complete only when a real certified KVM host proves that:

- the pinned official Firecracker binary and digest are used;
- the pinned Debian 13.6 image provenance and final digest are recorded;
- the Firecracker-compatible kernel boots through PCI VirtIO by default;
- default Firecracker seccomp remains enabled;
- the canonical guest provides unrestricted root inside its declared boundary;
- package, filesystem, service, process, namespace, cgroup, network, firewall, module, and compiler operations work;
- persistent and disposable storage behave exactly as declared;
- host-mediated reboot reconnects to a new verified Firecracker process;
- raw configuration and API access preserve native power;
- `sudo smp up` provides the zero-friction local path;
- one ChatGPT MCP integration exposes only `smp.go` and reaches the complete SMP surface;
- retries, serving-process restarts, disconnections, long output, and long operations do not create duplicate work or false success;
- no private StealthEye implementation was imported without explicit authorization.
