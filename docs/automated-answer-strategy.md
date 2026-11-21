# Automated Answer & Local Origination Routing Strategy

## 1. Problem Statement

In `dtnpm`, dynamic and ad-hoc multi-hop routing relies on BPv7 extension blocks evaluated by `dtnpm-mod-rego-routing`:

- **Block 200 (`block_type_mesh_routing`)**: carries `trajectory_control` (APRS-like ordered future hops with `hop_kind_strict = 1` or `hop_kind_alias = 2` such as `WIDE1-1`, `WIDE2-2`) and `replication_control` (Spray-and-Wait).
- **Block 201 (`default_block_type_treb`)**: Trace Route Extension Block (TREB), appending each visited node (`node_eid`, `arrival_time_ms`, `departure_time_ms`, `cla_info`) as the bundle traverses the network.
- **Block 6 (`block_type_previous_node`)**: RFC 9171 Previous Node block.
- **Block 10 (`block_type_hop_count`)**: RFC 9171 Hop Count limit.

When a bundle arrives at a local service (such as `dtntrigger`, `chat`, `files`, or `senml`) and triggers an **automated reply**:

1. `dtntrigger` currently invokes external commands with only two positional arguments: `$1` (`source` EID) and `$2` (path to the payload temp file).
2. Responder scripts (e.g. `examples/dtn-fortune.sh`) call `dtnsend -s fortune -r "$SOURCE_EID"` without `--aprs-path`, `--spray-quota`, or `--hop-limit`.
3. If the responding node has no static route back to `$SOURCE_EID` (for example, when `$SOURCE_EID` reached it via a dynamic `--aprs-path`, an RF digipeater, or an intermediate hub), the reply bundle has **no routing extension block** and either stalls in `Waiting` or is dropped (`no_known_route`).

---

## 2. Two-Layer Strategy

A robust solution combines **explicit return-path metadata at the service layer** with an **app ingress filter at local origination** so both smart responders and legacy tools (`dtntrigger` scripts, `dtnsend`, WebUI replies) work without manual route configuration.

### Layer A — Service-Level Return-Path Extraction (`dtntrigger` & Built-in Services)

1. **Compute a suggested return trajectory on bundle delivery**:
   - **Primary source (Block 201 — TREB)**: Read `hops[]` from the incoming bundle's TREB, reverse the list, remove the local node EID (and any local aliases), and emit the remaining relay/source EIDs as strict hops (`hop_kind_strict = 1`).
   - **Secondary source (Block 6 — Previous Node)**: If no TREB is present, use the Previous Node EID as the immediate first return hop (when different from `source`).
   - **Fallback**: If the incoming bundle carried an APRS alias path (`WIDE1-1`, `WIDE2-2`) and no TREB/Previous Node is available, suggest a bounded alias path or Spray-and-Wait quota.

2. **Expose metadata cleanly in `dtntrigger` (100% backwards-compatible)**:
   - Keep `$1` (`source`) and `$2` (`payload_file`) unchanged so existing scripts continue to work.
   - Export environment variables to the child process:
     - `DTN_BUNDLE_ID`: canonical bundle ID
     - `DTN_SOURCE_EID`: full source EID
     - `DTN_PREVIOUS_NODE`: Block 6 EID (if present)
     - `DTN_TREB_HOPS`: comma-separated forward TREB hops (if present)
     - `DTN_SUGGESTED_RETURN_PATH`: reversed, deduplicated strict hop list ready for `dtnsend --aprs-path "$DTN_SUGGESTED_RETURN_PATH"`
   - Update `examples/dtn-fortune.sh` (and similar trigger examples) to pass `--aprs-path "$DTN_SUGGESTED_RETURN_PATH"` whenever `DTN_SUGGESTED_RETURN_PATH` is non-empty.

3. **Maintain a bounded Peer-Path Cache in `dtnpm-core`**:
   - Whenever any local service (`dtntrigger`, `chat`, `files`, `senml`) receives a bundle with TREB or Previous Node info, update an in-memory (optionally SQLite-backed) LRU cache keyed by remote node EID (`dtn://<remote>/`) with a configurable TTL (e.g. `15m`–`1h`).
   - Interactive replies in the WebUI (`chat`, `files`) and automated responders that do not inspect incoming headers can still benefit automatically via Layer B.

---

### Layer B — App Ingress Filter (Local-Origination Policy & Block Injection)

