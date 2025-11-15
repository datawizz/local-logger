# Local Logger

A multi-purpose logging and proxy tool that serves three purposes:
1. **MCP (Model Context Protocol) server** for logging operations
2. **Claude Code hook processor** for logging tool usage events
3. **HTTPS MITM proxy** for recording Claude API traffic

All logs are stored in newline-delimited JSON (NDJSON) format with automatic daily rotation.

## Features

- **Unified Log Format**: MCP, hook, and proxy logs use the same NDJSON structure
- **Daily Log Rotation**: Automatic organization by date (YYYY-MM-DD.jsonl)
- **Structured Data**: JSON format enables easy parsing and analysis
- **HTTPS Interception**: MITM proxy with automatic TLS certificate generation
- **Full Request/Response Recording**: Captures complete HTTP traffic including headers and bodies
- **Rich Metadata**: Includes timestamps, source type, session IDs, tool names, and proxy data
- **Configurable**: Uses environment variables or TOML configuration file
- **Log Directory**: Uses `CLAUDE_MCP_LOCAL_LOGGER_DIR` or defaults to `~/.local-logger`

## Nix Module

This tool includes a NixOS/Darwin module that automatically:
- Builds and installs the `local-logger` binary
- Configures Claude Code hooks and MCP server settings
- Starts the HTTPS proxy service (launchd on Darwin, systemd on Linux)
- Sets proxy environment variables (`HTTP_PROXY`, `HTTPS_PROXY`)
- Manages log directories and permissions

To use, import the module in your configuration:

```nix
{
  imports = [ ./path/to/dotfiles/src/nix/modules/developer/claude.nix ];

  modules.developer.claude.enable = true;
}
```

This provides a complete, declarative setup for Claude Code logging and proxy functionality.

## Installation

### macOS PKG Installer (Recommended for macOS)

The easiest way to install on macOS is using the `.pkg` installer:

