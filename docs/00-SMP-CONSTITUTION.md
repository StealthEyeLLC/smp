# SMP Constitution

Status: Canonical governing specification

Project: SMP — Smallest Maximum Power

Repository: StealthEyeLLC/smp

## 1. Purpose

SMP exists to build the smallest possible implementation of a selected capability while retaining the maximum real power of that capability.

The project rejects the common trade in which a system is called minimal because its authority, compatibility, control surface, or practical usefulness has been reduced. SMP is minimal in mechanism, not minimal in power.

The project also rejects the opposite failure: adding frameworks, layers, services, abstractions, metadata, governance, orchestration, or ceremony that do not create concrete additional power.

The first SMP product is a Firecracker microVM that provides genuine unrestricted root authority inside the guest.

## 2. Controlling equation

Every design decision is governed by one rule:

**Select the smallest implementation that preserves the complete power promised by the current product contract.**

A design is invalid when either of the following is true:

1. It is smaller because it removes, narrows, filters, simulates, gates, or weakens promised power.
2. It is larger without creating additional promised power, necessary correctness, or required operability.

Power is the fixed requirement. Size is minimized only after power is preserved.

## 3. Meaning of smallest

Within SMP, smallest means the least total mechanism required to deliver the complete selected capability.

Total mechanism includes:

1. Source code.
2. Runtime processes.
3. Dependencies.
4. Configuration.
5. Persistent state.
6. Background services.
7. Control-plane layers.
8. Network protocols.
9. Build steps.
10. Operational steps.
11. Required privileges.
12. Maintenance burden.
13. Failure modes.
14. Concepts an operator must understand.

A reduction in one category does not count as smaller when it creates a larger or weaker system elsewhere. For example, deleting a controller is not a valid reduction if it replaces deterministic lifecycle behavior with undocumented manual steps.

## 4. Meaning of maximum power

Maximum power means that the selected capability is real, direct, complete, and operator-controlled.

A capability is not maximum power when it is:

1. Simulated rather than executed.
2. Exposed only through an allowlist.
3. Restricted to preapproved commands.
4. Missing essential flags or arguments.
5. Limited to a curated subset of files, packages, processes, devices, syscalls, or networking operations.
6. Replaced with a higher-level abstraction that cannot express the underlying operation.
7. Available only through an interactive path when an exact programmatic path is possible.
8. Available only programmatically when direct interactive control is part of the promised capability.
9. Silently downgraded when the full path fails.
10. Hidden behind arbitrary product tiers.
11. Subject to an SMP-imposed capability ceiling that is not inherent to the underlying selected platform.

Maximum power does not mean undefined scope. Every product must state its authority domain. Full power is then required inside that domain.

For the first product, the authority domain is the Firecracker guest. Unrestricted guest root is required. Host root, host escape, and undeclared host resource access are not part of that domain.

## 5. God-mode rule

When SMP describes a component as god mode, unrestricted, full power, or root, those terms are literal within the declared authority domain.

God mode must not be implemented as:

1. A privileged-looking user with reduced Linux capabilities.
2. A container root mapped to an unprivileged host identity and presented as equivalent to a VM root contract.
3. A command proxy with a fixed operation catalog.
4. A sudo wrapper that permits only selected commands.
5. A remote shell with filtered syntax.
6. A filesystem editor that cannot control the operating system.
7. A package runner without process, mount, network, service, device, and kernel-facing control.
8. A temporary demonstration that loses authority after reboot.
9. A fake success response when the underlying operation was not completed.

For the Firecracker base, god mode means genuine UID 0 in a Linux guest with the complete capability set made available by the guest kernel and the virtual hardware assigned to that microVM.

## 6. No watered-down additions

Every addition to SMP must satisfy all of the following:

1. Its exact power contribution is stated.
2. Its authority domain is stated.
3. Its limits are inherent, explicit, and not disguised.
4. It exposes the complete selected function rather than a curated subset.
5. It does not weaken an already completed capability.
6. It does not add a second inferior path that becomes the accidental default.
7. It does not silently fall back to a reduced mode.
8. It is the smallest known implementation that retains the full promised function.
9. It has a direct acceptance test proving the power exists.
10. Its removal would reduce real power, necessary correctness, or essential operability.

A capability may be absent because it has not yet been added. A capability must never be added in a knowingly weakened form and represented as complete.

## 7. Incremental scope without dilution

SMP may be built in narrow stages. Narrow stages are not watered down when their boundaries are explicit.

The correct pattern is:

