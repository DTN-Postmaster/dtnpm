# Mercury Connected ARQ CLA — Setup and Operations Guide

This guide explains step by step how to configure, deploy, and operate the **Mercury Connected ARQ Convergence Layer Adapter (CLA)** (`dtnpm-cla-mercury-arq`) with **DTN Postmaster** (`dtnpm`) for HF amateur radio BPv7 bundle transport.

---

## 1. Architecture & How It Works

The Mercury Connected ARQ CLA bridges Hardy BPv7 bundles with a **Mercury HF modem TNC** operating in connected ARQ mode over two local TCP sockets:

1. **Control Socket (`control-addr`, default `127.0.0.1:8300`)**:
   - Line-oriented CRLF ASCII protocol.
   - Handles modem initialization (`MYCALL <callsign>`, `BW<hz>`, `LISTEN ON`), outbound dialing (`CONNECT <src> <dst>`), teardown (`DISCONNECT`), and unsolicited modem status lines (`CONNECTED`, `DISCONNECTED`, `WRONG`).
2. **Data Socket (`data-addr`, default `127.0.0.1:8301`)**:
   - Binary stream framed with a `u32` big-endian length prefix per BPv7 bundle (maximum frame size: **16 MiB** / `16,777,216` bytes).

When a link is established (either inbound via `LISTEN ON` or outbound via a contact schedule), the CLA dynamically registers the remote peer's `NodeId` (`dtn://<callsign>/` or an explicit `node-id`) with the BPA (`Sink::add_peer`). When the ARQ session ends or a TCP socket drops, it removes the peer route (`Sink::remove_peer`) and cleans up session state.

### Deployment Modes

- **Integrated Mode (recommended)**: Built directly into `dtnpm` when compiled with the `cla-mercury` Cargo feature (included in the default `desktop` and `node-headless` profiles).
- **Standalone Mode**: Run `dtnpm-cla-mercury-arq` as an independent process that connects to a running `dtnpm` (or Hardy BPA) node over gRPC or a Unix Domain Socket.

---

## 2. Prerequisites

1. **Mercury HF Modem TNC**:
   - Installed and configured to expose its TCP control port (default `8300`) and TCP ARQ data port (default `8301`).
   - Connected to your HF transceiver audio / PTT interface (or a local simulation pipe for bench testing).
2. **DTN Postmaster (`dtnpm`)**:
   - Compiled with `cla-mercury` enabled (default), or `dtnpm-cla-mercury-arq` built for standalone use:
     ```bash
     # Integrated dtnpm (default includes cla-mercury)
     cargo build --release

     # Or standalone CLA binary
     cargo build --release --manifest-path ../dtnpm-cla-mercury-arq/Cargo.toml
     ```
3. **Valid Amateur Radio Callsign**:
   - Callsigns must be `3..=10` ASCII alphanumeric characters (containing at least one letter) with an optional `-SSID` (`0..=15`), e.g. `F4JXQ` or `F4JXQ-2`.
   - Callsigns are automatically trimmed and normalized to uppercase.
   - Blank or `N0CALL` callsigns emit a warning by default and are intended only for lab/bench testing. You can enforce strict identification with `require-callsign: true` (standalone config) or `--require-callsign` (CLI).

---

## 3. Configuration (Integrated Mode with `dtnpm`)

Add a `mercury-arq` entry under `clas` in your `dtnpm` configuration file (default: `~/.config/dtnpm/config.yaml` or `~/.config/dtnpm/config.toml`).

### 3.1 Configuration Reference

