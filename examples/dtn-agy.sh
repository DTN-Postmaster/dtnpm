#!/bin/bash
# Desktop notification + AI responder trigger helper for dtntrigger
# Argument 1: source EID
# Argument 2: path to temp file containing payload
set -euo pipefail

SOURCE_EID="$1"
PAYLOAD_FILE="$2"
PAYLOAD_CONTENT=$(cat "$PAYLOAD_FILE")

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
DTNSEND_BIN="dtnsend"
if ! command -v dtnsend >/dev/null 2>&1; then
    for candidate in "${SCRIPT_DIR}/dtnsend" "${SCRIPT_DIR}/../target/release/dtnsend" "${SCRIPT_DIR}/../target/debug/dtnsend"; do
        if [ -x "$candidate" ]; then
            DTNSEND_BIN="$candidate"
            break
        fi
    done
fi

if command -v notify-send >/dev/null 2>&1; then
    notify-send -- "DTN Bundle from $SOURCE_EID" "$PAYLOAD_CONTENT" || true
fi

# Run the command and capture output
response=$(agy -p -- "${PAYLOAD_CONTENT}")

# Send the response back to the sender (preserving dtntrigger's DTN_RETURN_PATH / DTN_GRPC_PORT)
DTNSEND_ARGS=(-s agy -r "$SOURCE_EID")
if [ -n "${DTN_GRPC_PORT:-}" ]; then
    DTNSEND_ARGS+=(-p "$DTN_GRPC_PORT")
fi
if [ -n "${DTN_RETURN_PATH:-}" ]; then
    DTNSEND_ARGS+=(--aprs-path "$DTN_RETURN_PATH")
fi
printf '%s\n' "$response" | "$DTNSEND_BIN" "${DTNSEND_ARGS[@]}"
