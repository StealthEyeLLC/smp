# Build and Acceptance Order

Status: Canonical implementation order

Project: SMP — Smallest Maximum Power

Reviewed: 2026-07-29

## 1. Execution rule

The specification pass is complete after this review. One implementation mission remains:

1. implement the complete base;
2. test it on the real authorized KVM host;
3. correct every failure;
4. certify local SMP and the single `smp.go` integration;
5. commit and remotely verify the finished result.

Do not create extra planning prompts, artificial phase handoffs, or repeated approval loops. The stages below are execution order inside that one mission, not separate conversations.

Do not stop after scaffolding, unit tests, mock success, partial boot, local-only success, or plugin-only success.

No production deployment or activation is implied unless separately authorized.

## 2. Pre-edit verification

Before implementation changes:

1. verify the repository is `StealthEyeLLC/smp`;
2. verify `main` contains the canonical documents;
3. record the exact starting commit and tree;
4. verify the working copy is clean;
5. verify no code, schemas, scripts, or architecture have been imported from another private StealthEye repository;
6. inspect the authorized target host without mutating production services;
7. verify `x86_64`, hardware virtualization, `/dev/kvm` read/write access, and a real KVM API probe;
8. verify TUN/TAP, nftables, disk, memory, and required build tools;
9. fail explicitly if the host cannot run the full Firecracker path.

No container, chroot, QEMU, user namespace, or host-shell simulation may substitute for failed KVM verification.

## 3. Exact external baseline

The initial implementation pins:

- Firecracker `v1.15.1`;
- the exact official `x86_64` release asset;
- the SHA-256 digest of that asset;
- Debian stable userspace;
- a Firecracker-compatible Linux 6.1 guest kernel and matching modules.

The implementation must not resolve `latest` during ordinary operation.

The kernel build must produce an uncompressed ELF `vmlinux` for `x86_64`. A compressed distribution `/boot/vmlinuz` file must not be assumed to satisfy Firecracker's boot contract.

The default VMM launch uses PCI VirtIO transport. MMIO remains an explicit override.

Firecracker's default host-side seccomp remains enabled.

## 4. Build order

### 4.1 Single binary foundation

1. Create the Rust `smp` binary.
2. Add exact version and build-commit reporting.
3. Define compact strict machine and request schema versions.
4. Implement canonical path resolution and directory layout.
5. Implement atomic file replacement.
6. Implement per-machine locks.
7. Implement PID plus process-start-time identity checks.
8. Implement plain human output and stable JSON output.
9. Implement `smp describe`.
10. Add no database, daemon requirement, scheduler, worker pool, plugin framework, or generalized job system.

### 4.2 Host diagnosis and setup

1. Implement `smp doctor`.
2. Probe the KVM API rather than checking only for `/dev/kvm`.
3. Report host architecture, kernel, virtualization, memory, disk, TUN/TAP, nftables, forwarding, SSH, and asset status.
4. Implement `smp doctor --fix` only for ordinary unambiguous prerequisites.
5. Print every host mutation.
6. Support local sudo re-execution.
7. Make non-interactive privilege failure exact; never wait on an invisible password prompt.

### 4.3 Pinned assets

1. Acquire and verify Firecracker `v1.15.1`.
2. Record the binary SHA-256 digest.
3. Cache verified assets for offline reuse.
4. Add explicit `--offline` behavior.
5. Build or acquire the exact Firecracker-compatible Linux 6.1 kernel.
6. Record the kernel version, source identity, configuration digest, module-tree identity, and image digest.
7. Reject mismatched or corrupt cached assets.

### 4.4 Canonical guest

1. Build the Debian stable root filesystem.
2. Install systemd, apt, OpenSSH, nftables, filesystem tools, networking tools, compiler support, and ordinary administration utilities.
3. Build boot-critical ext4 and VirtIO support into the kernel.
4. Install matching loadable modules into the root filesystem.
5. Avoid a default initramfs when it is not required to boot.
6. Enable PCI, VirtIO PCI, VirtIO block, VirtIO network, VirtIO RNG, namespaces, cgroup v2, overlayfs, nftables, TUN, veth, and bridge support.
7. Configure direct key-based root SSH without sudo mediation or `ForceCommand`.
8. Keep private SSH keys out of the base image and seed.
9. Build the one-shot seed-disk initialization path.
10. Generate unique machine ID and guest SSH host keys on first boot.
11. Enable a VirtIO entropy device by default.
12. Include canonical serial and `x86_64` reboot boot arguments.

### 4.5 Storage

