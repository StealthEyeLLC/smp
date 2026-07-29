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
- Debian `13.6` (`trixie`) userspace;
- a Firecracker-compatible Linux 6.1 guest kernel and matching modules.

The implementation must not resolve `latest` or the moving Debian `stable` alias during ordinary operation.

The Debian image build must record:

- the `trixie` suite;
- image build timestamp;
- mirror and repository `InRelease` identities;
- installed package names and exact versions;
- final root-filesystem digest.

A rebuild against changed repository contents is a new image identity even when the suite name remains `trixie`.

The kernel build must produce an uncompressed ELF `vmlinux` for `x86_64`. A compressed distribution `/boot/vmlinuz` file must not be assumed to satisfy Firecracker's boot contract.

The default VMM launch uses PCI VirtIO transport. MMIO remains an explicit override. PCI transport does not imply VFIO, GPU passthrough, arbitrary host PCI passthrough, USB passthrough, or nested KVM.

Firecracker's default host-side seccomp remains enabled.

## 4. Build order

### 4.1 Single binary foundation

1. Create the Rust `smp` binary.
2. Add exact version and build-commit reporting.
3. Define compact strict machine, request, and response schema versions.
4. Implement canonical path resolution and directory layout.
5. Validate machine identifiers so they cannot cause path traversal or alias unintended state.
6. Implement atomic file replacement.
7. Implement per-machine locks.
8. Implement PID plus process-start-time identity checks.
9. Implement plain human output and stable JSON output.
10. Implement `smp describe`.
11. Add no database, daemon requirement, scheduler, worker pool, plugin framework, or generalized job system.

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
2. Record the exact release asset and binary SHA-256 digest.
3. Cache verified assets for offline reuse.
4. Add explicit `--offline` behavior.
5. Build or acquire the exact Firecracker-compatible Linux 6.1 kernel.
6. Record the kernel version, source identity, configuration digest, module-tree identity, and image digest.
7. Build Debian 13.6 from the pinned `trixie` suite inputs.
8. Record Debian repository and package provenance plus the final rootfs digest.
9. Reject mismatched or corrupt cached assets.

### 4.4 Canonical guest

1. Build the Debian 13.6 `trixie` root filesystem.
2. Install systemd, apt, OpenSSH, nftables, filesystem tools, networking tools, compiler support, and ordinary administration utilities.
3. Build boot-critical ext4 and VirtIO support into the kernel.
4. Install matching loadable modules into the root filesystem.
5. Avoid a default initramfs when it is not required to boot.
6. Enable PCI, VirtIO PCI, VirtIO block, VirtIO network, VirtIO RNG, namespaces, cgroup v2, overlayfs, nftables, TUN, veth, and bridge support.
7. Configure direct key-based root SSH without sudo mediation or `ForceCommand`.
8. Keep private SSH keys out of the base image and seed.
9. Build the one-shot seed-disk initialization path.
10. Label the seed filesystem `SMP_SEED` and mount it read-only by that identity.
11. Select the canonical root by filesystem UUID or another stable identity rather than incidental device enumeration.
12. Generate unique machine ID and guest SSH host keys on first boot.
13. Enable a VirtIO entropy device by default.
14. Include canonical serial and `x86_64` reboot boot arguments.

### 4.5 Storage

1. Create an immutable canonical base image.
2. Create useful sparse writable machine disks.
3. Use reflink cloning when supported.
4. Use a correct sparse-copy fallback otherwise.
5. Record exact base, backing, filesystem, and image identities.
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
3. Prefer a supported private secure MCP tunnel so no public inbound port is required.
4. When direct remote exposure is selected, require authenticated encrypted transport.
5. Route every operation through the same SMP core as the local CLI.
6. Implement `describe` as the runtime capability-discovery operation.
7. Keep `operation` open-ended in the stable request schema so future SMP capabilities do not require more callable tools.
8. Implement strict schema versioning and exact argument arrays.
9. Normalize each request and calculate a deterministic request digest.
10. Implement request retry identity: the same request ID and digest returns the existing result; conflicting reuse fails.
11. Persist active request records across `smp serve` restart.
12. Retain terminal request records for the interval advertised by `describe`.
13. Store only the request digest, redacted operation summary, process identity, result state, and output references required for retry correctness.
14. Implement exact response truth for operation state, exit code, timeout, cancellation, stdout, stderr, truncation, capture exhaustion, error, and machine state.
15. Implement result handles for long output and long operations.
16. Advertise inline-output, captured-output, retention, and timeout limits through `describe`.
17. Implement `result.get`, `result.read`, `result.wait`, and `result.cancel` through the same `smp.go` tool.
18. Keep detached-operation state to one process identity, small state file, and output files.
19. On `smp serve` restart, adopt the same verified running detached process or classify it honestly; never duplicate it.
20. Add no queue, scheduler, worker service, automatic retry engine, workflow graph, database, or generalized artifact service.
21. Implement guest file transfer through the same tool.
22. Implement raw exact SMP argv access.
23. Implement raw Firecracker API method, path, headers, and body access.
24. Do not expose arbitrary host shell as though it were part of the guest-root contract.
25. Do not add any second callable plugin tool.
26. Do not depend on Baby or another private StealthEye repository or service.

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

