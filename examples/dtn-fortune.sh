#!/bin/bash
# DTN fortune trigger script for dtntrigger
# Argument 1: source EID
# Argument 2: path to temp file containing payload

set -e

SOURCE_EID="$1"
PAYLOAD_FILE="$2"

# Determine script directory to locate fortune databases
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"

# Resolve dtnsend binary location
DTNSEND_BIN=""
if command -v dtnsend >/dev/null 2>&1; then
    DTNSEND_BIN="dtnsend"
elif [ -x "${SCRIPT_DIR}/dtnsend" ]; then
    DTNSEND_BIN="${SCRIPT_DIR}/dtnsend"
elif [ -x "${SCRIPT_DIR}/../target/debug/dtnsend" ]; then
    DTNSEND_BIN="${SCRIPT_DIR}/../target/debug/dtnsend"
elif [ -x "${SCRIPT_DIR}/../target/release/dtnsend" ]; then
    DTNSEND_BIN="${SCRIPT_DIR}/../target/release/dtnsend"
else
    # Fallback to cargo run
    DTNSEND_BIN="cargo run --bin dtnsend --"
fi

# Read language preference from payload file
LANG_VAL=""
if [ -f "$PAYLOAD_FILE" ]; then
    # Clean whitespace and convert to lowercase
    LANG_VAL=$(tr -d '[:space:]' < "$PAYLOAD_FILE" | tr '[:upper:]' '[:lower:]')
fi

# Select the appropriate database
if [ "$LANG_VAL" = "fr" ] || [ "$LANG_VAL" = "french" ]; then
    FORTUNE_DB="${SCRIPT_DIR}/dtn_fortunes_fr"
else
    FORTUNE_DB="${SCRIPT_DIR}/dtn_fortunes"
fi

# Ensure the database has been indexed
if [ ! -f "${FORTUNE_DB}.dat" ]; then
    strfile "${FORTUNE_DB}" > /dev/null
fi

# Generate fortune
FORTUNE_MSG=$(fortune "${FORTUNE_DB}")

# Send the response back to the sender
# Using -s to specify sender name, -r to specify receiver EID,
# and DTN_RETURN_PATH / DTN_GRPC_PORT exported by dtntrigger for route-free replies
DTNSEND_ARGS=(-s fortune -r "$SOURCE_EID")
if [ -n "${DTN_GRPC_PORT:-}" ]; then
    DTNSEND_ARGS+=(-p "$DTN_GRPC_PORT")
fi
if [ -n "${DTN_RETURN_PATH:-}" ]; then
    DTNSEND_ARGS+=(--aprs-path "$DTN_RETURN_PATH")
fi

if [[ "$DTNSEND_BIN" == "cargo run"* ]]; then
    # Run cargo command from the repository root
    (cd "${SCRIPT_DIR}/.." && echo "$FORTUNE_MSG" | $DTNSEND_BIN "${DTNSEND_ARGS[@]}")
else
    echo "$FORTUNE_MSG" | "$DTNSEND_BIN" "${DTNSEND_ARGS[@]}"
fi
