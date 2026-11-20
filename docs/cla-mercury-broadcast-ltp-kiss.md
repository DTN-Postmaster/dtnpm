# LTP CLA over Mercury Broadcast KISS Mode — Setup and Operations Guide

This guide explains step by step how to configure, deploy, and operate the **Licklider Transmission Protocol (LTP, RFC 5326)** Convergence Layer Adapter (`dtnpm-cla-ltp-kiss`) over a **Mercury HF modem** operating in **Broadcast KISS TCP mode** with **DTN Postmaster** (`dtnpm`).

---

## 1. Architecture & Why LTP over Mercury Broadcast KISS

In Broadcast KISS mode, the Mercury HF modem exposes a **KISS over TCP** interface (typically port `8100`) instead of its connected ARQ sockets (`8300/8301`). Because Mercury's broadcast mode does not perform link-layer ARQ, **LTP (RFC 5326)** provides reliability, segmentation, and checkpoint-based acknowledgments (`Report Segment` / `Report Acknowledgment`) directly at the convergence layer.

Key capabilities of `dtnpm-cla-ltp-kiss` for HF simplex/half-duplex channels:
- **KISS-over-TCP Transport (`kiss-tcp`)**: Connects to Mercury's broadcast KISS TCP port with automatic exponential-backoff reconnection (`reconnect-initial-delay` / `reconnect-max-delay`).
- **Half-Duplex Turnaround Pacing (`half-duplex: true`)**: Prevents TX/RX collisions on simplex HF channels by pausing outbound transmission after emitting an LTP Checkpoint until either a `Report Segment` arrives (`wait-for-report`) or a fixed quiet interval elapses (`fixed-dwell`).
- **Leaky-Bucket Rate Limiting (`rate-limit-bytes-per-sec`)**: Shapes the aggregate KISS stream to match the HF modem's air-interface throughput so the TNC buffer is not overrun.
- **Minimal KISS Framing (`min-frame-len: 0`)**: Preserves compact frame sizes without unnecessary padding (unlike AX.25 TNCs such as Direwolf that may require `min-frame-len: 15`).

### Deployment Modes

- **Integrated Mode (recommended)**: Built directly into `dtnpm` when compiled with the `cla-ltp` Cargo feature (included in `desktop`, `amsat`, and `node-headless` profiles).
- **Standalone Mode**: Run `dtnpm-cla-ltp-kiss` as a separate process connecting to a `dtnpm` / Hardy BPA node over gRPC or a Unix Domain Socket.

---

## 2. Prerequisites

1. **Mercury HF Modem in Broadcast KISS Mode**:
   - Configured with its KISS TCP broadcast port listening (commonly `127.0.0.1:8100`).
   - Connected to your HF radio audio/PTT interface (or the local 8 kHz audio-bridge simulator for bench testing).
2. **DTN Postmaster (`dtnpm`)**:
   - Built with `cla-ltp` enabled (default), or `dtnpm-cla-ltp-kiss` built for standalone execution:
     ```bash
     # Integrated dtnpm (default includes cla-ltp)
     cargo build --release

     # Or standalone CLA binary
     cargo build --release --manifest-path ../dtnpm-cla-ltp-kiss/Cargo.toml
     ```
3. **Unique LTP Engine IDs**:
   - Each station must have a distinct numeric `engine-id` (e.g., `1` for Station A, `2` for Station B).

---

## 3. Configuration (Integrated Mode with `dtnpm`)

Add an `ltp` CLA entry under `clas` in your `dtnpm` configuration file (default: `~/.config/dtnpm/config.yaml` or `~/.config/dtnpm/config.toml`).

### 3.1 LTP CLA Configuration Reference

| Key (`kebab-case`) | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | String | *(required)* | Unique CLA instance name referenced by routing rules. |
| `type` | String | *(required)* | Must be `"ltp"`. |
| `engine-id` | Integer | `1` | Local LTP Engine ID (must be unique per station). |
| `max-segment-payload-len` | Integer | `1400` | Maximum LTP segment payload size in bytes. Use smaller values (e.g. `80`–`256`) for noisy HF channels. |
| `rtt` | Duration | `"500ms"` | Estimated round-trip time (e.g. `"3s"` for HF turnaround). |
| `ack-margin` | Duration | `"250ms"` | Extra margin added to retransmission timers (e.g. `"2s"` for HF modem latency). |
| `checkpoint-rtx-limit` | Integer | `10` | Maximum Checkpoint retransmissions before cancelling a session. |
| `report-rtx-limit` | Integer | `10` | Maximum Report Segment retransmissions before cancelling a session. |
| `peers` | List | `[]` | Remote LTP peers (`engine-id`, `node-id`, optional `rtt` / `ack-margin`). |

