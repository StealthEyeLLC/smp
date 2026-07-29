# SMP Prompt-1 Final Recovery Evidence

## Failed detached recovery

- Commit: `16c66c8da66d9bb874f33f9576c2294f97148c2f`
- Tree: `91a8adac215a31e57ac79cd6eab9341f6501bc18`
- Started: `2026-07-29T12:26:52Z`
- Completed: `2026-07-29T12:26:58Z`
- Result: `FAIL`
- Exit status: `1`
- Durable failure archive: `/var/lib/smp/results/archive/final-recovery-20260729T122652Z`
- Exact cause: `cargo fmt --all -- --check` reported canonical Rust formatting differences.
- Scope: failure occurred in the repository gate before binary installation, certification-machine reset, final Firecracker acceptance, or canonical asset replacement.

## Repository corrections

- Canonical formatting commit: `f99c9cebccd4e00b3f463159ca72bb3d75f6acf3`
- Canonical formatting tree: `0e671b57152b52fb1df8acbc190f3d46933b8e65`
- Pinned Rust lint correction: `aa47bd8806f205e9dea366e170e768ec3395da69`
- Pinned Rust lint tree: `753b6504eef54973c5c19c12b8416d33baa055c5`
- Network lifecycle and final recovery correction: `e91ec5dd14988d46e90a40bbb790e8c815173e7f`
- Corrected implementation tree: `4aee46f59dbc16fb4422794e8bd7f5017501d8a8`

The correction preserves the formatting gate, applies canonical Rustfmt output to the complete Rust repository, resolves the Clippy failures that were hidden behind the formatting failure, expands behavioral network-plan coverage, makes failed-launch and failed-readiness cleanup explicit, verifies positive network absence during real acceptance, archives the prior detached status, and verifies retained canonical asset digests without rebuilding them.

## Clean temporary repository gate

Source:

- Commit-equivalent corrected implementation tree: `4aee46f59dbc16fb4422794e8bd7f5017501d8a8`
- Rust toolchain: `1.97.1`
- Rustfmt: `PASS`
- Clippy, all targets with warnings denied: `PASS`
- Rust tests: `34 passed; 0 failed`
- Cargo metadata with generated locked dependency graph: `PASS`
- Bash syntax checks: `PASS`
- ShellCheck `0.11.0`, error severity: `PASS`
- Plugin metadata and single-tool assertions: `PASS`
- Network lifecycle and audit-helper assertions: `PASS`
- Standalone dependency assertions: `PASS`
- Tracked source remained clean after the gate: `PASS`

## Remaining host truth

Real Firecracker acceptance has not yet been rerun for these corrections. Prompt 1 remains uncertified until the detached recovery records all of:

- `SMP real Firecracker acceptance passed`
- `SMP targeted recovery complete`
- `acceptance_result=PASS`
- `final_exit_status=0`
- `"result": "PASS"`
- `"exitStatus": 0`

The final checkpoint report supplies the single immutable recovery command after the evidence-containing commit is created and remotely verified. Firecracker `1.15.1`, Linux `6.1.177`, its configuration and module tree, and Debian `13.6` are reused when their observed canonical digests match.

No Horsey, Baby, Fix, Quirt, or private StealthEye execution system was used.