1. Define a small capability envelope.
2. Implement 100 percent of that envelope.
3. Prove it directly.
4. Freeze its contract.
5. Add the next capability envelope without reducing the first.

The incorrect pattern is:

1. Claim a broad capability.
2. Implement a constrained subset.
3. Call the constraints safety, simplicity, a minimum viable product, or future work.

SMP favors narrow completeness over broad incompleteness.

## 8. Standalone repository law

SMP is a standalone repository and a standalone design authority.

No implementation may import, copy, adapt, transplant, vendor, mirror, cherry-pick, or structurally reproduce private StealthEye code from another repository unless the owner explicitly authorizes that exact source and purpose.

This prohibition includes:

1. Source files.
2. Scripts.
3. Schemas.
4. Database formats.
5. Service definitions.
6. Internal libraries.
7. Controllers.
8. Job systems.
9. Receipt systems.
10. Policy engines.
11. Deployment systems.
12. Naming conventions that carry hidden architecture.
13. Test harnesses.
14. Generated artifacts.
15. Commit history.
16. Branch history.
17. Copied documentation presented as new design.

General knowledge is allowed. Standard public dependencies are allowed. Public operating systems, Linux, Firecracker, language toolchains, package managers, and documented platform APIs are allowed. Their use does not make SMP a derivative of another StealthEye repository.

When a future import is authorized, the authorization must identify the repository, exact material, reason, and whether later synchronization is permitted. Silence is not authorization.

## 9. No hidden platform inheritance

SMP must not quietly become a client of another StealthEye control plane.

Unless explicitly authorized, SMP must not depend on another StealthEye repository or service for:

1. Scheduling.
2. Jobs.
3. Machines.
4. Artifacts.
5. Receipts.
6. Authentication.
7. Authorization.
8. Policy.
9. Deployment.
10. Storage.
11. Networking.
12. Recovery.
13. Cleanup.
14. Logging.
15. Configuration.
16. Secrets.
17. Build execution.
18. Release management.

SMP may be operated using ordinary development tools, Git, GitHub, a Linux host, and public dependencies. Its product must remain understandable and buildable from this repository plus declared public dependencies.

## 10. Power-first phase

The current phase is explicitly power first.

The project will establish the unrestricted capability before designing the later full safety implementation.

During this phase, SMP must not spend significant design or implementation effort on:

1. Policy engines.
2. Approval workflows.
3. Fine-grained authorization.
4. Multi-tenant isolation policy.
5. Signed receipt systems.
6. Evidence ledgers.
7. Attestation frameworks.
8. Compliance reporting.
9. Governance layers.
10. Risk scoring.
11. Production deployment automation.
12. Enterprise identity integration.
13. Secret-management platforms.
14. Extensive audit pipelines.
15. Tamper-evident event chains.
16. Long-retention operational history.
17. Complex rollback orchestration.
18. Safety-oriented command filtering.

These are deferred, not denied forever. They belong to a later separately authorized safety implementation.

No deferred safety feature may be smuggled into the current design in a way that expands the system or narrows power.

## 11. Minimum correctness is still required

Power that cannot be shown to work is not power. Therefore SMP includes the minimum correctness mechanisms required to distinguish a real working system from a claim.

The current minimum is:

1. Exact process exit status.
2. Direct error output.
3. Basic machine-readable state where lifecycle operations require it.
4. Deterministic identification of the microVM instance being controlled.
5. Tests that execute the promised privileged operations.
6. Positive confirmation that startup, connection, reboot, stop, and destruction occurred.
7. Version identification for the built SMP executable and selected external runtime dependencies.
8. A simple build result that can be reproduced on the declared host class.

This is verification, not a receipt architecture.

The current phase does not require signed receipts, append-only evidence, content-addressed proof bundles, durable workflow records, or a generalized audit subsystem.

## 12. No safety theater

SMP must not add mechanisms that look responsible but do not create meaningful protection or power.

Examples include:

1. Logging every command without a defined consumer or purpose.
2. Producing receipt objects that merely restate unverified claims.
3. Adding confirmations that can be bypassed by a different path.
4. Renaming restrictions as policy decisions without implementing a coherent authority model.
5. Adding configuration switches that are never tested.
6. Adding an allowlist while leaving equivalent unrestricted escape paths.
7. Claiming isolation from process naming or directory layout rather than the actual VM boundary.

During the power-first phase, an omitted safety framework is preferable to a decorative one.

## 13. Direct control law

SMP must expose the underlying power through the shortest practical control path.

For the Firecracker base this requires, at minimum:

1. A direct way to create a machine definition.
2. A direct way to start the microVM.
3. A direct way to reach an unrestricted root shell.
4. A direct way to execute an arbitrary command as guest root.
5. A direct way to observe serial output and failure.
6. A direct way to stop the microVM.
7. A direct way to destroy the machine and its writable state.

A controller may simplify these operations, but it must not replace Firecracker's expressive power with a closed catalog.

## 14. No silent restriction law

Any limit must be visible at the contract boundary.

SMP must not silently:

1. Remove Linux capabilities.
2. Apply a seccomp profile inside the guest.
3. Mount the guest root filesystem read-only when writable root is promised.
4. block package repositories.
5. filter outbound guest networking.
6. remove package-manager functions.
7. prevent service creation.
8. suppress kernel or system logs required to diagnose failure.
9. rewrite commands.
10. alter command arguments.
11. replace requested binaries.
12. cap CPU, memory, disk, process count, or file size without exposing the assigned machine configuration.
13. substitute a container for a microVM.
14. convert persistent mode into ephemeral mode.
15. convert direct root into sudo-mediated access.

Inherent Firecracker, Linux, hardware, kernel-build, and assigned-resource limits must be documented as platform boundaries rather than hidden.

## 15. No silent fallback law

When the full-power path fails, SMP must fail clearly.

It must not automatically fall back to:

1. A container.
2. A chroot.
3. A user namespace.
4. QEMU with a different device or execution model.
5. A local host shell.
6. A read-only guest.
7. A reduced kernel.
8. A network-disabled mode.
9. A non-root user.
10. A simulated response.

Alternative backends may be added later as separately named products, but they must never impersonate the Firecracker product.

## 16. Host and guest boundary

SMP's first power contract is intentionally asymmetric.

Inside the guest:

1. Root is unrestricted by SMP.
2. The guest controls its own operating system.
3. The guest may destroy or corrupt its own state.
4. The guest may reconfigure its own users, services, firewall, networking, mounts, filesystems, packages, and processes.
5. The guest may run arbitrary workloads supported by its kernel and virtual hardware.

At the host boundary:

1. The guest receives only explicitly assigned virtual hardware and connectivity.
2. Guest root is not represented as host root.
3. Host paths are not automatically mounted into the guest.
4. Host credentials are not automatically copied into the guest.
5. The Firecracker API socket remains host-side unless a later contract explicitly changes that boundary.
6. Host `/dev/kvm` is used by Firecracker and is not automatically passed through to the guest.

These statements define the product boundary. They are not an in-guest safety policy.

## 17. Resource truth

A machine's assigned resources must be explicit and real.

The machine definition must identify at least:

1. Machine ID.
2. Architecture.
3. vCPU count.
4. Memory size.
5. Kernel image.
6. Kernel boot arguments.
7. Root disk.
8. Additional disks.
9. Read-only or writable status for each disk.
10. Network interfaces.
11. Host TAP devices or equivalent direct bindings.
12. Guest connection method.
13. Writable state location.
14. Firecracker process identity or control socket.

SMP must not claim unlimited resources. Maximum power means unrestricted control of assigned resources, not fictional absence of physical limits.

## 18. State modes

The first product must distinguish state modes rather than mixing them.

Required modes are:

1. Persistent machine: writable guest state survives stop and subsequent start.
2. Disposable machine: writable guest state is unique to the instance and removed by explicit destruction.
3. Base image: source image used to create writable machine state and not mutated by ordinary instance operation.

A mode is full power when root has unrestricted authority over that mode's writable guest state. Ephemerality must not be used to conceal a read-only or restricted guest.

## 19. Operator supremacy

The operator is the authority for the current SMP phase.

SMP must not invent an internal authority that can overrule the operator inside the declared product scope.

The operator must be able to:

1. Select machine resources.
2. Select kernel and root image.
3. Set kernel boot arguments.
4. Attach supported disks and network interfaces.
5. Reach guest root.
6. Run arbitrary guest commands.
7. inspect guest output.
8. stop, reboot, reset, and destroy the guest.
9. retain or remove writable state according to the selected mode.
10. replace any SMP-generated image with a compatible operator-supplied image.

Defaults may exist. Defaults must not become ceilings.

## 20. Configuration law

Configuration must remain small, explicit, and complete.

A configuration field is justified only when it does one of the following:

1. Selects real power.
2. Selects an external resource.
3. Defines required lifecycle state.
4. Resolves a platform difference.
5. Makes an otherwise hidden limit explicit.

SMP must not create sprawling configuration for hypothetical future systems.

