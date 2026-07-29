# Firecracker God-Mode Base

Status: Canonical product specification

Project: SMP — Smallest Maximum Power

## 1. Product contract

The first SMP product is a standalone Firecracker microVM system that provides genuine unrestricted UID 0 inside a full Linux guest.

The guest root authority is literal within the guest boundary. SMP must not impose command allowlists, reduced Linux capabilities, restricted shells, sudo mediation, read-only root, artificial workload modes, package restrictions, outbound network filtering, or a curated operation catalog.

The guest remains bounded by the virtual hardware, kernel, resources, and connectivity actually assigned to the Firecracker microVM. Guest root is not host root.

## 2. Smallest complete architecture

The canonical architecture is:

```text
one SMP binary
one Firecracker process per running microVM
one machine directory
one writable root disk
one small seed disk
one TAP interface
one Firecracker API socket
normal root SSH
no mandatory SMP daemon
no database
no scheduler
no job system
no policy engine
no receipt system
```

SMP should be implemented as one Rust CLI and use an exact pinned official Firecracker release. The default launch path uses a generated Firecracker configuration file while retaining the API socket for direct native control.

## 3. Canonical host lane

The first certified host lane is a supported Linux VPS or bare-metal host with usable KVM access.

SMP must provide:

- `smp doctor` to report exact host readiness.
- `smp doctor --fix` to perform ordinary unambiguous prerequisite repair.
- automatic root re-execution when a requested host operation requires it.
- exact failure when KVM or another mandatory full-power prerequisite is unavailable.
- no silent fallback to containers, chroot, QEMU, a local shell, or a reduced guest.

## 4. Canonical guest

The canonical guest uses Debian stable with systemd, glibc, apt, OpenSSH, a general-purpose distro kernel, matching modules, and an initramfs.

The root filesystem is a writable sparse ext4 image. Reflink cloning should be used when supported, with a correct sparse-copy fallback.

The image must support ordinary Linux administration, including:

- package installation;
- service creation and control;
- users and groups;
- mounts and filesystems;
- namespaces and cgroup v2;
- nftables;
- TUN, veth, and Linux bridge creation;
- supported guest kernel-module loading;
- compilers and ordinary native workloads;
- nested software stacks and container runtimes supported by the guest kernel.

## 5. Minimal initialization

SMP must not require cloud-init or a permanent custom guest agent.

Machine-specific initialization uses a tiny read-only ext4 seed disk and a one-shot systemd unit. The seed may contain:

- hostname;
- root authorized keys;
- network configuration;
- optional files;
- one optional arbitrary root initialization script.

The one-shot service applies the declared configuration, runs the script, records its local completion state, and exits. It is not a resident control plane.

## 6. Networking

The zero-configuration path creates one TAP interface per machine, deterministic private addressing, nftables forwarding and masquerading, and working DNS.

SMP must also permit operator-supplied networking without narrowing it, including an existing TAP, existing bridge, explicit MAC address, explicit guest address, multiple supported network interfaces, and native Firecracker rate-limiter configuration.

Simple host-port publication must be available without a proxy service.

SMP must not silently filter guest networking.

## 7. Local control surface

The primary zero-friction command is:

```bash
sudo smp up
```

With no name, it uses the machine name `default`. The command prepares missing shared assets, creates or reuses the machine, starts it, waits for guest initialization and root SSH, and opens the root shell.

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
smp version
```

`exec` must preserve arbitrary argv, standard streams, terminal behavior, signals, and the guest command exit status as faithfully as the SSH transport permits.

Persistent mode is the default. Ephemeral mode changes only writable-state lifetime, not guest power.

## 8. Native-power escape hatches

SMP must never become the Firecracker capability ceiling.

It must provide direct access to:

- the Firecracker API socket;
- the exact generated Firecracker configuration;
- an operator-supplied complete Firecracker configuration;
- an operator-supplied Firecracker binary;
- an operator-supplied kernel, initrd, boot arguments, and root filesystem;
- arbitrary supported drives and network interfaces;
- supported native features such as vsock, MMDS, entropy devices, rate limiters, snapshots, PMEM, memory controls, and CPU templates when present in the pinned release.

Features not yet wrapped by a first-class SMP command remain reachable through the raw path.

## 9. Standalone ChatGPT control

SMP includes an optional remote-control mode so ChatGPT can operate SMP on an authorized VPS without depending on Baby or another StealthEye repository.

The official ChatGPT integration exposes exactly one callable tool:

```text
smp.go
```

This is a binding interface rule.

The integration must not expose separate callable tools for machine creation, command execution, file transfer, snapshots, images, inspection, networking, or lifecycle actions. All operations pass through `smp.go` using a structured request.

Canonical path:

```text
ChatGPT -> smp.go -> SMP remote endpoint on the authorized VPS -> SMP core -> Firecracker
```

Local path:

```text
operator -> smp CLI -> Firecracker
```

The remote endpoint is optional. Local SMP must remain fully usable without it.

### 9.1 Request envelope

The stable request envelope should remain small and extensible:

```json
{
  "operation": "exec",
  "machine": "default",
  "argv": ["bash", "-lc", "apt update && apt install -y git"],
  "stdin": null,
  "timeoutSeconds": 300,
  "outputLimitBytes": 1048576,
  "options": {}
}
```

Only fields needed by an operation are required. Future capabilities extend operation values and options; they do not create additional plugin tools.

### 9.2 Required operation classes

The one tool must be capable of expressing:

- host diagnosis and setup;
- machine lifecycle;
- arbitrary guest-root command execution;
- interactive-session preparation;
- file upload and download through explicit paths;
- machine and runtime inspection;
- image and disk operations;
- network configuration and port publication;
- snapshot and restore operations;
- raw Firecracker API requests;
- raw SMP CLI access for capabilities not yet represented structurally.

### 9.3 Power preservation

`smp.go` must not become a fixed command allowlist that is weaker than the local SMP CLI.

A raw escape operation must preserve access to the full SMP command surface and native Firecracker API. Structured operations are conveniences, not authority boundaries.

The response must preserve exact success or failure, exit status, bounded stdout and stderr, truncation truth, and relevant resulting machine state.

### 9.4 Minimal remote component

The remote bridge should be the smallest component that creates the required reachability:

- preferably an optional `smp serve` mode in the same binary;
- one authenticated HTTPS endpoint for `smp.go`;
- no separate database, scheduler, workflow engine, or resident guest agent;
- no dependency on another StealthEye service;
- no per-operation plugin proliferation.

Authentication and public exposure are necessary transport correctness, not a broad safety architecture. Exact production hardening remains outside the current power-first scope.

## 10. Minimum correctness

SMP must use per-machine locking, exact Firecracker process identity, atomic machine-state replacement, explicit partial-failure reporting, retained failure logs, and current host truth rather than trusting stale state files.

A CLI process exiting must not stop a persistent microVM.

`reconcile` may reconstruct only unambiguous missing runtime resources. Ambiguous state must be reported rather than guessed into success.

No generalized receipts, signed evidence, append-only event chain, durable jobs, or policy records are required in this phase.

## 11. Completion condition

This product is complete only when a real KVM host proves that the canonical guest can boot, provide unrestricted root, install software, manage the operating system, exercise the promised kernel and networking facilities, reboot, persist, stop, restart, and be destroyed without an SMP-imposed capability restriction.

The same installation must also prove that ChatGPT can reach the complete SMP surface through the single `smp.go` tool when the optional remote mode is enabled.