### 3.2 `transport` (`type: "kiss-tcp"`) Reference

| Key (`kebab-case`) | Type | Default | Description |
| --- | --- | --- | --- |
| `type` | String | *(required)* | Must be `"kiss-tcp"` for Mercury Broadcast KISS. |
| `server-addr` | SocketAddr | *(required)* | Mercury broadcast KISS TCP address (e.g. `"127.0.0.1:8100"`). |
| `bind-addr` | SocketAddr | `null` | Optional local source address to bind for the outbound TCP connection. |
| `port` | Integer | `0` | KISS port nibble (`0..15`). |
| `reconnect-initial-delay` | Duration | `"500ms"` | Initial delay before reconnecting after TCP loss. |
| `reconnect-max-delay` | Duration | `"30s"` | Maximum exponential-backoff reconnect delay. |
| `min-frame-len` | Integer | `0` | Minimum KISS frame payload length in bytes (`0` for Mercury). |
| `rate-limit-bytes-per-sec` | Integer | `null` | Optional leaky-bucket rate limit in bytes/s (e.g. `1200`). |
| `burst-capacity-bytes` | Integer | `2048` | Maximum burst capacity in bytes when rate limiting is enabled. |
| `half-duplex` | Boolean | `false` | Set `true` on simplex HF channels to enable Checkpoint turnaround pacing. |
| `turnaround-mode` | String | `"wait-for-report"` | `"wait-for-report"` (wait for RS up to `turnaround-timeout`) or `"fixed-dwell"`. |
| `turnaround-timeout` | Duration | `"3s"` | Max wait for a Report Segment before unblocking in `wait-for-report` mode. |
| `dwell-time` | Duration | `"500ms"` | Quiet duration after each Checkpoint in `fixed-dwell` mode. |

### 3.3 YAML Example (`~/.config/dtnpm/config.yaml`)

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
  - name: "ltp-mercury-hf"
    type: "ltp"
    engine-id: 1
    max-segment-payload-len: 80
    rtt: "3s"
    ack-margin: "2s"
    checkpoint-rtx-limit: 10
    report-rtx-limit: 10
    transport:
      type: "kiss-tcp"
      server-addr: "127.0.0.1:8100"
      port: 0
      min-frame-len: 0
      half-duplex: true
      turnaround-mode: "wait-for-report"
      turnaround-timeout: "4s"
      rate-limit-bytes-per-sec: 1200
      burst-capacity-bytes: 1024
    peers:
      - engine-id: 2
        node-id: "dtn://va3nb/"

routes:
  - pattern: "dtn://va3nb/*"
    cla: "ltp-mercury-hf"
    next-hop: "dtn://va3nb/"
```

### 3.4 TOML Example (`~/.config/dtnpm/config.toml`)

```toml
callsign = "F4JXQ-2"
node-id = "dtn://f4jxq-2/"

[[clas]]
name = "ltp-mercury-hf"
type = "ltp"
engine-id = 1
max-segment-payload-len = 80
rtt = "3s"
ack-margin = "2s"

[clas.transport]
type = "kiss-tcp"
server-addr = "127.0.0.1:8100"
port = 0
min-frame-len = 0
half-duplex = true
turnaround-mode = "wait-for-report"
turnaround-timeout = "4s"
rate-limit-bytes-per-sec = 1200
burst-capacity-bytes = 1024

[[clas.peers]]
engine-id = 2
node-id = "dtn://va3nb/"

[[routes]]
pattern = "dtn://va3nb/*"
cla = "ltp-mercury-hf"
next-hop = "dtn://va3nb/"
```

---

## 4. Configuration (Standalone Mode with `dtnpm-cla-ltp-kiss`)

If you run the LTP/KISS CLA as a standalone daemon connecting to `dtnpm` over gRPC, create a CLA config file (e.g. `ltp-mercury.yaml`):

```yaml
engine-id: 1
max-segment-payload-len: 80
rtt: "3s"
ack-margin: "2s"
transport:
  type: "kiss-tcp"
  server-addr: "127.0.0.1:8100"
  half-duplex: true
  turnaround-mode: "wait-for-report"
  turnaround-timeout: "4s"
  rate-limit-bytes-per-sec: 1200
  burst-capacity-bytes: 1024
