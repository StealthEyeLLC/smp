# SMP v1 build result

- Canonical source commit: `6b8eb5c11adf131606a58122d7ebf7933a8fc7c0`
- Canonical source tree: `82fc8ca80449c7e5643bb941319fd33f560ce7eb`
- Branch: `smp-v1`
- Stable implementation commit: `3e304549f0eaf69b6562ce326320ff3a9348febc`
- Stable implementation tree: `b9bad10fed897322887f39b5ba0ddc650181d54f`
- Rust toolchain: `1.88.0`

## Checkpoints

- `95e66cdf00d5b854fab5233809e688e2a407095e` / `e373a3ee8413e8e07a3a83bbbe8a2dabc99aa4e2`
- `70858567a2d4cf68aef98a765bf4d08e5e98d36d` / `626c95a73eee629a700f2d3a48c7ccaf5828304f`
- `d4381bcc47bd0b9fd0741169b117aafff4a2c2ea` / `9ed8b37bb7280332fcf87a6f79406d2bdfd66283`
- `55779cf942935d97d59cd3d569ca0ff7d43ab732` / `2346645b4b3c2176c2a971c614dcde4e7f3afbcc`
- `bb5c97ed856a1a1f4f94bc7ed303cd41957e04a1` / `f4769b1fb12322001b1059e44e520b0345bfc188`
- `eb6c4918b3379575e7c282894c1a6675d9f463bc` / `fdaed4e1385d23a9dc4d92cb7d19c92ccd7624e8`
- `3e304549f0eaf69b6562ce326320ff3a9348febc` / `b9bad10fed897322887f39b5ba0ddc650181d54f`

## Assets and result

- Release binary SHA-256: `a8cb63526d7011af7c03b1c8ff6d14aaf636ce7f5116289ab99ad1406d6c987f`
- Firecracker: `1.15.1`; archive SHA-256 `d4a32ab2322d887ca1bc4a4e7afa9cc35393e6362dfc2b3becb389d362e4275a`; binary SHA-256 `7e8b57e88c459396d4680d83dcdd8c7f72305447cb55b11f4ac98ad70a3f7825`.
- Kernel: `6.1.178`; source SHA-256 `7d83fa67ca75032b1ac6ef49973722073963c0cb9bc3aa7ef3efa749cf6c720f`; config SHA-256 `417ab9e234342a06edc88341f9eff90bb7db1fdf4610c56d816b0c41878e511c`; vmlinux SHA-256 `962f2a873c9c1fcc5a00b5b446f80781311b9019d0ad65ff6d99fa94d0f3d28b`; module-tree SHA-256 `0114e9fe5561b2f4ef0e3466cddb9e890cad2ca264c8434eaaf0d4a058880586`.
- Rootfs: blocked. The mandated signed snapshot `20260711T000000Z` installs `/etc/debian_version` as `13.5`, not required `13.6`; no rootfs or asset manifest was represented as complete.
- Build result: `BLOCKED` on the exact Debian version/snapshot incompatibility.
- Baby2 executor: `baby-quirt` `0.1.0`, commit `b3a7119fb9321d74fee9a730517f519ed0d351c4`, tree `9ebab95f7f136c8480495f812796ac0bdc558a21`, host `vps-c9f04f5e`.
- Baby2 is a build executor only and is not an SMP runtime dependency.
- All installed-host replacement, service, plugin, tunnel, and full host acceptance work remains reserved for Prompt 2 after the source incompatibility is resolved.