The default configuration should boot the canonical machine with no unnecessary choices, while the underlying controller remains capable of accepting the full selected Firecracker configuration envelope promised by the current product contract.

## 21. Dependency law

Dependencies are evaluated by total mechanism and retained power.

A dependency is acceptable when it is the smallest reliable way to obtain required power or correctness.

A dependency is rejected when:

1. It duplicates functionality already available directly.
2. It forces a reduced capability model.
3. It creates an unrelated service dependency.
4. It imports a large framework for a small operation.
5. It hides critical behavior behind conventions SMP cannot directly control.
6. It creates permanent coupling to another StealthEye project.

The project should prefer operating-system primitives, Firecracker's documented interface, small focused libraries, and direct file or socket operations over general platforms.

## 22. Implementation-language rule

Persistent SMP executable code should default to Rust because it can provide a small static executable, direct Linux integration, precise errors, and strong control over dependencies.

Shell may be used where it is genuinely the smallest complete mechanism for host preparation, image construction, or direct invocation of standard Linux tools.

Shell must not become an accidental orchestration framework. When lifecycle state, concurrency, structured configuration, or exact error handling exceeds a small script, that behavior belongs in the SMP executable.

Python, Node.js, and other runtimes are not default dependencies. They may be introduced only when they create concrete power that cannot be obtained as completely and more simply with the existing stack.

## 23. Failure semantics

Failure must be plain and exact.

Every operation must:

1. Return success only when its promised postcondition is true.
2. Return a nonzero or structured failure when the operation fails.
3. Preserve the underlying error information needed to diagnose the failure.
4. Avoid converting unknown state into success.
5. Avoid continuing into later destructive steps after a prerequisite fails.
6. Identify partially created resources when cleanup is required.

The current phase does not require a generalized durable transaction engine. It does require honest operation results.

## 24. Testing law

Tests exist to prove power, not to maximize test count.

Every promised capability must have at least one direct test that attempts the real operation.

Mocks do not certify god mode.

For the Firecracker base, certification requires a real KVM-capable Linux host and a real Firecracker microVM. Unit tests may support development, but the product is not complete until the real guest performs the privileged acceptance suite.

Tests must not pass by checking only command strings, configuration files, or expected logs. They must observe the resulting guest state or behavior.

## 25. Documentation law

Canonical documents must state:

1. What power is promised.
2. Where that power begins and ends.
3. What is implemented now.
4. What is absent.
5. What is deferred.
6. What is inherent to the external platform.
7. What would constitute watering down.
8. How completion is proven.

Documentation must not market a partial capability as complete.

## 26. Change law

Every material change must be evaluated in this order:

1. Does it reduce promised power?
2. Does it add a hidden restriction?
3. Does it introduce a silent fallback?
4. Does it import another repository's implementation without explicit authorization?
5. Does it create a new dependency or layer?
6. Does that layer create measurable power, necessary correctness, or essential operability?
7. Is there a smaller implementation with equal power?
8. Can the power be directly tested?

Any change that reduces promised power is rejected unless the owner explicitly changes the product contract.

Any change that adds mechanism without power, correctness, or essential operability is rejected.

## 27. Deferred safety implementation

After the power surface is complete and accepted, a separate safety phase may add strong isolation, policy, receipts, governance, multi-tenant controls, and production hardening.

That future phase must begin from the completed power contract. It must clearly distinguish:

1. Power retained.
2. Power governed.
3. Power requiring explicit authorization.
4. Power intentionally removed by an owner-approved contract change.

The future safety system must not retroactively redefine a restricted subset as the original SMP god-mode product.

## 28. Constitutional acceptance test

A proposed SMP component is constitutionally valid only when every answer below is yes:

1. Is its authority domain explicit?
2. Is its promised power complete inside that domain?
3. Is the implementation the smallest known complete mechanism?
4. Does it avoid arbitrary restrictions?
5. Does it avoid silent fallback?
6. Does it avoid unapproved inheritance from another StealthEye repository?
7. Does it avoid premature safety architecture?
8. Does it contain only the minimum correctness required to prove it works?
9. Can the real power be directly tested?
10. Does it preserve every previously completed power contract?

If any answer is no, the component is not complete.

## 29. Priority and amendment

This constitution controls all SMP design and implementation unless the owner explicitly amends it.

When another document conflicts with this constitution, this constitution wins unless the conflicting document is itself an explicit constitutional amendment.

Convenience, convention, framework defaults, security theater, inherited architecture, and implementation momentum do not override this document.