Snapshot implementation must preserve the distinction between VMM/memory snapshot files and separately managed block-device files. Restore must verify the snapshot-data version, Firecracker compatibility, CPU or template assumptions, host requirements, disks, networking, and vsock bindings. It must not claim existing network or vsock connections survive restore.

Differential snapshots and PCI device hotplug must not be represented as completed stable features while upstream marks them developer preview.

## 5. Required real-host acceptance

Certification requires the actual authorized KVM host and a real Firecracker microVM. Mock-only success is insufficient.

The acceptance run must prove:

1. the exact Firecracker `v1.15.1` release asset and binary digest;
2. KVM API usability;
3. the Debian 13.6 suite, repository, package, build-time, and rootfs provenance;
4. an uncompressed ELF Linux 6.1 kernel boots;
5. the kernel and module identities match;
6. PCI VirtIO is the default transport;
7. MMIO can be selected explicitly;
8. no unsupported VFIO, GPU, arbitrary PCI, USB, or nested-KVM capability is claimed;
9. Firecracker default seccomp is not disabled;
10. a VirtIO entropy device is present;
11. the Debian root filesystem boots with systemd;
12. the root and seed are found through stable filesystem identities;
13. `id -u` returns `0`;
14. root has the kernel's available capability set without SMP reduction;
15. apt can update and install an ordinary package;
16. root can create loop devices, filesystems, mounts, bind mounts, tmpfs, and overlayfs;
17. root can create and control namespaces and cgroup v2;
18. root can create nftables state, a TUN device, veth pair, and Linux bridge;
19. root can load and unload a suitable installed guest kernel module;
20. root can create and manage systemd services, users, groups, and processes;
21. the guest can install a compiler and build and run a native program;
22. an ordinary container runtime can run a container when supported by the selected kernel;
23. guest DNS and outbound connectivity work;
24. a guest listener is reachable through direct or published networking;
25. SSH, arbitrary exec, file transfer, and interactive TTY behavior work;
26. exact nonzero guest command exit status is returned;
27. invalid and path-traversing machine identifiers are rejected;
28. persistent state survives stop and start;
29. `smp reboot` terminates the old VMM, starts a new VMM, and reconnects;
30. the old and new Firecracker process identities differ after reboot;
31. disposable writable state is removed only when declared;
32. mutating one machine does not alter the base or another machine;
33. serial and failure logs are bounded and retained sufficiently for diagnosis;
34. custom kernel, rootfs, boot arguments, and complete configuration work;
35. raw Firecracker API access works;
36. block rescan works if disk growth is implemented;
37. host reboot or runtime loss can be reconciled from current machine state where unambiguous;
38. disabling or hiding KVM produces an exact failure and no fallback;
39. no restricted shell, `ForceCommand`, sudo mediation, command rewriting, or SMP guest-command filter exists.

## 6. `smp.go` acceptance

The ChatGPT connection is complete only when one installed app exposes one callable tool with the canonical identity:

```text
smp.go
```

The acceptance run must prove through that one tool:

1. `describe` returns the live SMP operation catalog, schemas, versions, provenance, and limits;
2. host diagnosis works;
3. machine creation and startup work;
4. unrestricted arbitrary guest-root execution works;
5. exact argv reaches the guest without implicit shell insertion;
6. exact guest failure and exit status are reported;
7. timeout, cancellation, transport failure, output-capture exhaustion, and operation failure are distinguishable;
8. retrying an identical `requestId` and digest does not duplicate the operation;
9. reusing a `requestId` with a different digest fails;
10. request retry identity survives `smp serve` restart;
11. long inline output returns a handle and can be read in chunks;
12. the advertised total captured-output limit is enforced and reported honestly;
13. a detached operation survives the initiating tool-call disconnect or timeout;
14. restarting `smp serve` adopts the same verified detached process rather than duplicating it;
15. status, wait, read, and cancel work through the same callable tool;
16. guest file upload and download work;
17. machine inspection and lifecycle control work;
18. raw exact SMP argv access works;
19. raw Firecracker API access works;
20. a newly implemented operation appears through `describe` without adding another callable tool;
21. no arbitrary host-shell capability is falsely presented as part of guest root;
22. no second app tool is exposed.

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
- developer-preview PCI device hotplug;
- VFIO or GPU passthrough;
- USB passthrough;
- nested KVM.

Minimum authentication, encrypted transport, request identity, process identity, restart adoption, and bounded output are correctness requirements, not a broad safety architecture.

## 8. Completion report

The final implementation commit must report:

- exact starting and final SMP commits and trees;
- exact Firecracker release asset and SHA-256 digest;
- exact kernel version, source identity, configuration digest, image digest, and module identity;
- exact Debian suite, repository identities, package versions, build timestamp, and rootfs digest;
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
- proof that `smp.go` was the only callable app tool;
- proof that retries and `smp serve` restarts did not duplicate operations;
- final clean repository status;
- remote verification of the final commit and tree.
