# Testing & Quality Assurance Guide — `dtnpm`

This document presents the testing strategy, offline validation, security guardrails, and coverage measurement for the `dtnpm` project.

---

## 1. Unit Tests & Local Integrity

- **Zero Network Dependency in Tests**: All test suites must be executable in offline mode (`--offline`).
- **CBOR & EID Encoding/Decoding Tests**: Validate that Endpoint Identifiers (EIDs) and BPv7 blocks (RFC 9171) conform to specifications and are protected against integer overflows or unbounded allocations.
- **BPSec Verification**:
  - Test all three policy modes (`Strict`, `Warn`, `Ignore`).
  - Verify that under `VerifyPolicy::Strict`, unsigned bundles (`Unsigned`), bundles with missing keys (`MissingKey`), or corrupted bundles (`Invalid`) are systematically rejected.
- **Sanitization and Memory Bounds**:
  - Validate filename sanitization in `dtnfiles` (protection against relative paths, escape characters, and overly long names).
  - Validate whitelist filtering of journald units in `hdy-stats` (rejection of leading dashes and injection metacharacters).
  - Validate SQL filter queries `--addr` and `--limit` in `dtnquery`.

---

## 2. Integration Tests & Service Mocks

- Integration tests simulate real interactions between components:
  - **`iam_integration_test.rs`**: Validates PBAC access control, user management, sessions, and service registration via the gRPC gateway and UNIX socket.
  - **`bpsec_chat_files_test.rs`**: Validates the full BPSec signing and verification chain for the Chat and Files modules.
  - **`ham_server_test.rs`**: Validates `dtnpm` server initialization with various TOML/YAML configurations, configuration parity, and CLA management.
  - **`bundle_inspector_test.rs`**: Validates bundle inspection in memory and on SQLite disk storage.
- Asynchronous tests use `tokio::test` with reasonable timeouts to prevent test harness hangs.

---

## 3. Code Coverage (`cargo-llvm-cov`)

To monitor for regressions in test coverage:
- Concise console report: `./scripts/check-coverage.sh --summary-only`
- Full report: `./scripts/check-coverage.sh`
- Interactive HTML report: `./scripts/check-coverage.sh --html` (available at `target/llvm-cov/html/index.html`)
- CI export: `./scripts/check-coverage.sh --lcov` (generates `lcov.info`)

> [!NOTE]
> Never commit locally generated `lcov.info` or `sbom.json` files to the Git working tree.

---

## 4. Pre-Delivery Validation Matrix

Before finalizing a task or creating a commit, you must run:

1. `cargo fmt --all -- --check`
2. `cargo clippy --offline --workspace --lib --bins -- -D warnings -D clippy::unwrap_used -D clippy::panic`
3. `cargo test --offline --all-targets`
4. Specific profiles:
   - `cargo check --offline --no-default-features --features "amsat"`
   - `cargo check --offline --no-default-features --features "node-headless"`
   - `cargo check --offline --no-default-features --features "webui-remote"`
5. Security audit: `./scripts/check-security.sh --strict`