| Key (`kebab-case`) | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | String | *(required)* | Unique CLA instance name referenced by routing rules. |
| `type` | String | *(required)* | `"mercury-arq"` (alias `"mercury"`). |
| `my-callsign` | String | Root `callsign` | Local station callsign; inherits root `callsign` if omitted. |
| `control-addr` | SocketAddr | `127.0.0.1:8300` | Mercury TNC TCP control socket address. |
| `data-addr` | SocketAddr | `127.0.0.1:8301` | Mercury TNC TCP ARQ data socket address. |
| `bandwidth` | Integer | `2300` | ARQ bandwidth in Hz (e.g. `500`, `2300`, `2750`). |
| `connect-timeout` | Duration | `"30s"` | Timeout for `CONNECT` handshake before resetting to `Disconnected`. |
| `grace-period` | Duration | `"30s"` | Drain period after a scheduled contact window ends for in-flight bundles. |
| `idle-timeout` | Duration | `null` (disabled) | Optional inactivity timeout to close an idle ARQ session. |
| `peers` | List | `[]` | Known remote stations and optional contact schedules. |

Each entry in `peers` supports:

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `callsign` | String | *(required)* | Remote station amateur radio callsign (e.g. `"VA3NB"`). |
| `node-id` | String | `dtn://<callsign>/` | Remote BPv7 Node EID (lowercased callsign by default). |
| `schedule` | Object | `null` | Optional contact schedule (`recurring` or `one-shot`) to dial automatically. |

### 3.2 YAML Example (`~/.config/dtnpm/config.yaml`)

```yaml
callsign: "F4JXQ-2"
node-id: "dtn://f4jxq-2/"

storage:
  in-memory: false
  data-dir: "~/.local/share/dtnpm"

grpc:
  enabled: true
  address: "127.0.0.1:50051"

web:
  enabled: true
  address: "127.0.0.1:8088"

clas:
  - name: "mercury-hf"
    type: "mercury-arq"
    control-addr: "127.0.0.1:8300"
    data-addr: "127.0.0.1:8301"
    bandwidth: 2300
    connect-timeout: "30s"
    grace-period: "30s"
    idle-timeout: "60s"
    peers:
      # Recurring HF contact window every 30 minutes for 5 minutes
      - callsign: "VA3NB"
        schedule:
          type: "recurring"
          cron: "0 */30 * * * *"
          duration: "5m"

      # Passive/inbound peer (no outbound schedule; accepts incoming calls)
      - callsign: "VE3YCA"

routes:
  - pattern: "dtn://va3nb/*"
    cla: "mercury-hf"
    next-hop: "dtn://va3nb/"
  - pattern: "dtn://ve3yca/*"
    cla: "mercury-hf"
    next-hop: "dtn://ve3yca/"
```

### 3.3 TOML Example (`~/.config/dtnpm/config.toml`)

```toml
callsign = "F4JXQ-2"
node-id = "dtn://f4jxq-2/"

[[clas]]
name = "mercury-hf"
type = "mercury-arq"
control-addr = "127.0.0.1:8300"
data-addr = "127.0.0.1:8301"
bandwidth = 2300
connect-timeout = "30s"
grace-period = "30s"
idle-timeout = "60s"

[[clas.peers]]
callsign = "VA3NB"

[clas.peers.schedule]
type = "recurring"
cron = "0 */30 * * * *"
duration = "5m"

[[routes]]
pattern = "dtn://va3nb/*"
cla = "mercury-hf"
next-hop = "dtn://va3nb/"
```

> [!NOTE]
> Contact schedules also support `type: "one-shot"` with ISO-8601 `start` and `end` timestamps for single planned passes or scheduled nets.

---

## 4. Configuration (Standalone Mode with `dtnpm-cla-mercury-arq`)

When running the CLA separately from `dtnpm` (for example, when the Mercury modem is on a dedicated radio host), create a standalone CLA configuration file (e.g. `mercury.yaml`):

```yaml
my-callsign: "F4JXQ-2"
require-callsign: true
control-addr: "127.0.0.1:8300"
data-addr: "127.0.0.1:8301"
bandwidth: 2300
connect-timeout: "30s"
grace-period: "30s"
idle-timeout: "60s"
peers:
  - callsign: "VA3NB"
  - callsign: "VE3YCA"
    node-id: "dtn://ve3yca/"
```

Start the standalone CLA and register it with your BPA node:

