# ADR 0023: File-Based CLA Integration, Security Boundaries, and Sneakernet Spooling

## Status
Accepted

## Context

Amateur radio and civil defense emergency communications frequently require **air-gapped / sneakernet bundle exchange** (USB flash drives, SD cards) or **filesystem bridges** to external software modems and tooling. Hardy provides `hardy-file-cla`, which watches an `outbox` directory for raw BPv7 files (deleting them on `Acceptance::Accepted` and quarantining them to `outbox/refused/` on `Acceptance::Refused`) and writes forwarded bundles into per-peer `inbox` directories.

Exposing local filesystem I/O inside `dtnpm` introduces security, reliability, and systemd-compatibility hazards that must be bounded by default in accordance with EU CRA (*Secure by Default*) and OWASP ASVS:
1. Unbounded `outbox` ingestion could OOM the node if an oversized file is dropped.
2. Permissive (`0755`/`0777`) spool directories allow local unprivileged users to inject raw BPv7 bundles or read forwarded bundles.
3. Misconfigured `outbox` paths (empty, `"."`, station `data-dir`, `config_path` parent, `$HOME`, `/`, or another CLA's `refused/` directory) could wipe station files or trigger infinite forwarding loops.
4. Multiple peers mapped to the same `inbox` directory in one `file-cla` cause `hardy-bpa` to reject the duplicate `ClaAddress::Private` while `ObservedClaSink` would otherwise desynchronize Rego routing state.
5. `dtnpm.service` enforces `ProtectSystem=strict` and `ProtectHome=true`, blocking `~/...` and `/var/spool/...` unless placed under `StateDirectory=dtnpm` (`/var/lib/dtnpm/...`) or whitelisted via `ReadWritePaths=`.

## Decisions

1. **Multi-Instance `file-cla` & `FILE` APRS Transport Tag (`cla-file` feature):**
   - `dtnpm` integrates `hardy-file-cla` under the `cla-file` feature (enabled in `desktop`, `amsat`, `node-headless`, and `minimal` profiles).
   - Operators may declare multiple `file-cla` / `file` entries with distinct `name`, `outbox`, `peers`, and APRS `aliases`.
   - `dtnpm-core` automatically appends the reserved `FILE` transport tag (`BUILTIN_CLA_TRANSPORT_ALIASES`) so `--aprs-path FILE1-1` reaches all `file-cla` instances while `--aprs-path <CUSTOM>1-1` targets a specific instance.

2. **Typed Configuration (`FileClaHamConfig` in `dtnpm/src/file_cla.rs`):**
   - Parsed via `#[serde(deny_unknown_fields, rename_all = "kebab-case")]` to fail fast on typos (such as `inbox` instead of `peers`).
   - Non-object/non-null `ClaType::Other` payloads are rejected at startup.
   - `ObservedClaSink::add_peer` notifies `PeerObserver::on_peer_up` strictly when `added == true`.

3. **Default `64 MiB` `max-bundle-size` Cap (OOM DoS Prevention):**
   - Every `file-cla` registers with `ClaInit { max_bundle_size: Some(64 MiB), .. }` by default, configurable per instance via `max-bundle-size` (positive integer bytes or byte-size string such as `"16MB"`; `0` is rejected).

4. **Fail-Closed `0700` Spool Permissions & Explicit Opt-In:**
   - Missing `outbox` and `inbox` directories are created with `0700` permissions on Unix (`DirBuilderExt::mode(0o700)`), and newly created directories are cleaned up if subsequent validation fails.
   - Existing spool directories with group/world permissions (`mode & 0o077 != 0`) **fail startup by default** unless `allow-permissive-spool: true` is explicitly configured (which logs a `warn!`).

5. **Path-Safety Invariants (`FileClaPathRegistry`):**
   - Rejects empty/whitespace paths, `"."` / `"./"`, duplicate `outbox` watchers across instances, `outbox`/`inbox` forwarding loops, duplicate peer `inbox` paths within the same `file-cla`, `outbox` pointing to `protected_dirs` (`data-dir`, `payload_dir`, canonical `config_path` parent, `$HOME`, `/`), and any `refused/` quarantine directory overlap.

6. **Dry-Run Pre-Flight Validator (`dtnpm check-config`):**
   - `dtnpm -c <config> check-config` validates node EIDs, `FileClaHamConfig` fields, path-safety invariants, and existing directory permissions without creating directories or starting the BPA.
   - Note on `hardy-bpa` disposal semantics: complete-but-invalid CBOR files dropped in `outbox` are treated by `hardy-bpa` as `Received::Disposed` (`Acceptance::Accepted`) and deleted cleanly, whereas `Acceptance::Refused` (truncation / BPA size cap) quarantines to `outbox/refused/`.

7. **Upstream `hardy-file-cla` Hardening Track:**
   - Runtime improvements inside `hardy/file-cla` itself (atomic `.tmp` → `fsync` → `rename`, `lstat`/`O_NOFOLLOW` symlink rejection, bounded channels, non-panicking watcher recovery, chunked streaming, and FAT32-safe filenames) are tracked upstream in `hardy/FILE-CLA-HARDENING-PROMPT.md`.