1. Create an immutable canonical base image.
2. Create useful sparse writable machine disks.
3. Use reflink cloning when supported.
4. Use a correct sparse-copy fallback otherwise.
5. Record exact base and backing identities.
6. Prevent accidental simultaneous writable use of one ordinary backing file.
7. Implement persistent and disposable modes without changing guest power.
8. Implement explicit disk growth and Firecracker block rescan when included in the base CLI.

### 4.6 Firecracker lifecycle

1. Generate the complete Firecracker JSON configuration.
2. Launch with `--enable-pci` by default.
3. Retain the API socket for post-boot control.
4. Preserve Firecracker's default seccomp configuration.
5. Launch one detached Firecracker process per running machine.
6. Record executable identity, PID, PID start time, API socket, config, and machine directory.
7. Implement states: absent, created, starting, running, ready, stopped, crashed, and stale.
8. Implement create, start, wait, status, inspect, stop, kill, reboot, and destroy.
9. Keep persistent machines alive after the invoking CLI exits.
10. Implement explicit disposable-state cleanup.
11. Retain bounded failure and serial logs.
12. Implement foreground launch for native interactive serial access.

### 4.7 Correct reboot behavior

On the canonical `x86_64` lane, implement reboot as a host-mediated restart:

1. request graceful guest reboot or shutdown;
2. verify the old Firecracker process terminates;
3. preserve disks and machine identity;
4. start a new verified Firecracker process;
5. wait for successful initialization and root SSH;
6. report the old and new process identities.

Do not claim that Firecracker performed an in-place reboot.

### 4.8 Networking

1. Create one TAP interface per default machine.
2. Allocate deterministic private addressing and record collision resolution.
3. Create a dedicated SMP nftables table and owned chains.
4. Configure only required forwarding, masquerading, and port-publication rules.
5. Do not replace the host's global forwarding policy.
6. Provide usable guest DNS with an explicit override.
7. Preserve existing-TAP, bridge, address, gateway, MAC, and raw network modes.
8. Reconstruct only unambiguous missing runtime networking through `smp reconcile`.
9. Remove only SMP-owned runtime resources.

### 4.9 Local operator experience

1. Implement `sudo smp up` as the complete zero-friction path.
2. Default the unnamed machine to `default`.
3. Reuse verified cached assets and an existing machine idempotently.
4. Implement direct root `ssh`.
5. Implement arbitrary `exec` with exact argv and no implicit shell.
6. Preserve stdin, stdout, stderr, exit status, signals, TTY allocation, and resize.
7. Implement file copy in both directions.
8. Implement bounded logs and serial following.
9. Implement inspect, wait, dry-run, human output, and JSON output.
10. Expose the raw Firecracker API socket and complete generated configuration.
11. Accept operator-supplied Firecracker binary, kernel, initrd, boot arguments, rootfs, drives, NICs, and complete configuration.

### 4.10 Single-tool ChatGPT integration

1. Implement optional MCP serving in the same `smp` binary, preferably `smp serve`.
2. Use the MCP server/app identifier `smp` and expose exactly one tool named `go`, yielding the canonical identity `smp.go`.
3. Prefer private secure MCP tunneling so no public inbound port is required.
4. When direct remote exposure is selected, require authenticated encrypted transport.
5. Route every operation through the same SMP core as the local CLI.
6. Implement `describe` as the runtime capability-discovery operation.
7. Keep `operation` open-ended in the stable request schema so future SMP capabilities do not require more callable tools.
8. Implement strict schema versioning and exact argument arrays.
9. Implement request retry identity: same request ID and same request returns the existing result; conflicting reuse fails.
10. Implement exact response truth for operation state, exit code, timeout, cancellation, stdout, stderr, truncation, error, and machine state.
11. Implement result handles for long output and long operations.
12. Implement `result.get`, `result.read`, `result.wait`, and `result.cancel` through the same `smp.go` tool.
13. Keep detached-operation state to one process identity, small state file, and output files.
14. Add no queue, scheduler, worker service, automatic retry engine, workflow graph, database, or generalized artifact service.
15. Implement guest file transfer through the same tool.
16. Implement raw exact SMP argv access.
17. Implement raw Firecracker API method, path, headers, and body access.
18. Do not expose arbitrary host shell as though it were part of the guest-root contract.
19. Do not add any second callable plugin tool.
20. Do not depend on Baby or another private StealthEye repository or service.

### 4.11 Native-power exposure

The complete base must expose unwrapped stable Firecracker functionality through raw configuration or API even when no first-class convenience command exists.

First-class follow-on commands may cover:

- multiple disks;
- multiple network interfaces;
- disk growth and offline mount;
- image capture and clone;
- vsock;
- MMDS;
- pause and resume;
- full snapshots and restore;
- PMEM;
- memory controls;
- CPU templates;
- rate limiters.

