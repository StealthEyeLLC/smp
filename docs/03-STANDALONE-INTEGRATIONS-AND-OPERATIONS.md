# SMP Standalone Integrations and Operations

Status: Canonical integration and operations specification

Project: SMP — Smallest Maximum Power

Reviewed: 2026-07-29

## 1. Purpose

This document defines how SMP connects to ChatGPT, GitHub, and the authorized VPS while remaining a standalone product.

Standalone means SMP owns its runtime implementation, process, state, credentials, endpoint, machine lifecycle, and Firecracker control. It does not mean SMP must avoid ordinary external development tools or share no physical host with another service.

The canonical remote product is a private ChatGPT plugin named `SMP` backed by an SMP-owned MCP server and a dedicated VPS connection.

## 2. Canonical identities

The following names are binding:

```text
repository:              StealthEyeLLC/smp
local executable:        smp
ChatGPT plugin name:     SMP
MCP server/app ID:       smp
only MCP tool:           go
callable identity:       smp.go
local service:           smp.service
optional tunnel service: smp-tunnel.service
```

The ChatGPT-facing name is `SMP`. The MCP namespace is `smp`. The only callable tool is `go`, producing the canonical callable identity `smp.go`.

No second callable SMP tool may be added for lifecycle, files, execution, networking, images, snapshots, status, recovery, or raw access. Every capability is expressed through the operation envelope of `smp.go`.

## 3. Canonical topology

The preferred remote topology is:

```text
ChatGPT
  -> private plugin named SMP
  -> one callable tool: smp.go
  -> SMP-only secure MCP connection
  -> smp serve on the authorized VPS
  -> shared SMP core
  -> Firecracker
```

Local operation remains:

```text
operator -> smp CLI -> Firecracker
```

The remote path is optional. Failure or removal of the ChatGPT connection must not prevent local SMP operation or stop already running persistent microVMs.

## 4. Dedicated SMP VPS connection

SMP receives its own remote connection even when Baby and SMP run on the same physical VPS.

The SMP connection must have its own:

- ChatGPT plugin registration;
- MCP server identity;
- tunnel or endpoint identity;
- transport credential;
- local listener or Unix socket;
- systemd service;
- configuration;
- state directories;
- logs;
- health and readiness result;
- credential rotation and removal path.

SMP must not reuse a Baby endpoint, socket, tunnel identity, token, service unit, database, state tree, job authority, receipt authority, artifact authority, workspace, or recovery controller.

Sharing the Linux host, KVM device, ordinary system packages, Git, GitHub, and public platform dependencies does not make SMP a Baby dependency.

## 5. Same useful attributes, independent implementation

SMP independently implements the operational attributes that make a one-tool connection effective:

- one stable callable tool;
- live `describe` capability discovery;
- exact argument arrays;
- strict request and response schema versions;
- deterministic request digests;
- idempotent network retry identity;
- no duplicate operation after a client timeout;
- detached operation continuation after disconnect;
- verified process adoption after `smp serve` restart;
- exact exit codes and failure classes;
- bounded inline output;
- chunked continuation for retained output;
- explicit output-capture exhaustion;
- file upload and download;
- status, wait, read, and cancel through the same tool;
- current host and process truth instead of stale state claims;
- raw SMP and Firecracker API escape paths.

These are SMP requirements, not imported Baby architecture. SMP may use general engineering knowledge, but no private Baby source, schema, service, or implementation fragment is copied without explicit authorization.

## 6. ChatGPT plugin contract

### 6.1 Plugin package

The private plugin is named exactly `SMP`.

Its underlying custom app connects to the SMP MCP server. The initial plugin should contain no additional callable action tools and no second app that duplicates the SMP control surface.

Non-callable presentation metadata, documentation, icons, and future user-interface resources do not violate the one-tool rule. A callable operation exposed separately from `smp.go` does violate it.

### 6.2 Stable published tool schema

ChatGPT may retain a reviewed snapshot of an MCP app's available tools and input schema. Therefore the published `go` tool schema must remain intentionally broad, strict, and stable.

New SMP capabilities normally appear through:

1. a new runtime operation name;
2. its schema in `describe`;
3. compatible use of the existing `options` envelope;
4. the unchanged callable identity `smp.go`.

