# Register the private SMP app

The repository definition is complete, but a private custom MCP connection cannot be registered through the GitHub App.

After direct bootstrap creates the dedicated tunnel and its private hostname:

1. Replace `REPLACE_WITH_DEDICATED_SMP_TUNNEL_HOSTNAME` in the installed copy of `/etc/smp/SMP.plugin.json` with that dedicated hostname. Do not commit the hostname if the workspace treats it as private.
2. Confirm the hostname routes only to `http://127.0.0.1:7745` through `smp-tunnel.service` and that Cloudflare Access requires the intended private workspace identity.
3. In the ChatGPT workspace, add a custom MCP app named exactly `SMP` using `https://<dedicated-smp-hostname>/mcp`.
4. Confirm its server namespace is `smp` and that discovery returns exactly one tool named `go`.
5. Start Prompt 2 in a fresh tab and call `smp.go` with `operation: "describe"` before any mutation.

The expected callable identity is `smp.go`. Do not add lifecycle, file, log, result, or raw-access tools separately.
