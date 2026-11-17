# Agent Guide — `dtnpm`

## 🎯 Mission
Provide the standalone `dtnpm` BPA daemon, the interactive configuration assistant (`wizard`), the DTN command-line utilities (`dtnprint`, `dtnsend`, `dtnquery`, `dtntrigger`, `dtnping`, `dtnfiles`, `dtnforward`, `dtnbib`, `hdy-stats`), as well as the orchestration of convergence layers (`dtnpm-cla-ltp-kiss`, `dtnpm-cla-hqfbp-kiss`, `dtnpm-cla-mercury-arq`), WebUI modules (`dtnpm-web-core`, `dtnpm-mod-*`), and the Basket application (`dtnpm-app-basket`), interacting directly with `hardy`'s DTN BPv7 implementation.

---

## 📝 Documentation Language

- **English must be used as the language for all documentation**, unless the file is suffixed with `.fr.md` or another language is explicitly requested in the prompt.

---

## 🛡️ Security Frameworks and Regulatory Requirements

Every agent and developer working on this repository MUST integrate the following requirements by design:

### 1. Cyber Resilience Act (CRA - EU Regulation 2024/2847)
- **Secure-by-Default**: All features must be configured in their most protective state by default.
- **Least Privilege and System Isolation**:
  - Strict `0600` / `0700` permissions on configuration files containing secrets (`config.toml`, keystores, tokens).
  - Systemd services (`dtnpm.service`, `dtn-hdy-ham.service`) running under an unprivileged user (`User=dtnpm`, `Group=dtnpm`) with active sandboxing (`ProtectSystem=strict`, `ProtectHome=true`, `NoNewPrivileges=true`, `RestrictSUIDSGID=true`, `StateDirectory=dtnpm`, `ConfigurationDirectory=dtnpm`).
- **Zero Secrets in Command Lines**:
  - NEVER force passing tokens or passwords via CLI arguments visible in `/proc/<pid>/cmdline`.
  - Systematically use `--token-file <PATH>` or environment variables (`DTN_HDY_TOKEN_FILE`, `DTN_HDY_TOKEN`), and `--password-stdin` for user administration commands.
- **Supply-Chain & VEX (Vulnerability Exploitability eXchange)**:
  - Any `cargo-audit` / `cargo-deny` security advisory must be immediately resolved or formally justified with a VEX document in `.cargo/audit.toml` and `deny.toml` (e.g., `RUSTSEC-2026-0459` marked `not_affected` because the SQLite session extension is unused).
  - Automatic generation of CycloneDX SBOM (`sbom.cdx.json`) and cryptographic checksums (`SHA256SUMS`) during releases.
  - Cryptographic pinning of CI dependencies: all GitHub Actions must be pinned by full 40-character SHA-1 hash with minimal permissions block `permissions: contents: read`.
- **Vulnerability Disclosure Policy**: Documented and kept up to date in [SECURITY.md](SECURITY.md).

### 2. OWASP (Top 10 / ASVS / API Security)
- **A01 Broken Access Control & IAM**:
  - The local gRPC gateway uses a secure registration gate (`GrpcRegistrationGate` / `GuardedBpaRegistration`) preventing any TOCTOU race window at startup.
  - PBAC (Policy-Based Access Control) via the Rego engine and strict role isolation (`operator`, `admin`).
- **A02 Cryptographic Failures**:
  - Systematic use of SHA-256 for digests (banning SHA-1 for integrity).
  - Strict BPSec (RFC 9172) verification by default: rejection of unsigned bundles or bundles with unknown keys.
- **A03 Injection & Sanitization**:
  - Strict sanitization of input strings, endpoint identifiers (EIDs), amateur radio callsigns (`[A-Z0-9/-]+`), and filenames (stripping path traversals `..`, dangerous characters, and enforcing length limits).
  - Shell-less subprocess execution (`tokio::process::Command::kill_on_drop(true)` with bounded timeout, vectorized arguments, and `--` separator).
- **A05 Security Misconfiguration & Explicit Opt-In**:
  - Enabling insecure behavior or degraded modes (e.g., unsigned network links, disabling gRPC authentication in lab environments, generic RF callsign `N0CALL`, allowing private addresses) is ONLY allowed if the user EXPLICITLY requests it (e.g., `--allow-unauthenticated-udp`, `--allow-unidentified-rf`, `--allow-third-party-report-to`).

