# SMP

**Smallest Maximum Power**

SMP is a standalone project for building the smallest possible implementations that preserve the maximum real power of the underlying system.

The first product is an unrestricted-root Firecracker microVM: a minimal machine that gives its operator genuine root authority inside the guest without command allowlists, capability reductions, policy wrappers, artificial product tiers, or container-style privilege ceilings.

SMP does not optimize for the smallest demo, the smallest source tree, or the smallest feature list. It optimizes for the smallest complete mechanism that retains 100 percent of the selected system's practical power.

## Governing rule

For every component, choose the smallest design that preserves full power. A smaller design that removes power is rejected. A larger design that adds no power is rejected.

## Current phase

The current phase builds power first.

Safety systems, extensive receipts, approval workflows, policy engines, production hardening, multi-tenant controls, and governance layers are intentionally deferred. Only the minimum correctness evidence needed to establish that the system actually works is included.

This is not permission to silently weaken the product. Any later safety implementation must wrap or govern the completed power surface without reducing it unless an explicit decision changes the SMP constitution.

## Standalone rule

SMP is a new standalone repository. No code, architecture, schemas, scripts, services, conventions, or implementation fragments are imported from another StealthEye repository unless the owner explicitly authorizes that specific import.

External operating systems, kernels, Firecracker, standard packages, language toolchains, and documented platform interfaces are dependencies, not inherited StealthEye implementations.

## Canonical documents

1. [SMP Constitution](docs/00-SMP-CONSTITUTION.md) — binding project laws, definitions, constraints, and decision rules.
2. [Firecracker God-Mode Base](docs/01-FIRECRACKER-GOD-MODE-BASE.md) — complete contract for the first product, including its standalone remote-control path.
3. [Build and Acceptance Order](docs/02-BUILD-AND-ACCEPTANCE-ORDER.md) — exact implementation sequence and completion gates.

## First product definition

The first product must boot a Firecracker microVM and provide unrestricted UID 0 inside a full Linux guest. Guest root must be able to install software, alter the guest operating system, create services, load supported guest modules, manage mounts, namespaces, cgroups, networking, firewall state, processes, users, package repositories, filesystems, and nested software stacks without an SMP-imposed restriction layer.

The guest is a virtual machine, not a container. Its root authority ends at the hardware and resources presented to the microVM. Host resources are not silently exposed, and host authority is not part of the guest-root contract.

## ChatGPT control

SMP includes its own optional standalone ChatGPT connection. It does not require Baby, another StealthEye control plane, or a collection of per-operation plugin tools.

The official integration exposes exactly one callable tool:

```text
smp.go
```

Every remote SMP capability is expressed through that one stable interface. New SMP features extend the request schema instead of adding more callable tools.

Canonical path:

```text
ChatGPT -> smp.go -> SMP on the authorized VPS -> Firecracker
```

Local SMP use does not require ChatGPT or a plugin:

```bash
sudo smp up
```

## Project status

Canonical specification foundation in progress. No implementation has been certified yet.
