# ADR 22: Security Hardening, IAM Enforcement & EU CRA Alignment

## Status

Accepted

## Context

A comprehensive security review against OWASP ASVS/Top 10, NIST SP 800-53, and the EU Cyber Resilience Act (CRA, Regulation (EU) 2024/2847) identified systemic and component-specific hardening requirements across the `dtnpm` assembler, its gRPC CLI utilities (`dtnbib`, `dtnfiles`, `dtnforward`, `dtnping`, `dtnprint`, `dtnsend`, `dtntrigger`, `hdy-stats`), systemd units, and CI/CD pipelines.

Specifically:
1. Binding an internal loopback `GrpcServer` via an ephemeral bind/drop sequence created a TCP TOCTOU race and allowed unauthenticated local processes to register CLAs/services directly against `hardy-proto` without traversing `GrpcSecurityGateway`.
2. CLI tools accepted Bearer tokens and passwords via process arguments without file/stdin alternatives, `dtntrigger` spawned unbounded blocking shell subprocesses without timeouts, `dtnfiles` buffered arbitrarily large streams before checking limits, `dtnbib` used unchecked slice arithmetic and prefix-matching transit checks, `hdy-stats` could reply to arbitrary third-party `report_to` EIDs, `dtnping` tracked state maps unboundedly, `dtnquery` computed bundle list digests using SHA-1, and `dtnpm.service` / `dtn-hdy-ham.service` defaulted to `User=root`.
3. GitHub Actions workflows used floating tags instead of immutable 40-character commit SHAs and lacked least-privilege `permissions: contents: read`.

At the same time, amateur radio testing and controlled lab deployments require explicit opt-in mechanisms for permissive behaviors (such as third-party `report_to` replies or disabling gRPC auth).

## Decision

1. **Zero-TOCTOU Internal gRPC & Registration Gate (`DTNPM-01`)**: `HamServer` binds `GrpcServer::new` directly to `127.0.0.1:0`, retrieves the live kernel-assigned address via `GrpcServer::local_addr()`, wraps `ServiceTracker` inside `GuardedBpaRegistration` backed by `GrpcRegistrationGate`, and starts `GrpcSecurityGateway::start_with_gate` whenever authentication or UDS protection is active.
2. **CLI Credential Hygiene & Hardening (`DTNPM-02`, `CLI-01`, `XC-05`)**:
   - Added `--token-file` (`resolve_bearer_token`) across `dtnpm` and all 8 client CLIs, plus `--password-stdin` (with deprecation warning on `--password`) for `dtnpm user add`/`passwd`.
   - Wizard-written config files enforce `0600` Unix permissions.
   - `dtnbib` uses `checked_add` + `.get(...)` bounds checking and exact `NodeId` authority equality for `--no-transit`.
   - `dtntrigger` bounds concurrent executions with an 8-permit Tokio semaphore and 60s `kill_on_drop` timeout; example scripts pass `--` before positional operands.
   - `dtnfiles` enforces `--max-size-mb` (default `100`) pre-buffering, sanitizes filenames to `[A-Za-z0-9_-]`, and performs async `tokio::fs::write`.
   - `hdy-stats` validates `--journald-unit` against `[A-Za-z0-9_.@-]+` (no leading `-`) and restricts `report_to` to the sender's `NodeId` authority unless `--allow-third-party-report-to` is explicitly enabled.
   - `dtnping` bounds ping history tracking maps at `10_000` entries; `dtnquery --digest` uses SHA-256 (`sha2`).
3. **Deployment, Supply Chain & CRA Governance (`CRA-01`, `CRA-02`, `XC-01`, `XC-03`)**:
   - Hardened `dtnpm.service` and `dtn-hdy-ham.service` to run as `User=dtnpm` with `ProtectSystem=strict`, `ProtectHome=true`, and `NoNewPrivileges=true`.
   - Pinned all CI/Release GitHub Actions to 40-character SHAs with explicit `permissions: contents: read`, removed obsolete `RUSTSEC-2024-0449` ignore, documented VEX `not_affected` for `RUSTSEC-2026-0459`, and added `SECURITY.md`.

## Consequences

- Default configurations meet secure-by-default CRA expectations without breaking intentional lab/testing flags.
- `dtnpm-web-core` UI assets and sibling crates are checked out deterministically and integrity-verified via SHA-pinned GitHub Actions and `cargo deny` / `cargo audit` / Syft CycloneDX SBOM generation.