---

## 🦀 Rust Engineering Best Practices

1. **Zero Panic / Zero Unwrap Rule**:
   - Production code (`src/lib.rs`, `src/main.rs`, `src/bin/*.rs`) MUST be 100% free of `.unwrap()`, `.expect()`, and `panic!` macro invocations.
   - Use error propagation `?`, `.ok_or_else()`, or `thiserror` / `anyhow` error types.
   - Strict linter enforced in CI and pre-commit: `-D warnings -D clippy::unwrap_used -D clippy::panic`.
2. **Architecture Process and Traceability (ADR)**:
   - Any architectural decision, protocol evolution, or component addition MUST be documented in the `adr/` directory as a numbered Architectural Decision Record, and indexed in [adr/README.md](adr/README.md).
3. **Robustness and Resource Bounding**:
   - Always bound memory allocations, stream reads (`.take()`), concurrent process semaphores, and peer tracking tables.
   - Prevent Denial-of-Service (DoS) attacks via memory exhaustion (OOM) or CPU saturation.

---

## ⚡ Critical Verification Commands

Before ANY task finalization or commit proposal, you must run:

- **Formatting**: `cargo fmt --all -- --check`
- **Security & Quality Linter**: `cargo clippy --offline --workspace --lib --bins -- -D warnings -D clippy::unwrap_used -D clippy::panic`
- **Full Tests**: `cargo test --offline --all-targets`
- **Build Profiles**:
  - `cargo check --offline --no-default-features --features "amsat"`
  - `cargo check --offline --no-default-features --features "node-headless"`
  - `cargo check --offline --no-default-features --features "webui-remote"`
- **Full Security Audit**: `./scripts/check-security.sh --strict`
- **Code Coverage**: `./scripts/check-coverage.sh --summary-only`
- **Install Local Git Hooks**: `./scripts/install-hooks.sh`

---

## 🗺️ Organization Map & Project Repository Layout

- `src/`: Standalone `dtnpm` BPA daemon, `wizard` assistant, gRPC security gateway, and CLI binaries in `src/bin/`.
- `../dtnpm-core`: Core data models, internal gRPC client (`create_remote_bpa`), PBAC IAM gateway, BPSec keystore, contact-plan scheduling.
- `../dtnpm-web-core`: Axum HTTP/Web gateway, `WebModule` interface, IAM authentication, proxying, and secure Preact UI components.
- `../dtnpm-cla-ltp-kiss`: LTP convergence layer (RFC 5326 / RFC 7122) and UDP/KISS gateway.
- `../dtnpm-cla-hqfbp-kiss`: HQFBP radio convergence layer over KISS TCP/Serial/UDP with FEC error correction.
- `../dtnpm-cla-mercury-arq`: Mercury ARQ connected HF radio convergence layer.
- `../dtnpm-mod-chat`: BPSec-secured DTN instant messaging WebUI application module.
- `../dtnpm-mod-files`: DTN file sharing and transfer WebUI application module with quota controls.
- `../dtnpm-mod-senml`: SenML (RFC 8428) telemetry collection and visualization WebUI application module.
- `../dtnpm-app-basket`: Daemon (`dtnbasket`) and CLI (`dtnbasket-cli`) for the DTN Basket protocol with SSRF mitigation.
- `adr/`: Architecture Decision Records registry (ADRs).
- `agent_docs/`: Technical deep-dive documentation for autonomous agents.
- `examples/`: Reference configurations, notification scripts, and demonstrators.
- `scripts/`: Security validation scripts (`check-security.sh`), coverage (`check-coverage.sh`), and Git hooks.

---

## 📚 Documentation Index

1. [agent_docs/conventions.md](agent_docs/conventions.md): Naming conventions, I/O streams, time handling, and Zero-Unwrap rules.
2. [agent_docs/testing_guidelines.md](agent_docs/testing_guidelines.md): Integration testing strategy, offline mocks, and coverage.
3. [agent_docs/architecture.md](agent_docs/architecture.md): System architecture, secure gRPC gateway, and data flows.
4. [agent_docs/deployment.md](agent_docs/deployment.md): Secure deployment guide with systemd (CRA sandboxing).
5. [adr/README.md](adr/README.md): Index of all Architectural Decision Records.
6. [SECURITY.md](SECURITY.md): Security policy and vulnerability disclosure process.
