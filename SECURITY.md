# Security Policy (`dtnpm`)

## Supported Versions

| Version | Supported | Notes |
| --- | --- | --- |
| `0.2.x` (`main`) | Yes | Active security updates & Cyber Resilience Act (CRA) alignment |
| `< 0.2.0` | No | Pre-split development snapshots |

## Reporting a Vulnerability

Please report security vulnerabilities responsibly rather than opening public GitHub issues:

1. Use **GitHub Private Vulnerability Reporting** (`Security` -> `Report a vulnerability`) on the `dtn-postmaster/dtnpm` repository, or contact the maintainer directly.
2. Include the affected component/binary (`dtnpm`, `dtnbib`, `dtnfiles`, `dtntrigger`, `hdy-stats`, etc.), reproduction steps, configuration (with secrets redacted), and impact assessment.
3. Initial acknowledgement target: **72 hours**; remediation and coordinated disclosure advisory target: **14–30 days** depending on severity.

## Security Defaults & Explicit Permissive Opt-Ins

- **gRPC Authentication & Registration Gate (`DTNPM-01`)**: Internal ephemeral loopback gRPC sockets are guarded by `GrpcRegistrationGate` / `GuardedBpaRegistration` so unprivileged local processes cannot bypass `GrpcSecurityGateway`. Direct unauthenticated gRPC is only exposed if `[grpc.auth] enabled = false` is explicitly configured without a security gateway.
- **Credentials & Config Files (`XC-05`, `CRA-01`)**:
  - `dtnpm user add` and `dtnpm user passwd` support `--password-stdin` (and warn when deprecated `--password` is passed on CLI).
  - All gRPC client CLIs (`dtnpm`, `dtnbib`, `dtnfiles`, `dtnforward`, `dtnping`, `dtnprint`, `dtnsend`, `dtntrigger`, `hdy-stats`) support `--token-file` (`0600`) in addition to `DTN_HDY_TOKEN` / UDS `--socket`.
  - Wizard-generated configuration files are saved with `0600` POSIX permissions.
  - Provided `systemd` units run under dedicated unprivileged user/group `dtnpm` with `ProtectSystem=strict`, `ProtectHome=true`, and `NoNewPrivileges=true`.
- **Utility Hardening (`DTNPM-02`, `CLI-01`)**:
  - `dtnbib` performs checked CBOR offset arithmetic and exact `NodeId` authority checks for `--no-transit`.
  - `dtntrigger` bounds concurrent command execution (8 permits, 60s timeout with `kill_on_drop`), and example helper scripts pass `--` before untrusted arguments.
  - `dtnfiles` enforces `--max-size-mb` (default `100`) prior to stream buffering and sanitizes bundle ID filenames strictly to `[A-Za-z0-9_-]`.
  - `hdy-stats` restricts `report_to` amplification to source `NodeId` authority unless `--allow-third-party-report-to` is explicitly passed, and validates `--journald-unit`.

## Coordinated Vulnerability Disclosure & VEX

CycloneDX SBOMs (`sbom.json`) are generated on every CI run. VEX justifications (including `RUSTSEC-2026-0459` `not_affected` for unused SQLite session APIs) are tracked in `.cargo/audit.toml` and `deny.toml`.