peers:
  - engine-id: 2
    node-id: "dtn://va3nb/"
```

Launch `dtnpm-cla-ltp-kiss`:

```bash
dtnpm-cla-ltp-kiss \
  --bpa-grpc http://127.0.0.1:50051 \
  --name ltp-mercury-hf \
  --config ltp-mercury.yaml
```

For authenticated gRPC endpoints or Unix Domain Sockets, pass `--token-file /path/to/token` (`DTN_HDY_TOKEN_FILE`) or `--socket /run/dtnpm/dtnpm.sock` (`DTN_HDY_SOCKET`).

---

## 5. Step-by-Step Operation

### Step 1 — Start the Mercury Modem in Broadcast KISS Mode
Ensure the Mercury modem is running and its broadcast KISS TCP port (`8100`) is open before or while `dtnpm` starts.

### Step 2 — Start DTN Postmaster
```bash
dtnpm -c ~/.config/dtnpm/config.yaml
```
`dtnpm` connects to `server-addr` (`127.0.0.1:8100`), registers all configured `peers` with the BPA, and begins listening for incoming KISS-framed LTP segments.

### Step 3 — Verify Status in the Web Dashboard
Open the DTN Postmaster Web Dashboard (`http://127.0.0.1:8088`) and navigate to **Station -> Liaisons & Radio**:
- Verify `ltp-mercury-hf` is listed with its KISS TCP transport and peer `engine-id` / `node-id` mappings.

### Step 4 — Send and Verify Bundles
```bash
# Send a bundle over LTP / Mercury KISS
dtnsend -r "dtn://va3nb/echo" "DTN Postmaster LTP over Mercury KISS test"

# Or run BPv7 ping across the HF link
dtnping dtn://va3nb/echo -c 3 -i 5s -w 20s
```

During transmission, the CLA segments the bundle into `max-segment-payload-len`-byte Red LTP data segments, emits a Checkpoint, and pauses (when `half-duplex: true`) until the peer's `Report Segment` is received or `turnaround-timeout` expires.

---

## 6. Local Bench Simulation (Without RF Hardware)

You can test end-to-end simplex LTP transfers locally using the scripts in `dtnpm-cla-ltp-kiss/examples/`:
- `stata-ltp.yaml` / `statb-ltp.yaml`: Two preconfigured stations (`Engine ID 1` on KISS port `8100` and `Engine ID 2` on KISS port `8500`) with half-duplex turnaround pacing and `1200 B/s` rate limiting.
- `test_simplex_mercury_ltp.sh`: Launches the dual Mercury simulator in `--simplex` mode over `simplex_audio_bridge.py`, starts both BPA nodes, and transfers a test bundle end-to-end.

---

## 7. Tuning & Troubleshooting

| Symptom / Behavior | Cause & Resolution |
| --- | --- |
| **Frequent retransmissions / timeouts on HF** | Increase `rtt` (e.g. `"3s"`–`"5s"`) and `ack-margin` (e.g. `"2s"`–`"4s"`), and reduce `max-segment-payload-len` (e.g. `80`–`160` bytes) to improve per-frame decode probability. |
| **TX/RX collisions on simplex channel** | Ensure `half-duplex: true` and `turnaround-mode: "wait-for-report"` are set, with `turnaround-timeout` slightly longer than the modem's PTT + turnaround delay (`"4s"`). |
| **TNC buffer overrun / dropped KISS frames** | Enable `rate-limit-bytes-per-sec` (e.g. `1200`) and keep `burst-capacity-bytes` small (`1024`). |
| **KISS TCP connection refused** | Confirm Mercury is started in broadcast KISS mode and listening on `server-addr` (e.g. `127.0.0.1:8100`). The CLA retries automatically with exponential backoff up to `reconnect-max-delay`. |
| **Green reassembly / session limits** | Receive sessions are bounded at **1,024** concurrent slots and Green reassembly is capped at **32 MiB** per bundle. |
