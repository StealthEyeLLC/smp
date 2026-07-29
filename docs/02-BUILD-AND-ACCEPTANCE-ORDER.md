# Build and Acceptance Order

Status: Canonical implementation order

Project: SMP — Smallest Maximum Power

## 1. Execution rule

Implementation must proceed in the smallest number of complete passes. This is not a multi-document planning program.

From the current specification baseline, completion is divided into two implementation prompts:

1. finalize and commit the complete governing and product specifications;
2. implement, test on real KVM and Firecracker, correct failures, certify the base, and commit the finished result.

No extra planning prompts, artificial phase handoffs, or repeated approval loops are required unless the owner explicitly changes scope.

## 2. Build order

### 2.1 Foundation

1. Create the single Rust `smp` binary.
2. Add version reporting and a compact machine configuration format.
3. Implement exact path handling, atomic state writes, and per-machine locking.
4. Implement `smp doctor` and `smp doctor --fix` for the canonical host lane.
5. Pin and acquire the exact official Firecracker binary and required guest assets.

### 2.2 Canonical guest

1. Build the Debian stable root filesystem.
2. Use a general-purpose distro kernel, modules, and initramfs.
3. Build the one-shot seed-disk initialization path.
4. Generate unique machine identity and SSH host keys on first boot.
5. Provide direct unrestricted root SSH.
6. Support writable sparse disks and reflink clone with sparse-copy fallback.

### 2.3 Firecracker lifecycle

1. Generate the complete Firecracker configuration.
2. Launch one detached Firecracker process per running machine.
3. Retain the API socket and logs.
4. Verify process identity using more than PID existence alone.
5. Implement create, start, wait, status, inspect, stop, kill, reboot, and destroy.
6. Keep persistent machines alive independently of the invoking CLI process.
7. Implement explicit ephemeral writable-state cleanup.

### 2.4 Networking

1. Create one TAP interface per default machine.
2. Allocate deterministic private addressing with collision handling.
3. Configure nftables forwarding, masquerading, and optional port publication.
4. Provide working DNS.
5. Preserve explicit existing-TAP, bridge, address, MAC, and raw configuration modes.
6. Reconstruct unambiguous missing runtime networking through `smp reconcile`.

### 2.5 Operator experience

1. Implement `sudo smp up` as the complete zero-friction path.
2. Implement direct root `ssh` and arbitrary `exec`.
3. Preserve argv, stdin, stdout, stderr, terminal sizing, signals, and exit status.
4. Implement file copy, logs, console, wait, JSON output, and dry-run inspection.
5. Expose raw Firecracker API and complete operator-supplied configurations.

### 2.6 Single-tool ChatGPT integration

1. Add optional `smp serve` behavior to the same binary unless direct implementation proves a smaller equally complete mechanism.
2. Expose exactly one authenticated remote action corresponding to the official callable tool `smp.go`.
3. Implement the structured request and response envelope.
4. Route every structured operation through the same SMP core used by the local CLI.
5. Include a raw escape operation that reaches the complete SMP CLI and Firecracker API surface.
6. Do not add additional callable plugin tools.
7. Do not depend on Baby or another StealthEye repository or service.

### 2.7 Native-power follow-ons

After the complete base passes, expose without architectural replacement:

1. multiple disks;
2. multiple network interfaces;
3. disk growth and offline mount;
4. image capture and clone;
5. vsock;
6. MMDS;
7. pause and resume;
8. full snapshots and restore;
9. other stable native features in the pinned Firecracker release.

Raw access must make unwrapped native features available before first-class convenience commands exist.

## 3. Required real-host acceptance

Certification requires a real supported KVM host. Mock-only success is insufficient.

The acceptance run must prove:

1. a real Firecracker process boots the canonical kernel and root filesystem;
2. `id -u` returns `0`;
3. root has the guest kernel's available capability set without SMP reductions;
4. apt can update and install an ordinary package;
5. root can create filesystems, loop devices, mounts, bind mounts, tmpfs, and overlayfs;
6. root can create and control namespaces and cgroup v2;
7. root can create nftables state, TUN devices, veth pairs, and a Linux bridge;
8. root can load and unload a suitable shipped guest kernel module;
9. root can create and manage systemd services, users, groups, and processes;
10. the guest can compile and run a native program;
11. the guest has working DNS and outbound connectivity;
12. a guest listener is reachable through direct or published networking;
13. SSH, arbitrary exec, file transfer, and interactive TTY behavior work;
14. guest reboot reconnects correctly;
15. persistent state survives Firecracker stop and restart;
16. ephemeral writable state is removed after destruction;
17. custom kernel, rootfs, and raw Firecracker API paths work;
18. mutating one machine does not alter the canonical base or another machine;
19. disabling KVM produces a clear failure rather than a fallback;
20. no restricted shell, ForceCommand, sudo mediation, or SMP command filter exists.

## 4. `smp.go` acceptance

The optional ChatGPT connection is complete only when one installed plugin exposes one callable tool named exactly:

```text
smp.go
```

The acceptance run must prove through that one tool:

1. host diagnosis;
2. machine creation and startup;
3. unrestricted arbitrary guest-root command execution;
4. exact guest command failure and exit status reporting;
5. bounded stdout and stderr with explicit truncation truth;
6. file transfer in both directions;
7. machine inspection and lifecycle control;
8. raw SMP command access;
9. raw Firecracker API access;
10. a newly added SMP operation can be reached by extending the request schema without adding another callable plugin tool.

A plugin with multiple per-operation callable tools fails the canonical contract.

A one-tool plugin that exposes only a curated subset and lacks a full-power escape path also fails the canonical contract.

## 5. Deferred systems

The following must not delay the power-first base:

- policy engines;
- approval workflows;
- fine-grained authorization;
- multi-tenant control;
- signed receipts;
- evidence ledgers;
- durable jobs;
- workflow engines;
- schedulers;
- fleet management;
- remote artifact services;
- a web dashboard;
- a separate SMP control-plane repository.

The remote endpoint still requires the minimum authentication and transport integrity necessary to prevent accidental anonymous public control. That requirement must not expand into an unrelated governance architecture during this phase.

## 6. Completion report

The final implementation commit must report:

- exact SMP commit and tree;
- exact Firecracker version and binary digest;
- exact guest kernel and Debian identities;
- certified host class;
- commands executed;
- test results;
- unsupported native Firecracker capabilities, if any;
- known limits inherent to Firecracker, the selected guest kernel, or assigned virtual hardware;
- proof that the repository remained standalone;
- proof that `smp.go` is the only callable plugin tool;
- final clean repository status.

No production deployment or activation is implied by implementation certification unless separately authorized.
