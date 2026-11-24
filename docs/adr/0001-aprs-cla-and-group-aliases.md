# ADR-0001: Unified Rego Routing, CLA/Band Aliases, and ANSRVR Group Aliases

- **Status:** Accepted & Implemented (Phase 1 + Phase 2)
- **Date:** 2026-11-24
- **Scope:** `dtnpm-core`, `dtnpm-mod-rego-routing`, `dtnpm`, and future `dtnpm-app-ansrvr` / module

---

## 1. Context

`dtnpm` nodes deployed on amateur radio networks (44Net TCPCLv4, VHF/UHF KISS, HF Mercury ARQ, QO-100 / satellite LTP) need a safe, zero-surprise default routing policy that works identically on both **hub/relay nodes** (e.g. `DTN-F4JXQ`) and **leaf/edge nodes** without manual Rego authoring:

1. **Why `DefaultRib` blanket wildcards fail:** Pushing catch-all `routes` like `dtn:**` from `routing.rego` into Hardy's `DefaultRib` causes `DefaultRib` to forward bundles back to the peer they just arrived from, because `DefaultRib` only matches destination EIDs and cannot inspect `previous_node`, `source`, `treb_visited_nodes`, or `delivered_peers`. Hardy's `RegoEgressRewriter` runs after route selection and cannot cancel a forward.
2. **Why traditional APRS `WIDE1`/`WIDE2` aliases do not fit DTN:** On AX.25 APRS, `WIDE1-1` / `WIDE2-2` means generic local RF digipeating. In a multi-transport DTN node bridging TCPCLv4, VHF, UHF, 10m, 2.4 GHz, and QO-100, generic `WIDE` flooding would leak RF traffic onto Internet links or vice versa, and lacks any semantic meaning for operators.

---

## 2. Decisions

### Decision 1: Unified Default `routing.rego` Precedence (`decide` + Empty `routes`)