Implementing an **App Ingress Filter** at the boundary where local applications submit bundles to the BPA (gRPC `ApplicationService` / `dtnsend` and internal `ServiceSink::send`) solves the problem even for unmodified tools and scripts.

#### Where it sits
```text
[dtnsend / dtntrigger script / WebUI chat / files]
                     │
                     ▼  (local bundle submission, before BPA routing)
         ┌───────────────────────┐
         │   App Ingress Filter  │  ◄── inspects missing Blocks 10, 194, 200, 201
         │  (app_ingress.rego +  │  ◄── consults Peer-Path Cache + FIB
         │   Peer-Path Cache)    │
         └───────────┬───────────┘
                     │  injects missing routing/trace blocks
                     ▼
         [Hardy BPA Storage & FIB / Rego Routing]
```

#### Decision Rules (in priority order)

1. **Respect explicit application blocks (no-op on override)**:
   - If the submitted bundle already has Block 200 (`trajectory_control` / `replication_control`), Block 194 (`spray_permit`), or Block 10 (`hop_count`), do **not** overwrite those blocks.
2. **Check whether a static/connected route already exists**:
   - If the destination is directly connected as an active peer or matched by a specific static route (not just a catch-all), no trajectory injection is required (though Block 10 / Block 201 can still be added if configured).
3. **Consult the Peer-Path (TREB) Cache**:
   - If the destination node EID has a fresh cached return path from a recent incoming bundle, inject Block 200 (`trajectory_control`) with `hop_kind_strict = 1` along that reversed path.
4. **Evaluate `app_ingress.rego` fallback policy**:
   - Allow operators to express per-destination, per-service, or per-CLA defaults in Rego (mirroring `ingress.rego` / `egress.rego`), for example:
     - Inject a default `Hop Count` (Block 10, e.g. `max_hops = 15`) on all locally originated bundles lacking one.
     - Initialize `TREB` (Block 201) when `node.record_treb == true` so the reply itself records its path for downstream nodes.
     - Inject a default APRS alias path (e.g. `WIDE1-1,WIDE2-2`) or Spray-and-Wait permit (e.g. `spray_quota = 4`) when no static route and no cached TREB path exist.

---

## 3. Security, Loop Prevention & Edge Cases

- **Loop & storm prevention**:
  - Always strip the local node EID (and any configured local aliases) from the reversed TREB path before injecting Block 200.
  - Use `hop_kind_strict` (not `WIDE` flooding) for cached TREB return paths, and always attach Block 10 (`hop_count`) as a hard upper bound.
- **Stale topology handling**:
  - DTN links are intermittent; a cached TREB path from hours ago may no longer be reachable. Keep the Peer-Path Cache TTL bounded and optionally fall back to Spray-and-Wait / default gateway if delivery fails or expires.
- **Cache poisoning / spoofing**:
  - When BPSec verification (`verify_policy`) is enabled, only update the Peer-Path Cache from bundles whose signature verified (`VerifyResult::Valid`) or from trusted directly-connected peers.
  - Bound the cache size (e.g. LRU of 1,024 remote nodes) so an external sender cannot exhaust memory.
- **Multi-CLA / asymmetric links**:
  - A path traversed inbound (e.g. RF KISS -> Gateway -> TCPCLv4) may differ from the preferred outbound link. `egress.rego` and local static routes with higher priority should still take precedence when a direct or preferred route is available.

---

## 4. Recommended Implementation Phases

1. **Phase 1 (Quick win — `dtntrigger` + `dtn-fortune.sh`)**:
   - Pass incoming bundle metadata (`Block 201` TREB / `Block 6` Previous Node) through `ServiceSink` / gRPC delivery to `dtntrigger`, export `DTN_SUGGESTED_RETURN_PATH` in the environment, and update `examples/dtn-fortune.sh` to pass `--aprs-path`.
2. **Phase 2 (Peer-Path Cache in `dtnpm-core`)**:
   - Record `(remote_node_eid -> reversed_treb_hops, timestamp)` on service delivery in a bounded TTL-LRU cache.
3. **Phase 3 (App Ingress Filter + `app_ingress.rego`)**:
   - Hook local origination in `dtnpm-core` / `dtnpm-mod-rego-routing` before BPA dispatch to automatically attach Block 10, Block 201, and (when missing and needed) Block 200 / Block 194 according to cache + Rego policy.