Differential snapshots and PCI device hotplug must not be represented as completed stable features while upstream marks them developer preview.

## 5. Required real-host acceptance

Certification requires the actual authorized KVM host and a real Firecracker microVM. Mock-only success is insufficient.

The acceptance run must prove:

1. the exact Firecracker `v1.15.1` binary digest;
2. KVM API usability;
3. an uncompressed ELF Linux 6.1 kernel boots;
4. PCI VirtIO is the default transport;
5. MMIO can be selected explicitly;
6. Firecracker default seccomp is not disabled;
7. a VirtIO entropy device is present;
8. the Debian root filesystem boots with systemd;
9. `id -u` returns `0`;
10. root has the kernel's available capability set without SMP reduction;
11. apt can update and install an ordinary package;
12. root can create loop devices, filesystems, mounts, bind mounts, tmpfs, and overlayfs;
13. root can create and control namespaces and cgroup v2;
14. root can create nftables state, a TUN device, veth pair, and Linux bridge;
15. root can load and unload a suitable installed guest kernel module;
16. root can create and manage systemd services, users, groups, and processes;
17. the guest can install a compiler and build and run a native program;
18. an ordinary container runtime can run a container when supported by the selected kernel;
19. guest DNS and outbound connectivity work;
20. a guest listener is reachable through direct or published networking;
21. SSH, arbitrary exec, file transfer, and interactive TTY behavior work;
22. exact nonzero guest command exit status is returned;
23. persistent state survives stop and start;
24. `smp reboot` terminates the old VMM, starts a new VMM, and reconnects;
25. the old and new Firecracker process identities differ after reboot;
26. disposable writable state is removed only when declared;
27. mutating one machine does not alter the base or another machine;
28. serial and failure logs are bounded and retained sufficiently for diagnosis;
29. custom kernel, rootfs, boot arguments, and complete configuration work;
30. raw Firecracker API access works;
31. block rescan works if disk growth is implemented;
32. host reboot or runtime loss can be reconciled from current machine state where unambiguous;
33. disabling or hiding KVM produces an exact failure and no fallback;
34. no restricted shell, `ForceCommand`, sudo mediation, command rewriting, or SMP guest-command filter exists.

## 6. `smp.go` acceptance

The ChatGPT connection is complete only when one installed plugin/app exposes one callable tool with the canonical identity:

```text
smp.go
```

The acceptance run must prove through that one tool:

1. `describe` returns the live SMP operation catalog and versions;
2. host diagnosis works;
3. machine creation and startup work;
4. unrestricted arbitrary guest-root execution works;
5. exact argv reaches the guest without implicit shell insertion;
6. exact guest failure and exit status are reported;
7. timeout, cancellation, transport failure, and operation failure are distinguishable;
8. retrying an identical `requestId` does not duplicate the operation;
9. reusing a `requestId` with a different payload fails;
10. long output returns a handle and can be read completely in chunks;
11. a detached operation survives the initiating tool-call disconnect or timeout;
12. status, wait, read, and cancel work through the same callable tool;
13. guest file upload and download work;
14. machine inspection and lifecycle control work;
15. raw exact SMP argv access works;
16. raw Firecracker API access works;
17. a newly implemented operation appears through `describe` without adding another callable tool;
18. no second plugin tool is exposed.

A one-tool integration that exposes only a curated subset and lacks the raw escape paths fails the contract.

## 7. Deferred systems

The following must not delay the power-first base:

- policy engines;
- owner-approval workflows;
- fine-grained authorization;
- multi-tenant controls;
- signed receipts;
- evidence ledgers;
- durable workflow jobs;
- schedulers;
- worker fleets;
- remote artifact services;
- a web dashboard;
- a separate SMP control-plane repository;
- production jailer integration;
- differential snapshots;
- developer-preview PCI device hotplug.

Minimum authentication, encrypted transport, request identity, process identity, and bounded output are correctness requirements, not a broad safety architecture.

## 8. Completion report

The final implementation commit must report:

- exact starting and final SMP commits and trees;
- exact Firecracker release asset and SHA-256 digest;
- exact kernel version, source identity, configuration digest, image digest, and module identity;
- exact Debian userspace identity;
- exact certified host identity and KVM probe result;
- commands executed;
- unit, integration, and real-host acceptance results;
- local CLI acceptance results;
- `smp.go` acceptance results;
- unsupported or deferred native Firecracker capabilities;
- known limits inherent to Firecracker, the selected kernel, architecture, or assigned virtual hardware;
- proof that Firecracker seccomp remained enabled;
- proof that PCI was the default and MMIO remained selectable;
- proof that the repository remained standalone;
- proof that `smp.go` was the only callable plugin tool;
- final clean repository status;
- remote verification of the final commit and tree.
