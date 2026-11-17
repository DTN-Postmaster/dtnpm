# Development Conventions — `dtnpm`

This document formalizes the conventions, coding standards, and robustness requirements applicable to the `dtnpm` project.

---

## 1. Coding Standards & Zero Panic / Zero Unwrap

- **Absolute Rule**: No calls to `.unwrap()`, `.expect()`, or invocations of the `panic!()` macro are allowed in production code (`--lib`, `--bins`).
- **Error Handling**:
  - Use the `?` operator to propagate errors.
  - Transform options via `.ok_or_else(|| ...)` or `.ok_or(...)`.
  - For mutexes and shared locks, handle poisoned locks safely: `lock().map_err(|e| ...)` or `unwrap_or_else(|e| e.into_inner())`.
- **Static Validation**: The strict linter must be executed and pass with zero warnings:
  ```bash
  cargo clippy --workspace --lib --bins -- -D warnings -D clippy::unwrap_used -D clippy::panic
  cargo fmt --all -- --check
  ```

---

## 2. Command-Line Arguments & Secrets Management (CRA)

- **`clap` CLI Parsing**: All commands use `clap`'s `derive` attribute.
- **Secure Secrets Management**:
  - In accordance with Cyber Resilience Act (CRA) requirements, NEVER force the user to pass passwords or authentication tokens in plaintext on the command line.
  - Systematically prefer `--token-file <PATH>` (with support for the `DTN_HDY_TOKEN_FILE` environment variable).
  - The deprecated `--token` option must print a security warning to `stderr` urging migration to `--token-file`.
  - For user creation/modification commands (`user add`, `user passwd`), provide the `--password-stdin` option to inject the secret from a secure channel.
- **Port Resolution**:
  - Default local gRPC port: `50051`.
  - Default Web / HTTP port: `3000`.
  - Always honor the priority environment variables `HARDY_GRPC_PORT` or `DTN_WEB_PORT`.

---

## 3. Output Stream Separation

- **`stdout`**: Reserved exclusively for functional outputs and data consumable by piped scripts (decoded payloads, bundle IDs, serialized CBOR, JSON statistics, digest hashes, ping metrics).
- **`stderr`**: Reserved exclusively for logging, security warnings, execution traces, and error messages.
- **Atomic Writes**: Lock `std::io::stdout().lock()` when writing multi-line blocks to prevent interleaving during concurrent executions.

---

## 4. Dates and Timestamps

- Systematically use the `time` library (specifically `time::OffsetDateTime`).
- Format dates as `Rfc3339`-compliant strings (`time::format_description::well_known::Rfc3339`).

---

## 5. Explicit Security Overrides (Opt-in)

- Any permissive behavior (e.g., accepting unsigned traffic, tolerating unidentified peers) must require an explicit CLI flag or a dedicated option in the configuration file (e.g., `--allow-unauthenticated-udp`, `--allow-unidentified-rf`, `--allow-third-party-report-to`).
- It is strictly forbidden to lower default security levels without explicit user consent.

---

## 6. Architectural Traceability (ADR)

- Any modification to structure, data models, or security protocols must be recorded in a new ADR in `adr/` and referenced in `adr/README.md`.
