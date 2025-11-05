#!/usr/bin/env bash
# Comprehensive test script for local-logger

set -euo pipefail

# Setup
TEMP_DIR=$(mktemp -d)
export CLAUDE_MCP_LOCAL_LOGGER_DIR="$TEMP_DIR"
trap "rm -rf $TEMP_DIR" EXIT

echo "=== Local Logger Tests ==="

# Run Rust tests
echo "Running cargo tests..."
cargo test

# Build binary
echo "Building release binary..."
cargo build --release

# Test hook mode
echo "Testing hook mode..."
echo '{"hook_event_name":"PreToolUse","tool_name":"Bash","session_id":"test-123","tool_input":{"command":"ls"}}' | ./target/release/local-logger hook

TODAY=$(date +%Y-%m-%d)
LOG_FILE="$TEMP_DIR/$TODAY.jsonl"

if [ -f "$LOG_FILE" ]; then
    echo "✓ Log file created: $LOG_FILE"
    
    # Verify JSON format
    if jq empty "$LOG_FILE" 2>/dev/null; then
        echo "✓ Valid NDJSON format"
    else
        echo "✗ Invalid JSON"
        exit 1
    fi
    
    # Check content
    if grep -q '"type":"Hook"' "$LOG_FILE" && grep -q '"tool_name":"Bash"' "$LOG_FILE"; then
        echo "✓ Hook event logged correctly"
    else
        echo "✗ Missing expected content"
        exit 1
    fi
else
    echo "✗ Log file not created"
    exit 1
fi

echo ""
echo "All tests passed!"