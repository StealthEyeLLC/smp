# SMP private plugin

Register `plugin.json` only after the dedicated SMP tunnel is healthy. Replace the endpoint and authentication placeholders with SMP-owned values. The manifest declares one namespace, `smp`, and one tool, `go`; its callable identity is `smp.go`.

Before registration, verify `GET /healthz`, then `GET /readyz`. After registration, open a fresh ChatGPT tab, call `smp.go` with `operation: "describe"`, and verify that the returned operation catalog and component identities match the installed handoff commit.
