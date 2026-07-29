# Dedicated SMP tunnel

SMP uses a dedicated token-managed Cloudflare Tunnel. The canonical local origin is `http://127.0.0.1:7745`; the public Internet must not receive an anonymous SMP control port.

Create a separate tunnel and hostname in Cloudflare Zero Trust, route only that hostname to the local SMP origin, and protect it with the intended private access policy. Copy the dedicated tunnel token through the interactive `scripts/bootstrap.sh` prompt or write it from a protected local source to:

```text
/etc/smp/credentials/tunnel-token
```

Required ownership and mode are `root:root` and `0600`. The token is read through cloudflared's `--token-file` option, not a command argument. `smp-tunnel.service` has no dependency on another product's tunnel, credential, socket, configuration, or service.

Local checks after bootstrap:

```bash
curl --fail http://127.0.0.1:7745/healthz
curl --fail http://127.0.0.1:7745/readyz
systemctl is-active smp.service
systemctl is-active smp-tunnel.service
```

Stopping the tunnel does not stop SMP or any Firecracker process. Restarting `smp.service` does not stop persistent microVMs because the service owns only `smp serve`.