A breaking change to the outer MCP tool schema, authentication model, or transport requires a new plugin/app review or republish where the platform requires it. SMP must not assume a live server schema change automatically updates an already approved ChatGPT installation.

### 6.3 Workspace and platform truth

Workspace administrators control whether the private SMP plugin is enabled and who may use it. Platform-required write confirmations may still appear. SMP must not claim it can suppress ChatGPT workspace policy or confirmation behavior.

### 6.4 Capability discovery

The first remote call after connection, upgrade, or recovery is `describe`.

`describe` must return at least:

- SMP version and build identity;
- request and response schema versions;
- complete operation catalog and argument schemas;
- Firecracker version and digest;
- guest image and kernel identities;
- host architecture and certification state;
- inline and captured output limits;
- result retention limits;
- timeout limits;
- transport mode;
- server instance identity;
- current machine summaries when requested.

An incompatible schema or server version fails clearly. It must not silently downgrade to a reduced command set.

## 7. Secure reachability

The preferred connection uses an SMP-only supported secure MCP tunnel so the MCP server does not require an open public inbound port.

When a tunnel is used:

```text
ChatGPT -> OpenAI tunnel endpoint -> SMP tunnel client -> local smp serve endpoint
```

The tunnel client is an external transport dependency, not a second SMP control plane. It may run as `smp-tunnel.service` and must have a separate SMP-only tunnel identity and credential.

The local `smp serve` endpoint should bind only to a Unix socket or loopback address when used behind the tunnel.

If direct remote exposure is deliberately selected instead, it must use authenticated encrypted transport and an explicitly configured listener. Anonymous public SMP control is invalid.

## 8. VPS process and filesystem layout

The canonical installation layout is:

```text
/usr/local/bin/smp            immutable active executable
/etc/smp/                     non-secret configuration
/etc/smp/credentials/         SMP-only credentials, root-readable only
/var/lib/smp/                 authoritative SMP state
/var/lib/smp/machines/        machine definitions and writable state
/var/lib/smp/assets/          verified Firecracker, kernel, and image assets
/var/lib/smp/requests/        minimal retry-identity records
/var/lib/smp/results/         bounded-lifetime result output
/run/smp/                     runtime sockets, locks, and process identity
```

Operational logs may use journald or bounded SMP-owned files. Secrets must not appear in process arguments, repository contents, guest seed disks, base images, machine records, ordinary logs, or `describe` output.

`/var/lib/smp` must not be nested beneath a Baby state directory. Baby must not own SMP's files, and SMP must not own Baby's files.

## 9. Service model

The local CLI remains daemonless for ordinary commands. Remote availability uses systemd to supervise `smp serve`; the SMP binary must not implement an unnecessary self-daemonization layer.

The service may run with the host authority required to manage KVM, TAP devices, nftables, Firecracker processes, and SMP-owned files. Non-interactive service operation must never wait for a sudo password prompt.

The remote service must provide local health and readiness truth without adding another ChatGPT tool. Health means the serving process and local state are usable. Readiness additionally means the MCP endpoint can answer `describe` and its configured transport is available.

Restarting `smp.service` must not stop persistent microVMs. After restart it must reconcile machine state, adopt verified detached operations, and refuse ambiguous process identity.

Stopping or restarting `smp-tunnel.service` must affect only ChatGPT reachability. It must not stop SMP, mutate machines, or destroy retained results.

## 10. Credential separation

The following credential classes are separate:

1. ChatGPT or secure-tunnel credential;
2. direct SMP endpoint credential, when direct exposure is used;
3. host installation or administration credential;
4. guest root SSH client key;
5. GitHub App installation authority;
6. optional guest-owned repository credentials deliberately installed by the operator.

No Baby credential authorizes SMP. No SMP credential authorizes Baby.

The host's GitHub App credential must not be copied into a guest, seed disk, base image, machine record, result file, or plugin request. Guest repository access, when desired, uses a separately declared guest credential.

Rotation of an SMP transport credential must not require rebuilding Firecracker images or changing guest SSH identity. Removing the SMP plugin or tunnel must not remove local machine state.

## 11. GitHub App integration

### 11.1 Current verified state

On 2026-07-29, the existing GitHub App installation for `StealthEyeLLC` was verified to include `StealthEyeLLC/smp` and expose repository capabilities reported as:

```text
admin
maintain
push
pull
triage
```

