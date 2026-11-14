# Agent Router Guide

## Mission
Provide the `dtnpm` standalone BPA daemon and command-line utilities (`dtnprint`, `dtnsend`, `dtnquery`, `dtntrigger`, `dtnping`, `dtnfiles`, `dtnforward`, `dtnbib`, `hdy-stats`, and sibling `dtnpm-app-basket` binaries `dtnbasket` / `dtnbasket-cli`), along with sibling convergence layers (`dtnpm-cla-ltp-kiss`, `dtnpm-cla-hqfbp-kiss`, `dtnpm-cla-mercury-arq`) and WebUI modules (`dtnpm-web-core`, `dtnpm-mod-*`), for interacting with and deploying the `hardy` BPA implementation of DTN BPv7.
The source code of the `hardy` project is located at `../../hardy`.
The source code of the `DTN7` project is located at `../../dtn7-rs`.

## Critical Commands
Always run verification tests and check formatting before final delivery:
- **Build**: `cargo build`
- **Test**: `cargo test`
- **Lint**: `cargo clippy && cargo fmt`
- **Panic & Unwrap Guardrails**: Production code (`--lib`, `--bins`) MUST be 100% `unwrap`-free and `panic!`-free. Use `?`, `.ok_or_else()`, or poison-safe mutex access.
- **Security & Quality**: `./scripts/check-security.sh` (or `./scripts/check-security.sh --strict`)
- **Code Coverage**: `./scripts/check-coverage.sh` (or `./scripts/check-coverage.sh --html` / `--summary-only`)
- **CI Sync**: Whenever updating `hardy` dependencies or signatures, update the `hardy` checkout `ref` in `.github/workflows/ci.yml`.

## Directory & Organization Map
- `src/`: Standalone `dtnpm` BPA daemon, wizard, server wiring, and CLI utility binaries in `src/bin/` (`dtnprint`, `dtnsend`, `dtnquery`, `dtntrigger`, `dtnping`, `dtnfiles`, `dtnforward`, `dtnbib`, and `hdy-stats`).
- `../dtnpm-core`: Core library (`HamConfig`, storage, routing, CLA manager, BPsec keystore, `BpaClient`, Basket primitives).
- `../dtnpm-web-core`: Axum WebUI server, `WebModule` plugin trait, and core Preact UI.
- `../dtnpm-cla-ltp-kiss`: LTP (RFC 5326 / RFC 7122) engine (`hardy-ltp`) and UDP/KISS CLA.
- `../dtnpm-cla-hqfbp-kiss`: HQFBP over KISS TCP / Serial / UDP CLA.
- `../dtnpm-cla-mercury-arq`: Mercury HF Connected ARQ CLA.
- `../dtnpm-mod-chat`, `../dtnpm-mod-files`, `../dtnpm-mod-senml`: Fullstack DTN WebUI modules.
- `../dtnpm-app-basket`: DTN Basket Protocol daemon (`dtnbasket`) and CLI (`dtnbasket-cli`).
- `examples/`: Example scripts, station YAML/TOML configs, and demonstrations.
- `tests/interop/scapy-ltp/`: Scapy-based LTP interoperability tests.
- `agent_docs/`: Reference documentation for autonomous developers.
- `adr/`: Architectural Decision Records (ADRs) tracking design and implementation choices.
- `scripts/`: Security (`check-security.sh`), coverage (`check-coverage.sh`), and Git hook (`install-hooks.sh`) scripts.

## Documentation Index
Read these files progressively based on your task context:
1. [agent_docs/architecture.md](agent_docs/architecture.md): Read before changing client registration or data flow logic.
2. [agent_docs/conventions.md](agent_docs/conventions.md): Read before writing new code, options, or refactoring CLI arguments.
3. [agent_docs/testing_guidelines.md](agent_docs/testing_guidelines.md): Read before adding or validating tests.
4. [adr/README.md](adr/README.md): Index of all Architectural Decision Records.
5. [agent_docs/deployment.md](agent_docs/deployment.md): Deployment guide on remote architectures and using systemd services.

## Verification
> [flat]
> [!IMPORTANT]
> You must ALWAYS verify your work by running the complete command check and test suites (`cargo test`, lint checks `cargo clippy && cargo fmt --check`, security guardrails `./scripts/check-security.sh`, and ensuring `.github/workflows/ci.yml` dependencies are synchronized) before finishing any task.




