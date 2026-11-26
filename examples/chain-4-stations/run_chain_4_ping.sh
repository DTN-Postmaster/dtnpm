#!/usr/bin/env bash
# Runs 4 route-free DTN Postmaster stations in sequence:
#   STATA (35051) <-> STATB (35052) <-> STATC (35053) <-> STATD (35054)
# and verifies end-to-end ping replies from dtn://statd/echo using:
#   1. Spray-and-Wait (--spray-quota 8) + TREB traceback return path
#   2. APRS-like strict trajectory (--aprs-path "STATB,STATC") + TREB traceback return path
#   3. APRS-like strict trajectory (--aprs-path "STATB,STATC,STATD") + TREB traceback return path

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DTNPM_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

if [[ -n "${DTNPM_BIN:-}" && -n "${DTNPING_BIN:-}" ]]; then
    :
elif [[ -x "$DTNPM_DIR/target/release/dtnpm" && -x "$DTNPM_DIR/target/release/dtnping" && ( ! -x "$DTNPM_DIR/target/debug/dtnpm" || "$DTNPM_DIR/target/release/dtnpm" -nt "$DTNPM_DIR/target/debug/dtnpm" ) ]]; then
    DTNPM_BIN="$DTNPM_DIR/target/release/dtnpm"
    DTNPING_BIN="$DTNPM_DIR/target/release/dtnping"
elif [[ -x "$DTNPM_DIR/target/debug/dtnpm" && -x "$DTNPM_DIR/target/debug/dtnping" ]]; then
    DTNPM_BIN="$DTNPM_DIR/target/debug/dtnpm"
    DTNPING_BIN="$DTNPM_DIR/target/debug/dtnping"
else
    echo "Building dtnpm and dtnping (debug)..."
    cargo build --manifest-path "$DTNPM_DIR/Cargo.toml" --offline --bin dtnpm --bin dtnping
    DTNPM_BIN="$DTNPM_DIR/target/debug/dtnpm"
    DTNPING_BIN="$DTNPM_DIR/target/debug/dtnping"
fi

LOG_DIR="$(mktemp -d)"
PIDS=()

cleanup() {
    for pid in "${PIDS[@]:-}"; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
    rm -rf "$LOG_DIR"
}
trap cleanup EXIT

wait_for_port() {
    local port="$1"
    local label="$2"
    for _ in $(seq 1 60); do
        if (echo >/dev/tcp/127.0.0.1/"$port") 2>/dev/null; then
            return 0
        fi
        sleep 0.25
    done
    echo "ERROR: $label did not listen on 127.0.0.1:$port" >&2
    return 1
}

echo "Starting 4 route-free stations (STATD -> STATC -> STATB -> STATA)..."
for name in statd statc statb stata; do
    "$DTNPM_BIN" -c "$SCRIPT_DIR/${name}.yaml" >"$LOG_DIR/${name}.log" 2>&1 &
    PIDS+=($!)
done

wait_for_port 35054 "STATD gRPC"
wait_for_port 35053 "STATC gRPC"
wait_for_port 35052 "STATB gRPC"
wait_for_port 35051 "STATA gRPC"

# Allow TCPCLv4 handshakes along the 4-station chain to settle
sleep 2

echo "=== Test 1: Spray-and-Wait (--spray-quota 8) ==="
"$DTNPING_BIN" -p 35051 -c 1 -W 10s dtn://statd/echo --spray-quota 8

echo "=== Test 2: APRS-like (--aprs-path STATB,STATC) ==="
"$DTNPING_BIN" -p 35051 -c 1 -W 10s dtn://statd/echo --aprs-path "STATB,STATC"

echo "=== Test 3: APRS-like (--aprs-path STATB,STATC,STATD) ==="
"$DTNPING_BIN" -p 35051 -c 1 -W 10s dtn://statd/echo --aprs-path "STATB,STATC,STATD"

echo "All 4-station route-free ping tests passed."