The repository's default branch is `main`.

This is an observed integration state, not a permanent constitutional guarantee. Implementation and release work must re-check repository access before relying on it.

### 11.2 Role of the GitHub App

The GitHub App may be used for authorized repository operations such as:

- reading the repository;
- creating and updating branches;
- writing commits;
- opening or reviewing pull requests;
- reading commit and tree identities;
- verifying remote checkpoints;
- performing other actions allowed by its current installation permissions.

The GitHub App is a development and repository-management integration. It is not part of the SMP runtime path and is not required for `smp up`, machine operation, `smp serve`, or `smp.go`.

Revoking GitHub App access may prevent future repository operations, but it must not stop an installed SMP service or running microVM.

### 11.3 Repository scope and token handling

The installation is configured for repository access that currently includes SMP. An all-repositories installation normally includes repositories subsequently created under the installed account, but actual access and permissions must be verified rather than inferred before each implementation mission.

No GitHub App private key, installation token, or user token may be committed to this repository.

If SMP-related automation ever mints a GitHub App installation token, it must use a short-lived token, request no broader repository or permission scope than needed for that operation when narrowing is available, avoid logging the token, and discard it after use.

SMP itself must not require a resident GitHub credential on the VPS unless a later explicitly authorized feature creates real runtime power that requires it.

## 12. Installation, upgrade, and removal correctness

The implementation must provide a small exact installation path for the SMP binary, directories, configuration, and optional services.

An upgrade must:

1. identify the current and candidate SMP versions and digests;
2. install the candidate binary atomically;
3. preserve machine state and credentials;
4. restart only the SMP remote service when required;
5. verify health, readiness, and `describe`;
6. restore the previous binary if the new service cannot become ready;
7. never restart or mutate running Firecracker machines merely because the control service was upgraded.

This is bounded service rollback, not a generalized deployment system.

Removal must distinguish:

- disconnecting the ChatGPT plugin;
- stopping or deleting the tunnel;
- disabling `smp serve`;
- uninstalling the executable;
- retaining machine state;
- explicitly destroying machine state.

No uninstall command may silently destroy persistent microVM disks.

## 13. Independence acceptance

SMP is operationally standalone only when all of the following are proven:

1. ChatGPT displays the private plugin as `SMP`;
2. the plugin exposes exactly one callable tool, `smp.go`;
3. `describe` reports the live SMP catalog and identities;
4. the connection uses an SMP-only endpoint or tunnel identity;
5. Baby and SMP have different credentials, services, sockets, state roots, and logs;
6. stopping Baby does not stop SMP or its microVMs;
7. removing Baby's connection does not remove `smp.go`;
8. stopping the SMP tunnel removes remote reachability without stopping local SMP or its microVMs;
9. restarting `smp.service` preserves persistent microVMs and adopts verifiable detached operations;
10. revoking GitHub App access does not affect installed SMP runtime behavior;
11. local `sudo smp up` works without ChatGPT, GitHub, or Baby;
12. GitHub App access to `StealthEyeLLC/smp` can be independently verified before repository mutation;
13. no Baby implementation or credential is present in the SMP repository or runtime state;
14. no GitHub credential is present in the canonical guest, seed, or base image;
15. a plugin schema upgrade is handled honestly rather than assumed to propagate automatically.

## 14. Deferred systems

The following remain deferred and are not implied by the dedicated connection:

- a second SMP control-plane repository;
- a generalized deployment platform;
- a durable workflow or job system;
- a database;
- an artifact authority;
- a receipt or evidence authority;
- a policy engine;
- multi-tenant authorization;
- a web dashboard;
- automatic production activation;
- a runtime dependency on GitHub;
- a runtime dependency on Baby.

## 15. Completion condition

This integration specification is complete when the implementation proves:

- the plugin is named `SMP`;
- `smp.go` is its only callable tool;
- the dedicated SMP connection works on the authorized VPS;
- the local CLI remains fully usable without the connection;
- service, tunnel, credentials, state, and logs are independent from Baby;
- the existing GitHub App can access and update `StealthEyeLLC/smp` when authorized;
- GitHub access is not required by the installed runtime;
- service restart, tunnel loss, client retry, and long operations do not create duplicate work or false success;
- no missing integration dependency is hidden behind manual tribal knowledge.
