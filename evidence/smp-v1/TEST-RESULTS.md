# SMP v1 test results

- Canonical command: `SMP_TEST_WORKSPACE=/var/lib/baby-quirt/workspaces/smp-v1-total-rebuild/isolated/repository-gate-v4 bash scripts/test-repository.sh`
- Formatting: passed.
- Clippy with warnings denied: passed.
- Rust tests: passed, 34 of 34.
- Shell syntax: passed.
- ShellCheck: passed.
- JSON and plugin schema validation: passed; exactly one tool, `smp.go`.
- Isolated installer: passed, including repeat install and simulated readiness rollback.
- Replacement safety: passed, including preservation, explicit destructive cleanup, and ambiguous-process refusal.
- Firecracker smoke: not executed because canonical rootfs construction failed first. The dependent isolated-netns smoke exited nonzero without launching Firecracker.
- Remaining Prompt-2 tests: real installation/replacement, systemd health/readiness, tunnel/plugin activation, PCI and MMIO lifecycle, networking, persistence, reboot, exact argv, file transfer, raw API, and removal.

Exact blocker: the verified Debian snapshot required by the canonical documents reports `/etc/debian_version` `13.5`, while the immutable requested artifact version is `13.6`.
