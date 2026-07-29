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
- one optional private ChatGPT plugin named `SMP`;
- exactly one callable plugin tool: `smp.go`.

PCI means Firecracker's VirtIO PCI transport. The initial contract does not claim VFIO, GPU or arbitrary host PCI passthrough, USB passthrough, cross-architecture emulation, or nested KVM.

SMP never resolves an unversioned latest VMM or the moving Debian `stable` alias during ordinary machine creation. It never falls back to a container, chroot, QEMU, host shell, reduced guest, or simulation when KVM is unavailable.

## Current phase

The current phase builds and proves power first.

Safety systems, extensive receipts, approval workflows, policy engines, production jailer integration, multi-tenant controls, and governance layers are intentionally deferred. Minimum authentication, encrypted remote transport, exact process identity, retry identity, restart adoption, bounded output, and honest success or failure remain required because they are correctness, not safety theater.

Any later safety implementation must govern the completed power surface without silently redefining a restricted subset as the original product.

## Least-theater law

**Use the least amount of theater possible, always.**

Implementation begins after the smallest source-integrity check needed to avoid editing the wrong repository or commit. Established owner-confirmed infrastructure is not repeatedly re-proven. Verification happens at the point where its result changes the next action. A tool limitation is not evidence that established infrastructure is absent.

For the current build, the authorized VPS and working KVM lane are accepted established facts. Prompt 1 must begin implementation through the available durable execution surface rather than searching for a special host-shell or KVM-probe operation. Fresh-tab `smp.go` verification belongs to Prompt 2.

## Standalone rule

SMP is a new standalone repository. No code, architecture, schemas, scripts, services, conventions, or implementation fragments are imported from another private StealthEye repository unless the owner explicitly authorizes that exact import.

Public operating systems, kernels, Firecracker, standard packages, language toolchains, Git, GitHub, and documented platform interfaces are dependencies or development tools, not inherited StealthEye implementations.

SMP may share the authorized Linux VPS with Baby while remaining standalone. It must have its own process, service, endpoint or tunnel identity, credentials, sockets, state root, logs, and recovery behavior. Baby must not be in the SMP runtime path.

## Canonical documents

1. [SMP Constitution](docs/00-SMP-CONSTITUTION.md) — binding project laws, definitions, constraints, and decision rules.
2. [Firecracker God-Mode Base](docs/01-FIRECRACKER-GOD-MODE-BASE.md) — exact first-product contract, Firecracker baseline, lifecycle semantics, and standalone ChatGPT control.
3. [Build and Acceptance Order](docs/02-BUILD-AND-ACCEPTANCE-ORDER.md) — implementation order and capability acceptance.
4. [Standalone Integrations and Operations](docs/03-STANDALONE-INTEGRATIONS-AND-OPERATIONS.md) — the `SMP` plugin, dedicated VPS connection, GitHub App role, service layout, credentials, upgrade behavior, and independence tests.
5. [Least-Theater Execution Law](docs/04-LEAST-THEATER-LAW.md) — constitutional amendment requiring immediate execution, point-of-use verification, and minimal ceremony.

Document 5 overrides conflicting preflight, verification, evidence, and prompt language in documents 1 through 4.

## Local use

The canonical zero-friction path is:

```bash
sudo smp up
```

That command must prepare or reuse verified pinned assets, create or reuse the default machine, start Firecracker, wait for successful initialization and direct root SSH, and open the root shell.

## ChatGPT control

The ChatGPT-facing plugin is named exactly:

```text
SMP
```

Its underlying MCP server/app identifier is `smp`. It exposes exactly one tool, `go`, producing the canonical callable identity:

```text
smp.go
```

Canonical path:

```text
ChatGPT -> SMP plugin -> smp.go -> dedicated SMP connection -> smp serve -> SMP core -> Firecracker
```

The first remote operation is `describe`, which returns the live versioned capability catalog, external-component identities, schema versions, server identity, and transport limits. New SMP capabilities extend that runtime catalog rather than adding more callable tools.

Long output and long-running operations remain reachable through result handles and the same `smp.go` tool. Request retry identity and detached-operation adoption survive `smp serve` restart without requiring a scheduler, worker pool, database, or generalized job system.

The raw SMP and Firecracker API paths preserve the complete declared control surface. They do not silently add arbitrary host-shell execution, which is outside the guest-root authority contract.

The private plugin may use a dedicated secure MCP tunnel. The SMP tunnel, credential, service, and endpoint must be separate from Baby even when both products use the same VPS.

## GitHub repository access

The existing GitHub App installation for `StealthEyeLLC` was verified on 2026-07-29 to include `StealthEyeLLC/smp` with reported `admin`, `maintain`, `push`, `pull`, and `triage` repository capabilities. The default branch is `main`.

That GitHub App is authorized development and repository-management infrastructure. It may be used to inspect, commit, branch, review, and remotely verify SMP according to its current permissions. It is not part of the installed SMP runtime and must not be required to start or operate microVMs.

Check the repository, source commit, clean state, and usable write path once at mission start. Recheck access only when an actual repository operation fails or current evidence indicates access changed. GitHub App private keys and tokens must never be committed, embedded in a guest image, placed in the seed disk, or exposed through `smp.go`.

## Reboot truth

Firecracker does not provide a general in-place guest reboot contract. On the canonical `x86_64` lane, `smp reboot` gracefully terminates the old Firecracker process, starts a new verified process against the same persistent machine state, and reconnects. SMP must not claim the original VMM survived.

## Project status

The canonical specification is complete across the five documents above. Implementation is split into two honest prompts: Prompt 1 builds SMP immediately through the available durable execution surface and prepares the dedicated tunnel and plugin; Prompt 2 runs in a fresh tab and verifies the actual `smp.go` integration end to end.