```bash
dtnpm-cla-mercury-arq \
  --bpa-grpc http://127.0.0.1:50051 \
  --name mercury-hf \
  --config mercury.yaml \
  --require-callsign
```

For authenticated gRPC endpoints or Unix Domain Sockets, pass:
- `--token-file /path/to/token` (or `DTN_HDY_TOKEN_FILE`)
- `--socket /run/dtnpm/dtnpm.sock` (or `DTN_HDY_SOCKET`)

For lab/bench testing without a real callsign (`N0CALL`), pass `--allow-unidentified-rf`.

---

## 5. Step-by-Step Operation

### Step 1 — Start the Mercury Modem
Start your Mercury TNC first so that its control (`8300`) and data (`8301`) TCP ports are listening before (or while) `dtnpm` connects.

### Step 2 — Start DTN Postmaster
```bash
dtnpm -c ~/.config/dtnpm/config.yaml
```
On startup, the Mercury CLA driver:
1. Connects to `control-addr` and `data-addr`.
2. Sends `MYCALL <my-callsign>`, `BW<bandwidth>`, and `LISTEN ON` to the control socket.
3. Enters the `Disconnected` idle state, ready to accept incoming RF ARQ calls or execute scheduled outbound contacts.

### Step 3 — Verify Status in the Web Dashboard
Open the DTN Postmaster Web Dashboard (e.g. `http://127.0.0.1:8088`) and check **Station -> Liaisons & Radio**:
- Confirm the `mercury-hf` CLA is listed with its configured control/data ports and peers.
- During an active ARQ session, the peer status transitions to **connected** and queued bundles for that peer are forwarded automatically.

### Step 4 — Send and Receive Bundles
Once a contact is active (either scheduled or initiated by the remote station):

```bash
# Send a text bundle to VA3NB over Mercury ARQ
dtnsend -r "dtn://va3nb/chat" "CQ VA3NB de F4JXQ-2 over Mercury HF ARQ"

# Or ping the remote station's echo service
dtnping dtn://va3nb/echo -c 3
```

Bundles queued while the link is down remain safely stored in the BPA until the next contact window opens and `Sink::add_peer` advertises the active route.

---

## 6. Local Bench Simulation (Without RF Hardware)

The `dtnpm-cla-mercury-arq/examples/` directory includes a dual-station audio-bridge testbed for simulating two Mercury modems (`STATA` and `STATB`) over an 8 kHz FIFO audio bridge:

- `launch_dual_mercury.sh`: Launches two local Mercury instances (`8300/8301` and `8400/8401`) linked by `simplex_audio_bridge.py` (`--simplex` or `--duplex`).
- `stata-mercury.yaml` / `statb-mercury.yaml`: Sample `dtnpm` configurations for `STATA` and `STATB` with a recurring contact schedule.

---

## 7. Operational Limits & Troubleshooting

| Symptom / Behavior | Cause & Resolution |
| --- | --- |
| **Driver fails to connect to `127.0.0.1:8300`** | Ensure the Mercury TNC is running and listening on the configured `control-addr` and `data-addr`. The driver retries with exponential backoff. |
| **`"WRONG"` control line / stuck `Connecting` state** | If the modem rejects a command or a remote station does not answer within `connect-timeout` (default `30s`), the FSM automatically resets to `Disconnected` so subsequent contacts are not blocked. |
| **Bundle rejected with `PayloadTooLarge`** | BPv7 bundles exceeding **16 MiB** (`MAX_MERCURY_FRAME`) are rejected before buffering. Keep HF payloads compact for ARQ efficiency. |
| **Bundle not forwarded (`NoNeighbour`)** | The target `ClaAddress` callsign must match the currently connected peer's callsign. Verify your `routes` `next-hop` and `peers` callsign mapping. |
| **In-flight bundle cut off at window end** | Increase `grace-period` (default `30s`) so the contact coordinator waits longer before issuing `DISCONNECT`. |
| **Dynamic peer limit** | Up to **256** dynamic peer callsign entries (`MAX_PEERS`) are retained in memory per CLA instance. |
