# SMP

**Smallest Maximum Power**

SMP is a standalone project for building the smallest possible implementations that preserve the maximum real power of the selected underlying system.

The first product is an unrestricted-root Firecracker microVM: a minimal machine that gives its operator genuine UID 0 inside the guest without command allowlists, capability reductions, policy wrappers, artificial product tiers, or container-style privilege ceilings.

SMP minimizes total mechanism only after the complete declared power contract is preserved. A smaller design that removes power is rejected. A larger design that adds no power, necessary correctness, or essential operability is also rejected.

## Current product baseline

The first certified lane is intentionally exact:

- `x86_64` Linux host and guest;
- real KVM access;
- official Firecracker `v1.15.1`, pinned by exact release asset and SHA-256 digest;
- PCI VirtIO transport by default, with explicit MMIO override;
- Firecracker's default host-side seccomp retained;
- Debian `13.6` (`trixie`) userspace, pinned by repository and package provenance plus rootfs digest;
- Firecracker-compatible uncompressed ELF Linux 6.1 kernel with matching modules;
- writable persistent or disposable ext4 machine state;
- direct key-based root SSH;
- one local `smp` binary;
- one optional ChatGPT callable tool: `smp.go`.

PCI means Firecracker's VirtIO PCI transport. The initial contract does not claim VFIO, GPU or arbitrary host PCI passthrough, USB passthrough, cross-architecture emulation, or nested KVM.

SMP never resolves an unversioned latest VMM or the moving Debian `stable` alias during ordinary machine creation. It never falls back to a container, chroot, QEMU, host shell, reduced guest, or simulation when KVM is unavailable.

## Current phase

The current phase builds and proves power first.

Safety systems, extensive receipts, approval workflows, policy engines, production jailer integration, multi-tenant controls, and governance layers are intentionally deferred. Minimum authentication, encrypted remote transport, exact process identity, retry identity, restart adoption, bounded output, and honest success or failure remain required because they are correctness, not safety theater.

Any later safety implementation must govern the completed power surface without silently redefining a restricted subset as the original product.

## Standalone rule

SMP is a new standalone repository. No code, architecture, schemas, scripts, services, conventions, or implementation fragments are imported from another private StealthEye repository unless the owner explicitly authorizes that exact import.

Public operating systems, kernels, Firecracker, standard packages, language toolchains, and documented platform interfaces are dependencies, not inherited StealthEye implementations.

## Canonical documents

1. [SMP Constitution](docs/00-SMP-CONSTITUTION.md) — binding project laws, definitions, constraints, and decision rules.
2. [Firecracker God-Mode Base](docs/01-FIRECRACKER-GOD-MODE-BASE.md) — exact first-product contract, Firecracker baseline, lifecycle semantics, and standalone ChatGPT control.
3. [Build and Acceptance Order](docs/02-BUILD-AND-ACCEPTANCE-ORDER.md) — the single remaining implementation mission, execution order, and real-host completion gates.

## Local use

The canonical zero-friction path is:

```bash
sudo smp up
```

That command must prepare or reuse verified pinned assets, create or reuse the default machine, start Firecracker, wait for successful initialization and direct root SSH, and open the root shell.

## ChatGPT control

SMP includes an optional standalone MCP mode. It does not require Baby or another StealthEye control plane.

The MCP server/app identifier is `smp` and it exposes exactly one tool, `go`, producing the canonical callable identity:

```text
smp.go
```

Canonical path:

```text
ChatGPT -> smp.go -> SMP MCP endpoint on the authorized VPS -> SMP core -> Firecracker
```

The first remote operation is `describe`, which returns the live versioned capability catalog, external-component identities, and transport limits. New SMP features extend that runtime catalog rather than adding more callable tools.

Long output and long-running operations remain reachable through result handles and the same `smp.go` tool. Request retry identity and detached-operation adoption survive `smp serve` restart without requiring a scheduler, worker pool, database, or generalized job system.

The raw SMP and Firecracker API paths preserve the complete declared control surface. They do not silently add arbitrary host-shell execution, which is outside the guest-root authority contract.

## Reboot truth

Firecracker does not provide a general in-place guest reboot contract. On the canonical `x86_64` lane, `smp reboot` gracefully terminates the old Firecracker process, starts a new verified process against the same persistent machine state, and reconnects. SMP must not claim the original VMM survived.

## Project status

Canonical specification corrected and complete. No implementation has been certified yet. One implementation mission remains: build, test on the real authorized KVM host, correct failures, certify local SMP and `smp.go`, commit, and remotely verify the finished result.
