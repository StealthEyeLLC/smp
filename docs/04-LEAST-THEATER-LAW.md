# Least-Theater Execution Law

Status: Constitutional amendment

Project: SMP — Smallest Maximum Power

Effective: 2026-07-29

Owner directive: **Use the least amount of theater possible, always.**

This amendment overrides any conflicting preflight, verification, evidence, prompt, or process language elsewhere in the SMP documents.

## 1. Execution first

When implementation is authorized, begin implementation after the smallest source-integrity check needed to avoid editing the wrong repository or commit.

The normal pre-edit check is limited to:

1. correct repository;
2. correct source commit or branch;
3. no conflicting uncommitted work;
4. usable write path.

Then build.

Do not expand this into an environmental audit, readiness ceremony, permission inventory, architecture review, or repeated planning pass.

## 2. Established infrastructure is accepted

Owner-stated, repeatedly used infrastructure facts are accepted as established unless current evidence directly contradicts them.

For the current SMP implementation:

- the authorized VPS exists;
- KVM works;
- Firecracker has worked on that VPS;
- the connected GitHub App includes `StealthEyeLLC/smp`;
- the existing authorized durable execution path may be used to build SMP.

Do not block implementation to re-prove those facts.

Do not demand a KVM probe before writing code.

Do not search for a special host-shell or KVM-probe tool when the authorized durable execution interface can already run the required build commands.

## 3. Verify only at the point of use

Verification is performed only when its result changes the next action.

Examples:

- verify a downloaded digest before executing the asset;
- verify a process identity before signaling it;
- verify a test result before claiming the feature works;
- verify the final remote commit after pushing it.

Do not verify a future requirement before the implementation that makes the requirement testable exists.

Do not perform end-state certification as a prerequisite to starting.

## 4. Tool-bound truth

A prompt must not require the current tab to prove something its available tools cannot observe.

When a tool lacks direct access to a fact that the owner has already established:

1. accept the established fact;
2. continue all executable work;
3. test it later from the tool or tab that can actually observe it.

Tool absence is not evidence that the infrastructure is absent.

Do not spend turns searching for imaginary tools or alternate permission paths.

## 5. Build operator rule

The existing authorized durable execution surface may be used as a temporary build operator.

Use its workspace and generic execution operations directly. Create a workspace when needed. Use the registered shell execution profile when available.

Do not confuse “SMP must be standalone at runtime” with “SMP may not be built using an existing authorized execution tool.”

Baby, Fix, or another operator may transport commands during construction without becoming an SMP runtime dependency.

## 6. Minimum evidence

Evidence exists to preserve useful truth, not to manufacture ceremony.

Keep only evidence needed to answer:

1. what was built;
2. which commit contains it;
3. whether the relevant tests passed;
4. what remains unverified;
5. whether runtime independence was preserved.

Do not create large evidence taxonomies, duplicate proof bundles, exhaustive inventories, or positive-absence rituals unless a real ambiguity or destructive operation requires them.

Normal successful cleanup needs a direct status check, not a separate evidence system.

## 7. Prompt split

Prompt 1 implements SMP and prepares the dedicated tunnel and plugin registration. It does not require fresh-tab invocation of a newly added plugin.

Prompt 2 runs in a fresh tab and verifies the actual `SMP` plugin and `smp.go` end to end.

Prompt 1 must not delay implementation for KVM re-verification, host-lane certification, plugin invocation, or final acceptance work that belongs after the implementation exists.

## 8. Progress law

Do not narrate routine checks at length.

Do not repeatedly announce that work is being inspected, assessed, retrieved, or considered.

Use short progress updates only when:

- a material checkpoint completed;
- a real blocker appeared;
- user input is genuinely required.

Inspection without resulting action is not progress.

## 9. Blocker law

A blocker must be current, concrete, and causally necessary.

The following are not blockers:

- inability to perform a redundant verification;
- lack of a specially named host-shell tool when generic execution exists;
- inability to prove a feature that has not been implemented yet;
- uncertainty that can be resolved while implementing;
- a theoretical future platform limitation.

When a real blocker exists, state it once in one sentence and continue every unaffected part of the mission.

## 10. Acceptance timing

Build-time checks happen during implementation.

Real capability tests happen after the corresponding capability exists.

Fresh-tab plugin tests happen in Prompt 2.

Do not front-load final certification into Prompt 1.

## 11. Constitutional test

For every proposed check, document, service, evidence item, or prompt section, ask:

> Does this directly enable implementation, prevent a concrete destructive mistake, or prove a capability that now exists?

If not, omit it.

The least-theater path is the canonical path.