Every `dtnpm` instance ships with an embedded `routing.rego` where:
- `default routes := []` — `routing.rego` never installs blanket wildcards into `DefaultRib`. Static routes configured by the operator in `config.yaml` / `config.toml` (`routes:`) remain in `DefaultRib` via `RouteManager` as the `FALLBACK` layer.
- `RegoRib::decide(&bundle)` evaluates per-bundle forwarding in the following strict precedence order:
  1. **Registered local service / admin endpoint** (enforced in Rust `RegoRib::decide` before Rego) → `Deliver`
  2. **Unregistered local-node destination** (`is_local_node(destination)`) → `WAIT` (never spray or leak local typos outward)
  3. **Direct connected destination** (`is_same_node(peer.peer_eid, destination)`) → `FORWARD` (`retain: false`)
  4. **APRS strict next-hop** (`aprs_strict_next_hop != null`) → `FORWARD` (`retain: false`)
  5. **APRS CLA / band alias hop** (`aprs_active_alias != null`) → `FORWARD` (`retain: true`, strictly constrained to connected peers on a CLA advertising that alias; otherwise `WAIT`)
  6. **APRS ANSRVR group alias hop** *(Phase 2)* → `FORWARD` (`retain: true`, constrained to subscribed peers; otherwise `WAIT`)
  7. **Spray-and-Wait Spray phase** (`spray_quota > 1`) → `FORWARD` (`retain: true`, quota split in `egress.rego`)
  8. **Spray-and-Wait Wait phase** (`spray_quota != null`, `L <= 1` or all relays already delivered) → `WAIT`
  9. **Plain bundle** → `FALLBACK` to `DefaultRib` (operator static `routes:` if configured, otherwise Hardy's Default-to-Wait)

**Loop & bounce-back exclusion set** (applied to every non-direct relay candidate):
- Local node EIDs (`is_local_node`)
- Blacklisted peers (`not helpers.peer_allowed`)
- `input.bundle.previous_node` (RFC 9171 Block 6)
- `input.bundle.source` (guards against senders such as `dtn-android-messenger` that omit Block 6)
- `input.bundle.treb_visited_nodes` (multi-hop past path)
- `input.bundle.delivered_peers` (peers already forwarded a copy)

### Decision 2: Kind 1 APRS Aliases — CLA / Band / Transport Aliases (Phase 1)

APRS trajectory aliases in Block Type 200 (`ScopedAlias`) represent **transport or band scopes** rather than `WIDE1`/`WIDE2`:

- **Explicit per-CLA `aliases`:** Each `clas[]` entry in `HamConfig` accepts `aliases: Vec<String>` (e.g. `["VHF", "2M"]`, `["UHF"]`, `["10m"]`, `["2G4"]`, `["QO100"]`), normalized to uppercase ASCII.
- **Automatic transport tag:** `canonical_cla_aliases(cla_type, configured)` automatically adds the canonical transport tag for the CLA type:
  - `tcpclv4` / `tcpcl` → `TCPCL`
  - `ltp` → `LTP`
  - `hqfbp` → `HQFBP`
  - `mercury-arq` / `mercury` → `MERCURY`
- **Hop-count decrement (`New n-N Paradigm` only):** `parse_aprs_path` and `ingress.rego` strictly enforce the APRS **New n-N Paradigm** (`1 <= N <= n <= 7`):
  - Standard form `<SCOPE><n>-<N>` (e.g., `VHF2-2`, `10M2-1`, `TCPCL1-1`, `ARES2-2`, `WIDE1-1`).
  - Hyphenated form `<SCOPE>-<n>-<N>` for digit-ending aliases (e.g., `QO100-1-1`, `2G4-2-2`).
  - Legacy `WIDE1`/`WIDE2` are removed from `RegoRoutingConfig::default()` (`supported_aliases = []`, `trapped_aliases = []`) and are not matched unless explicitly configured.
- **Strict CLA constraint:** When `aprs_active_alias` is active, `RegoRib::decide` only forwards to connected peers whose `cla_aliases` contains that alias (passing `peer_id` so multi-CLA peers use the matching CLA). If no matching CLA peer is connected, the bundle parks in `WAIT` rather than leaking onto another CLA.
- **SSID disambiguation guarantee:** Because only New n-N (`<SCOPE><n>-<N>` or `<SCOPE>-<n>-<N>`) is parsed as a `ScopedAlias`, amateur radio callsigns with SSIDs (e.g., `F4JXQ-9`, `F4JXQ-1`, `N0CALL-15`) and legacy non-n-N strings (`VHF-2`, `ARES-2`) unambiguously parse as `StrictTarget` hops without colliding with aliases.

### Decision 3: Kind 2 APRS Aliases — ANSRVR-Style Group Aliases (`dtn://<node>/ansrvr`) (Phase 2)

To support amateur-radio group announcements and group-scoped Block 200 routing:

- **Hybrid architecture:**
  1. A local `ansrvr` service (`dtn://<node>/ansrvr`) accepts plain-text APRS-style management commands:
     - `J <group>` / `JOIN <group>` — subscribe sender to `<group>` with configurable TTL (default 24h)
     - `U <group>` / `UNJOIN <group>` — unsubscribe sender from `<group>`
     - `K <group>` / `KEEPALIVE <group>` — refresh subscription TTL
     - `L` / `LIST [group]` — reply with active groups / subscriber list
     - `CQ <group> <message>` — fan-out announcement bundle to each subscribed member EID
  2. Active SQLite subscriptions are fed into `RegoRoutingEngine` (bumping `policy_version` and notifying `RegoRib`) so Block 200 New n-N group alias hops (e.g. `ARES2-2`, `CQ1-1`) route/replicate to currently connected peers subscribed to that group.
- **Security, CRA, and IAM constraints (mandatory for Phase 2):**
  - **Source-only self-management:** `J`, `U`, and `K` apply strictly to `bundle.primary.source` (normalized to node EID); third-party subscribe/unsubscribe requests are rejected.
  - **BPSec enforcement:** `/ansrvr` honours the station's `DirectionalKeyStore` / `VerifyPolicy` (`Strict` / `Warn` / `Ignore`).
  - **Reserved alias namespace:** Built-in transport tags (`TCPCL`, `LTP`, `HQFBP`, `MERCURY`) and configured CLA/band aliases cannot be registered or hijacked as user group names.
  - **Resource bounds:** Bounded maximum groups (`max-groups`), maximum subscribers per group (`max-members-per-group`), 60s TTL expiration reaper, and per-source rate limiting (`cq-rate-limit-secs`) on `CQ` fan-out.
  - **REST API (`dtnpm-mod-rego-routing`):** `GET /api/rego_routing/v1/groups` and `DELETE /api/rego_routing/v1/groups/{group}?member=<node_eid>`.

### Decision 4: `record-treb` and Companion Fixes

- `RegoRoutingYamlConfig` exposes `record-treb` (default `false` to avoid adding Block 201 overhead on low-bandwidth RF links unless enabled), `supported-aliases`, and `trapped-aliases`.
- `RouteManager::parse_route` assigns priority `100` to specific static routes and `200` to default catch-all wildcards (`dtn:**`, `ipn:**`, `*:**`), fixing the previous priority inversion (`10` vs `100`) in Hardy's RIB where lower numbers take precedence.

---

## 3. Consequences

- Hub and leaf `dtnpm` nodes can share the exact same embedded `routing.rego` with zero manual Rego edits.
- Plain bundles never bounce back to senders or leak across unrelated peers; Spray-and-Wait and CLA-scoped APRS bundles work out of the box.
- Phase 2 (`/ansrvr`) can be added cleanly as a new service + `RegoRoutingEngine` group-membership hook without breaking Phase 1 config or Rego contracts.