1. Download the latest `.pkg` from the [releases page](https://github.com/yourusername/local-logger/releases)
2. Double-click the installer and follow the prompts
3. Enter your password when requested

The installer will:
- Install the binary to `/usr/local/bin/local-logger`
- Generate and trust TLS certificates for HTTPS interception
- Set up a LaunchAgent to auto-start the proxy
- Install an uninstaller at `/usr/local/bin/local-logger-uninstall.sh`

See [macOS PKG Installation Guide](docs/installation/macos-pkg.md) for detailed instructions.

### From Source

```bash
# Using Make (recommended)
make release        # Build release binary
make install        # Install to ~/.cargo/bin
make init           # Initialize certificates
make pkg            # Build macOS PKG installer

# Or using Cargo directly
cargo build --release
cargo install --path .
local-logger init

# See all available commands
make help
```

### Nix/NixOS

This repository includes a Nix module for declarative installation. See [Nix Module](#nix-module) section above.

## Usage

### MCP Server Mode (Default)

Run as an MCP server that provides logging tools:

```bash
# Run with default settings
local-logger

# Or explicitly specify server mode
local-logger serve
```

### Claude Code Hook Mode

Process Claude Code hook events from stdin:

```bash
# Hook mode automatically logs to today's file
echo '{"hook_event_name":"PreToolUse",...}' | local-logger hook
```

The hook mode flexibly accepts any valid JSON input. While it expects certain fields for Claude Code hooks (like `hook_event_name`, `tool_name`, `session_id`), it will gracefully handle missing fields and preserve any additional data sent by Claude Code.

### Proxy Mode

Run as an HTTPS MITM proxy to intercept and record Claude API traffic:

```bash
# Run with default configuration
local-logger proxy

# Run with custom port
local-logger proxy --port 9090

# Run with configuration file
local-logger proxy --config proxy-config.toml
```

Then configure your environment to use the proxy:

```bash
export HTTPS_PROXY=http://127.0.0.1:6969
export HTTP_PROXY=http://127.0.0.1:6969

# Now run Claude Code - all traffic will be logged
claude "Help me with this code"
```

### Certificate Initialization

Initialize TLS certificates for HTTPS interception:

```bash
# Generate certificates with default settings
local-logger init

# Force regenerate even if they exist
local-logger init --force

# Use a custom certificate directory
local-logger init --cert-dir /custom/path

# Quiet mode (for scripts)
local-logger init --quiet
```

The `init` command:
- Creates `~/.local-logger/certs/` directory (or custom path)
- Generates a self-signed CA certificate (`ca.pem`)
- Generates a private key (`ca.key`)
- Provides instructions for trusting the certificate
- Is idempotent (safe to run multiple times)

**Note:** The PKG installer automatically runs `init` and trusts the certificate during installation.

### Claude Code Configuration Management

Install or uninstall local-logger from Claude Code configuration automatically:

```bash
# Install local-logger into Claude Code
local-logger install-claude

# Uninstall local-logger from Claude Code
local-logger uninstall-claude

# Quiet mode (for scripts)
local-logger install-claude --quiet
local-logger uninstall-claude --quiet
```

The `install-claude` command:
- Adds local-logger MCP server to `~/.claude.json`
- Adds hooks to `~/.claude/settings.json` for all hook types (PreToolUse, PostToolUse, etc.)
- Creates files and directories if they don't exist
- Preserves all existing configuration
- Is idempotent (safe to run multiple times)

The `uninstall-claude` command:
- Surgically removes only local-logger MCP server entry from `~/.claude.json`
- Surgically removes only local-logger hooks from `~/.claude/settings.json`
- Preserves all other hooks and configuration
- Cleans up empty hook type arrays
- Is idempotent (safe to run multiple times)

**Note:** The PKG installer and Nix modules automatically run `install-claude` during installation.

#### Certificate Installation

On first run, the proxy will generate a root CA certificate. You need to trust this certificate:

**macOS:**
```bash
sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain ~/.local-logger/certs/ca.pem
```

**Linux:**
```bash
sudo cp ~/.local-logger/certs/ca.pem /usr/local/share/ca-certificates/local-logger.crt
sudo update-ca-certificates
```

#### Proxy Configuration

Create a `proxy-config.toml` file:

```toml
[proxy]
listen_addr = "127.0.0.1"
listen_port = 6969

[tls]
cert_dir = "/Users/you/.local-logger/certs"
generate_ca = true

[recording]
output_dir = "/Users/you/.local-logger"
pretty_print = true
include_bodies = true
max_body_size = 10485760  # 10MB

[filtering]
target_hosts = ["api.anthropic.com"]
capture_patterns = []
```

Or use environment variables:

```bash
export CLAUDE_LOGGER_PROXY_PORT=6969
export CLAUDE_LOGGER_PROXY_ADDR=127.0.0.1
export CLAUDE_LOGGER_PROXY_CERT_DIR=$HOME/.local-logger/certs
export CLAUDE_MCP_LOCAL_LOGGER_DIR=$HOME/.local-logger
```

## Claude Code Integration

### Automatic Configuration

The easiest way to configure Claude Code is using the built-in command:

```bash
local-logger install-claude
```

This automatically adds the MCP server and hooks to your Claude Code configuration files.

### Manual Configuration (Alternative)

The Nix module and PKG installer automatically configure Claude Code. If you need to configure manually, add to your `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [{
      "matcher": "",
      "hooks": [{"type": "command", "command": "local-logger hook"}]
    }],
    "PostToolUse": [{
      "matcher": "",
      "hooks": [{"type": "command", "command": "local-logger hook"}]
    }]
  }
}
```

And to `~/.claude.json`:

```json
{
  "mcpServers": {
    "local-logger": {
      "command": "local-logger",
      "args": ["serve"]
    }
  }
}
```

All hook events will be logged to the daily log file (e.g., `2025-10-03.jsonl`).

### Removing Configuration

To remove local-logger from Claude Code:

```bash
local-logger uninstall-claude
```

This surgically removes only local-logger entries while preserving all other configuration.

## Log Format

Each log file contains one JSON object per line (NDJSON format):

```json
{"timestamp":"2025-10-03T15:30:45Z","date":"2025-10-03","source":{"type":"Mcp"},"level":"INFO","message":"User logged in","session_id":null,"tool_name":null,"hook_event":null,"proxy_event":null}
{"timestamp":"2025-10-03T15:31:02Z","date":"2025-10-03","source":{"type":"Hook","event_type":"PreToolUse"},"level":"HOOK","message":null,"session_id":"abc123","tool_name":"Bash","hook_event":{"hook_event_name":"PreToolUse","tool_name":"Bash","session_id":"abc123","tool_input":{"command":"ls -la"}},"proxy_event":null}
{"timestamp":"2025-10-03T15:32:15Z","date":"2025-10-03","source":{"type":"Proxy","session_id":"uuid-here","direction":"request"},"level":"PROXY","message":"POST https://api.anthropic.com/v1/messages","session_id":"uuid-here","tool_name":null,"hook_event":null,"proxy_event":{"method":"POST","uri":"https://api.anthropic.com/v1/messages","headers":{...},"body":"..."}}
```

### Log Entry Fields

- `timestamp`: ISO 8601 timestamp in UTC
- `date`: Date in YYYY-MM-DD format (matches filename)
- `source`: One of:
  - `{"type":"Mcp"}` - MCP server log
  - `{"type":"Hook","event_type":"<event_name>"}` - Hook event
  - `{"type":"Proxy","session_id":"<uuid>","direction":"request|response"}` - Proxy traffic
- `level`: Log level (INFO, ERROR, WARN for MCP; HOOK for hooks; PROXY for proxy)
- `message`: Log message
- `session_id`: Session identifier
- `tool_name`: Name of the tool being invoked (hooks only)
- `hook_event`: Complete hook event data (hooks only)
- `proxy_event`: Complete proxy request/response data (proxy only)

## MCP Tools Available

When running in MCP server mode, the following tools are available:

### write_log
Write a log message to today's log file.
- Parameters:
  - `message` (required): The log message
  - `level` (optional): Log level (default: INFO)

### read_logs
Read log entries from a specific date.
- Parameters:
  - `date` (optional): Date in YYYY-MM-DD format (default: today)
  - `lines` (optional): Number of recent entries to show (default: 50)

### list_log_files
List all available daily log files with entry counts.

### clear_log
Clear all entries from a specific date's log file.
- Parameters:
  - `date` (required): Date in YYYY-MM-DD format

## Working with NDJSON Logs

The NDJSON format makes it easy to process logs with standard tools:

```bash
# View today's logs with jq
cat ~/.local-logger/2025-10-03.jsonl | jq .

# Filter for hook events
cat *.jsonl | jq 'select(.source.type == "Hook")'

# Filter for proxy events
cat *.jsonl | jq 'select(.source.type == "Proxy")'

# View all API requests
cat *.jsonl | jq 'select(.source.type == "Proxy" and .source.direction == "request")'

# Count events by tool name
cat *.jsonl | jq -r '.tool_name // empty' | sort | uniq -c

# Find all errors
cat *.jsonl | jq 'select(.level == "ERROR")'

# Extract API request bodies
cat *.jsonl | jq 'select(.source.type == "Proxy") | .proxy_event.body' -r

# Convert to CSV
cat *.jsonl | jq -r '[.timestamp, .level, .message // .tool_name] | @csv'
```

## Environment Variables

- `CLAUDE_MCP_LOCAL_LOGGER_DIR`: Custom directory for log files (default: `~/.local-logger`)
- `CLAUDE_LOGGER_PROXY_PORT`: Proxy listen port (default: 6969)
- `CLAUDE_LOGGER_PROXY_ADDR`: Proxy listen address (default: 127.0.0.1)
- `CLAUDE_LOGGER_PROXY_CERT_DIR`: Certificate directory (default: `~/.local-logger/certs`)

## Development

The project uses:
- Unified log entry structure for consistency across all modes
- Automatic date-based file naming
- Asynchronous I/O for performance
- TLS certificate generation with rcgen
- hyper and tokio for HTTP/HTTPS proxy
- Strict input validation for security

Key modules:
- `main.rs`: CLI and runtime management
- `certificate_manager.rs`: TLS certificate generation and caching
- `proxy_server.rs`: HTTPS MITM proxy implementation
- `proxy_config.rs`: Configuration management
- `UnifiedLogEntry`: Common structure for all logs
- `LogSource`: Enum distinguishing MCP, Hook, and Proxy logs

## Security Considerations

### Certificate Management
- Root CA is generated per installation
- Private keys are stored with restrictive permissions (chmod 600)
- Certificates are cached per hostname
- Never commit CA keys to version control

### Data Privacy
- API keys and tokens are visible in proxy logs
- Consider the security of log storage
- Be cautious with log sharing

### Network Security
- Proxy binds to localhost (127.0.0.1) by default
- Target hosts can be filtered via configuration
- Only intercepts configured hosts by default (api.anthropic.com)

## Troubleshooting

### Certificate Trust Issues
If you see SSL errors, ensure the root CA is properly installed in your system trust store.

### Proxy Not Intercepting Traffic
Verify that:
1. The proxy is running and listening on the correct port
2. Environment variables `HTTP_PROXY` and `HTTPS_PROXY` are set
3. The application respects proxy environment variables

### Performance
Proxy mode may add latency due to:
- TLS decryption/re-encryption
- Request/response logging
- Body buffering

For better performance, disable body recording or reduce `max_body_size` in configuration.
