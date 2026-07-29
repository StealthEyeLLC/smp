# Least-Theater and Full-Standalone Law

Status: Constitutional amendment

Project: SMP — Smallest Maximum Power

Effective: 2026-07-29

Owner directives:

**Use the least amount of theater possible, always.**

**SMP is standalone during build, installation, testing, operation, recovery, and removal.**

This amendment overrides conflicting preflight, verification, evidence, prompt, bootstrap, build-operator, and process language elsewhere in the SMP documents.

## 1. Execution first

When implementation is authorized, begin after the smallest source-integrity check needed to avoid editing the wrong repository or commit.

The normal initial check is limited to:

1. correct repository;
2. correct source commit or branch;
3. no conflicting uncommitted work;
4. usable repository write path.

Then implement.

Do not expand this into an environmental audit, readiness ceremony, permission inventory, architecture review, repeated planning pass, or search for unrelated execution systems.

## 2. Full standalone means full standalone

Unless the owner explicitly authorizes a named exception, SMP must not use another private StealthEye system during:

- source implementation;
- build execution;
- installation;
- host preparation;
- Firecracker launch;
- testing;
- plugin setup;
- tunnel setup;
- operation;
- recovery;
- upgrade;
- cleanup;
- removal.

This prohibition includes:

- Baby;
- Fix;
- StealthEye Horsey;
- Quirt;
- any private StealthEye operator, broker, workspace system, scheduler, job system, deployment system, or execution proxy.

Do not invoke them.

Do not inspect them for workspaces, profiles, shell access, host identity, KVM access, or deployment capability.

Do not substitute one when another is unavailable.

The GitHub App may manage the SMP repository because repository management is not SMP runtime execution. It must not become SMP's build runner, host executor, or runtime dependency.

## 3. Honest bootstrap boundary

Before SMP exists on a host, some first host action is inherently required to install it.

That bootstrap must be:

- SMP-owned;
- minimal;
- explicit;
- reproducible from this repository;
- independent of Baby, Fix, Horsey, or another private StealthEye control plane.

The repository must provide one direct bootstrap entrypoint, preferably a single command or small installer, that installs the first `smp` binary and its own connection.

After that first action, all further SMP build, installation, testing, upgrade, operation, and recovery must use SMP itself or ordinary direct operating-system facilities owned by SMP.

If the current ChatGPT tab has no direct way to perform the first host action, it must not search for or invoke another StealthEye operator. It must complete every repository-side prerequisite and stop at the exact one-action bootstrap boundary.

Tool absence is not permission to use Horsey, Baby, Fix, or another substitute.

## 4. Established infrastructure is accepted

Owner-stated, repeatedly used infrastructure facts are accepted unless current evidence directly contradicts them.

For SMP:

- the authorized VPS exists;
- KVM works;
- Firecracker has worked on that VPS;
- the connected GitHub App includes `StealthEyeLLC/smp`.

Do not block source implementation to re-prove those facts.

Do not demand a KVM probe before writing code.

Verify KVM and Firecracker only when the SMP-owned implementation reaches the point where it can actually launch and test them.

## 5. Verify only at the point of use

Verification is performed only when its result changes the next action.

Examples:

- verify a downloaded digest before executing the asset;
- verify a process identity before signaling it;
- verify a test result before claiming the feature works;
- verify the final remote commit after pushing it.

Do not verify a future requirement before the implementation that makes it testable exists.

Do not perform end-state certification as a prerequisite to starting.

## 6. Tool-bound truth

A prompt must not require the current tab to prove something its available tools cannot observe.

When the current tab cannot perform a required action:

1. complete all work it can perform directly;
2. identify the exact missing action;
3. stop at that boundary;
4. do not route through an unauthorized substitute;
5. do not claim the action occurred.

Do not spend turns searching for imaginary tools, alternate permission paths, Horsey, Baby, Fix, or another operator.

## 7. Minimum evidence

Evidence exists to preserve useful truth, not to manufacture ceremony.

Keep only evidence needed to answer:

- what was built;
- which commit contains it;
- whether the relevant tests passed;
- what remains unverified;
- whether standalone independence was preserved.

Do not create large evidence taxonomies, duplicate proof bundles, exhaustive inventories, or positive-absence rituals unless a real ambiguity or destructive operation requires them.

Normal successful cleanup needs a direct status check, not a separate evidence system.

## 8. Prompt split

Prompt 1 implements SMP in the repository and prepares the SMP-owned bootstrap, tunnel configuration, plugin definition, and installation package.

Prompt 1 must not invoke Horsey, Baby, Fix, or another private StealthEye execution system.

When the current tab cannot perform the first direct host bootstrap, Prompt 1 ends with the exact single bootstrap action and a complete handoff.

Prompt 2 begins after that direct SMP bootstrap and uses the actual SMP connection and `smp.go` to install, test, correct, and certify SMP end to end.

Fresh-tab plugin tests belong to Prompt 2.

## 9. Progress law

Do not narrate routine checks at length.

Do not repeatedly announce that work is being inspected, assessed, retrieved, or considered.

Use short progress updates only when:

- a material checkpoint completed;
- a real blocker appeared;
- user input is genuinely required.

Inspection without resulting action is not progress.

## 10. Blocker law

A blocker must be current, concrete, and causally necessary.

The following are not blockers:

- inability to perform a redundant verification;
- inability to prove a feature that has not been implemented yet;
- uncertainty that can be resolved while implementing;
- a theoretical future platform limitation.

A missing direct bootstrap path is a real boundary only after all repository-side bootstrap prerequisites are complete.

When a real blocker exists, state it once in one sentence and continue every unaffected part of the mission.

## 11. Acceptance timing

Build-time checks happen during implementation.

Real capability tests happen after the corresponding capability exists and after SMP has been directly bootstrapped.

Fresh-tab `smp.go` tests happen in Prompt 2.

Do not front-load final certification into Prompt 1.

## 12. Constitutional test

For every proposed check, tool, service, operator, evidence item, or prompt section, ask:

> Does this directly enable SMP itself, prevent a concrete destructive mistake, or prove a capability that now exists without introducing another private control plane?

If not, omit it.

The fully standalone, least-theater path is the canonical path.
