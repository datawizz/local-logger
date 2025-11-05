//! Local Logger - MCP server, Claude Code hook logger, and HTTPS proxy
//!
//! This tool serves three purposes:
//! 1. As an MCP (Model Context Protocol) server for logging operations
//! 2. As a Claude Code hook processor for logging tool usage events
//! 3. As an HTTPS MITM proxy for recording Claude API traffic
//!
//! All logs are stored in newline-delimited JSON (NDJSON) format with automatic
//! daily rotation. Each day's logs are stored in a file named YYYY-MM-DD.jsonl.
//!
//! ## Usage
//!
//! ### MCP Server Mode (default)
//! ```bash
//! local-logger serve
//! # or just
//! local-logger
//! ```
//!
//! ### Claude Code Hook Mode
//! ```bash
//! # In ~/.claude/settings.json:
//! "hooks": {
//!   "PreToolUse": [{"hooks": [{"type": "command", "command": "local-logger hook"}]}]
//! }
//! ```
//!
//! ### Proxy Mode
//! ```bash
//! local-logger proxy
//! # Then set environment variables:
//! export HTTPS_PROXY=http://127.0.0.1:6969
//! export HTTP_PROXY=http://127.0.0.1:6969
//! ```
//!
//! All modes write logs to the same unified daily log file.

mod certificate_manager;
mod jsonl_tracing_layer;
mod log_writer;
mod proxy_config;
mod proxy_server;
pub mod schema;
mod tail_reader;

use anyhow::Result;
use clap::{Parser, Subcommand};
use log_writer::LogWriter;
use proxy_config::ProxyConfig;
use proxy_server::ProxyServer;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schema::LogEntry;
use serde::Deserialize;
use std::{
    fs::{self, File},
    future::Future,
    io::{self, BufRead, BufReader, Read},
    path::PathBuf,
    sync::Arc,
};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "local-logger")]
#[command(about = "Local Logger - MCP server, hook logger, and HTTPS proxy", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run as MCP server (default)
    Serve,
    /// Process Claude Code hook JSON from stdin
    Hook,
    /// Run as HTTPS MITM proxy
    Proxy {
        /// Configuration file path
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Listen port (overrides config)
        #[arg(short, long)]
        port: Option<u16>,
    },
}

/// Hook event payload from stdin (for parsing only)
#[derive(Debug, Deserialize)]
struct HookEventInput {
    /// Event type: "PreToolUse", "PostToolUse", etc.
    hook_event_name: Option<String>,
    /// Name of the tool being invoked (e.g., "Bash", "Read", "Write")
    tool_name: Option<String>,
    /// Unique session identifier for the Claude Code session
    session_id: Option<String>,
    /// Tool input parameters (varies by tool)
    tool_input: Option<serde_json::Value>,
    /// Transcript path
    transcript_path: Option<String>,
    /// Current working directory
    cwd: Option<String>,
    /// Additional fields that may be present in the hook payload
    #[serde(flatten)]
    extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WriteLogRequest {
    pub message: String,
    pub level: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadLogsRequest {
    /// Date to read logs from (YYYY-MM-DD format), defaults to today
    pub date: Option<String>,
    pub lines: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ClearLogRequest {
    /// Date of the log to clear (YYYY-MM-DD format)
    pub date: String,
}

#[derive(Clone)]
pub struct LocalLogger {
    log_writer: LogWriter,
    tool_router: ToolRouter<LocalLogger>,
}

#[tool_router]
impl LocalLogger {
    pub fn new() -> Result<Self> {
        let log_writer = LogWriter::from_env()
            .map_err(|e| anyhow::anyhow!("Failed to create LogWriter: {}", e))?;

        Ok(Self {
            log_writer,
            tool_router: Self::tool_router(),
        })
    }

    /// Get the log file path for a specific date
    fn get_log_file_path_for_date(&self, date: &str) -> PathBuf {
        self.log_writer.get_log_file_path(date)
    }

    /// Validate date format (YYYY-MM-DD)
    fn validate_date_format(&self, date: &str) -> Result<(), ErrorData> {
        if date.len() != 10 || !date.chars().nth(4).map_or(false, |c| c == '-') 
            || !date.chars().nth(7).map_or(false, |c| c == '-') {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                "Invalid date format. Expected YYYY-MM-DD".to_string(),
                None,
            ));
        }
        Ok(())
    }

    /// Write a log entry to the appropriate daily log file
    async fn write_log_entry(&self, entry: LogEntry) -> Result<(), std::io::Error> {
        self.log_writer.write_async(entry).await
    }

    #[tool(description = "Write a log message to today's log file")]
    async fn write_log(
        &self,
        Parameters(WriteLogRequest { message, level }): Parameters<WriteLogRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let entry = LogEntry::new_mcp(
            Uuid::new_v4().to_string(),
            level.unwrap_or_else(|| "INFO".to_string()),
            message,
        );

        let date = entry.date.clone();

        match self.write_log_entry(entry).await {
            Ok(_) => {
                let log_file_path = self.get_log_file_path_for_date(&date);
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Successfully wrote log entry to {}",
                    log_file_path.file_name().unwrap().to_str().unwrap()
                ))]))
            }
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to write log: {}", e),
                None,
            )),
        }
    }


    #[tool(description = "Read recent log entries from a specific date")]
    async fn read_logs(
        &self,
        Parameters(ReadLogsRequest { date, lines }): Parameters<ReadLogsRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        use schema::LogEvent;

        let date = date.unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());

        if let Err(e) = self.validate_date_format(&date) {
            return Err(e);
        }

        let log_file_path = self.get_log_file_path_for_date(&date);

        if !log_file_path.exists() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "No logs found for date: {}",
                date
            ))]));
        }

        let lines_to_show = lines.unwrap_or(50);

        // Use efficient tail reading instead of loading entire file
        match tail_reader::read_last_n_lines(&log_file_path, lines_to_show) {
            Ok(log_entries) => {
                let recent_entries: Vec<String> = log_entries
                    .iter()
                    .map(|entry| {
                        match &entry.event {
                            LogEvent::Mcp(mcp) => format!(
                                "[{}] [{}] {}",
                                entry.timestamp.format("%H:%M:%S"),
                                mcp.level,
                                mcp.message
                            ),
                            LogEvent::Hook(hook) => {
                                let mut parts = vec![
                                    format!("[{}]", entry.timestamp.format("%H:%M:%S")),
                                    format!("[HOOK:{}]", hook.event_type),
                                ];

                                if let Some(tool) = &hook.tool_name {
                                    parts.push(format!("Tool: {}", tool));
                                }

                                parts.push(format!("Session: {}", entry.session_id));

                                parts.join(" | ")
                            },
                            LogEvent::ProxyRequest(req) => {
                                use schema::BodyContent;
                                let body_preview = match &req.body.content {
                                    BodyContent::Text { data } => {
                                        if data.len() > 500 {
                                            format!("\n  Body: {}...", &data[..500])
                                        } else if !data.is_empty() {
                                            format!("\n  Body: {}", data)
                                        } else {
                                            String::new()
                                        }
                                    },
                                    BodyContent::Binary { .. } => format!("\n  Body: [Binary, {} bytes]", req.body.size_bytes),
                                    BodyContent::Truncated { preview, .. } => format!("\n  Body: {}... [truncated]", preview),
                                    BodyContent::DecompressionFailed { error } => format!("\n  Body: [Decompression failed: {}]", error),
                                    BodyContent::Empty => String::new(),
                                };
                                format!(
                                    "[{}] [PROXY:REQUEST] {} {} (ID: {}){}",
                                    entry.timestamp.format("%H:%M:%S"),
                                    req.method,
                                    req.uri,
                                    req.id,
                                    body_preview
                                )
                            },
                            LogEvent::ProxyResponse(resp) => {
                                use schema::BodyContent;
                                let body_preview = match &resp.body.content {
                                    BodyContent::Text { data } => {
                                        if data.len() > 500 {
                                            format!("\n  Body: {}...", &data[..500])
                                        } else if !data.is_empty() {
                                            format!("\n  Body: {}", data)
                                        } else {
                                            String::new()
                                        }
                                    },
                                    BodyContent::Binary { .. } => format!("\n  Body: [Binary, {} bytes]", resp.body.size_bytes),
                                    BodyContent::Truncated { preview, .. } => format!("\n  Body: {}... [truncated]", preview),
                                    BodyContent::DecompressionFailed { error } => format!("\n  Body: [Decompression failed: {}]", error),
                                    BodyContent::Empty => String::new(),
                                };
                                format!(
                                    "[{}] [PROXY:RESPONSE] Status: {} Duration: {}ms (Req ID: {}){}",
                                    entry.timestamp.format("%H:%M:%S"),
                                    resp.status,
                                    resp.duration_ms,
                                    resp.request_id,
                                    body_preview
                                )
                            },
                            LogEvent::ProxyDebug(debug) => {
                                format!(
                                    "[{}] [{}] [{}] {}{}",
                                    entry.timestamp.format("%H:%M:%S"),
                                    debug.level,
                                    debug.module.as_ref().unwrap_or(&"proxy".to_string()),
                                    debug.message,
                                    debug.line.map(|l| format!(" (line {})", l)).unwrap_or_default()
                                )
                            },
                        }
                    })
                    .collect();

                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Recent {} entries from {}:\n\n{}",
                    log_entries.len(),
                    date,
                    recent_entries.join("\n")
                ))]))
            }
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to read log file: {}", e),
                None,
            )),
        }
    }

    #[tool(description = "List all available daily log files")]
    async fn list_log_files(&self) -> Result<CallToolResult, ErrorData> {
        match fs::read_dir(self.log_writer.logs_dir()) {
            Ok(entries) => {
                let mut log_files = Vec::new();

                for entry in entries {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        if path.is_file() && path.extension().map_or(false, |ext| ext == "jsonl") {
                            if let Some(filename) = path.file_stem().and_then(|n| n.to_str()) {
                                // Validate that it's a date format
                                if filename.len() == 10 && filename.chars().nth(4) == Some('-') 
                                    && filename.chars().nth(7) == Some('-') {
                                    let metadata = fs::metadata(&path).ok();
                                    let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                                    
                                    // Count number of entries in the file
                                    let entry_count = File::open(&path)
                                        .ok()
                                        .map(|f| BufReader::new(f).lines().count())
                                        .unwrap_or(0);

                                    log_files.push((filename.to_string(), size, entry_count));
                                }
                            }
                        }
                    }
                }

                if log_files.is_empty() {
                    Ok(CallToolResult::success(vec![Content::text(
                        "No log files found".to_string(),
                    )]))
                } else {
                    // Sort by date (newest first)
                    log_files.sort_by(|a, b| b.0.cmp(&a.0));
                    
                    let formatted_list = log_files
                        .iter()
                        .map(|(date, size, entries)| {
                            format!("{} - {} entries ({} bytes)", date, entries, size)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    Ok(CallToolResult::success(vec![Content::text(format!(
                        "Available daily logs:\n\n{}",
                        formatted_list
                    ))]))
                }
            }
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to read logs directory: {}", e),
                None,
            )),
        }
    }

    #[tool(description = "Clear contents of a log file for a specific date")]
    async fn clear_log(
        &self,
        Parameters(ClearLogRequest { date }): Parameters<ClearLogRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        if let Err(e) = self.validate_date_format(&date) {
            return Err(e);
        }

        let log_file_path = self.get_log_file_path_for_date(&date);

        if !log_file_path.exists() {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("No log file exists for date: {}", date),
                None,
            ));
        }

        match File::create(&log_file_path) {
            Ok(_) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Successfully cleared log file for date: {}",
                date
            ))])),
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to clear log file: {}", e),
                None,
            )),
        }
    }
}

#[tool_handler]
impl ServerHandler for LocalLogger {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(concat!(
                "This is a local logging MCP server that provides tools for managing log files. ",
                "You can write log messages, read recent entries, list available log files, and clear log files. ",
                "All log files are stored in a 'logs' directory relative to the server's working directory. ",
                "Log entries include timestamps and severity levels for better organization."
            ).to_string()),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Hook) => {
            // Run in hook mode synchronously (no async runtime needed)
            run_hook_mode_sync()
        }
        Some(Commands::Proxy { config, port }) => {
            // Run as HTTPS proxy with multi-threaded runtime
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(run_proxy_server(config, port))
        }
        Some(Commands::Serve) | None => {
            // Run as MCP server with multi-threaded runtime
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(run_mcp_server())
        }
    }
}

async fn run_mcp_server() -> Result<()> {
    // Initialize logging to stderr (required for stdio MCP servers)
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("Starting Local Logger MCP Server");

    // Create the local logger service
    let logger = LocalLogger::new().map_err(|e| {
        tracing::error!("Failed to create LocalLogger: {:?}", e);
        e
    })?;

    // Serve using stdio transport
    let service = logger.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("Serving error: {:?}", e);
    })?;

    // Wait for the service to complete
    service.waiting().await?;
    
    Ok(())
}

async fn run_proxy_server(config_path: Option<PathBuf>, port: Option<u16>) -> Result<()> {
    use jsonl_tracing_layer::JsonlTracingLayer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Create LogWriter for unified logging
    let log_writer = Arc::new(
        LogWriter::from_env()
            .map_err(|e| anyhow::anyhow!("Failed to create LogWriter: {}", e))?
    );

    // Initialize custom tracing with JSONL output using unified LogWriter
    let jsonl_layer = JsonlTracingLayer::new(log_writer.as_ref().clone());

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with(jsonl_layer)
        .init();

    // Load configuration
    let mut config = if let Some(path) = config_path {
        ProxyConfig::from_file(path)?
    } else {
        ProxyConfig::from_env()
    };

    // Override port if specified
    if let Some(p) = port {
        config.listen_port = p;
    }

    // Create and run proxy server with unified LogWriter
    let proxy = ProxyServer::new(config, log_writer)?;
    proxy.run().await?;

    Ok(())
}

/// Process Claude Code hook events synchronously
///
/// This function:
/// 1. Reads JSON hook event data from stdin
/// 2. Accepts any valid JSON, extracting known fields if available
/// 3. Logs it to today's unified log file as NDJSON
/// 4. Returns exit code 0 to allow tool execution (exit code 2 would block PreToolUse)
fn run_hook_mode_sync() -> Result<()> {
    // Read JSON from stdin
    let mut buffer = String::new();
    io::stdin().read_to_string(&mut buffer)?;

    // First, try to parse as HookEventInput for structured data
    // If that fails, parse as raw JSON Value
    let (hook_event, _raw_json) = match serde_json::from_str::<HookEventInput>(&buffer) {
        Ok(event) => (Some(event), serde_json::from_str::<serde_json::Value>(&buffer).ok()),
        Err(_) => {
            // If HookEventInput parsing fails, try to parse as raw JSON
            match serde_json::from_str::<serde_json::Value>(&buffer) {
                Ok(json) => {
                    // Create a minimal HookEventInput with extracted fields
                    let event = HookEventInput {
                        hook_event_name: json.get("hook_event_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        tool_name: json.get("tool_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        session_id: json.get("session_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        tool_input: json.get("tool_input").cloned(),
                        transcript_path: json.get("transcript_path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        cwd: json.get("cwd").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        extra: std::collections::HashMap::new(),
                    };
                    (Some(event), Some(json))
                },
                Err(e) => {
                    // If we can't parse as JSON at all, log the raw input as an error
                    eprintln!("Failed to parse hook input as JSON: {}", e);
                    eprintln!("Raw input: {}", buffer);
                    return Err(anyhow::anyhow!("Invalid JSON input: {}", e));
                }
            }
        }
    };

    // Create a LogWriter directly - no async needed
    let log_writer = LogWriter::from_env()
        .map_err(|e| anyhow::anyhow!("Failed to create LogWriter: {}", e))?;

    let hook_event = hook_event.unwrap(); // Safe because we ensure it's Some above

    let entry = LogEntry::new_hook(
        hook_event.session_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string()),
        hook_event.hook_event_name.clone().unwrap_or_else(|| "Unknown".to_string()),
        hook_event.tool_name.clone(),
        hook_event.tool_input.clone(),
        hook_event.transcript_path.clone(),
        hook_event.cwd.clone(),
        hook_event.extra.clone(),
    );

    // Write to log synchronously
    log_writer.write_sync(&entry)
        .map_err(|e| anyhow::anyhow!("Failed to write log: {}", e))?;

    // Return success (exit code 0 to allow execution)
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Helper function to create a LocalLogger with a temporary directory
    fn create_test_logger() -> Result<LocalLogger> {
        let temp_dir = TempDir::new()?;
        std::env::set_var("CLAUDE_MCP_LOCAL_LOGGER_DIR", temp_dir.path());
        let logger = LocalLogger::new()?;
        // Keep the temp_dir alive by leaking it (it will be cleaned up when the process exits)
        std::mem::forget(temp_dir);
        Ok(logger)
    }

    #[test]
    fn test_validate_date_format() {
        let logger = create_test_logger().unwrap();
        
        // Valid dates
        assert!(logger.validate_date_format("2025-01-19").is_ok());
        assert!(logger.validate_date_format("2024-12-31").is_ok());
        
        // Invalid dates
        assert!(logger.validate_date_format("2025-1-19").is_err());
        assert!(logger.validate_date_format("2025/01/19").is_err());
        assert!(logger.validate_date_format("25-01-19").is_err());
        assert!(logger.validate_date_format("not-a-date").is_err());
    }

    #[test]
    fn test_log_file_path_generation() {
        let logger = create_test_logger().unwrap();
        let path = logger.get_log_file_path_for_date("2025-01-19");
        assert!(path.to_string_lossy().ends_with("2025-01-19.jsonl"));
    }

    #[test]
    fn test_log_entry_serialization() {
        let entry = LogEntry::new_mcp(
            "test-session".to_string(),
            "INFO".to_string(),
            "Test message".to_string(),
        );

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"type\":\"Mcp\""));
        assert!(json.contains("\"level\":\"INFO\""));
        assert!(json.contains("\"message\":\"Test message\""));
        assert!(json.contains("\"schema_version\":1"));
    }

    #[test]
    fn test_hook_log_entry_serialization() {
        let entry = LogEntry::new_hook(
            "test-123".to_string(),
            "PreToolUse".to_string(),
            Some("Bash".to_string()),
            Some(serde_json::json!({"command": "ls"})),
            None,
            None,
            std::collections::HashMap::new(),
        );

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"type\":\"Hook\""));
        assert!(json.contains("\"event_type\":\"PreToolUse\""));
        assert!(json.contains("\"tool_name\":\"Bash\""));
        assert!(json.contains("\"session_id\":\"test-123\""));
    }

    #[test]
    fn test_hook_event_input_with_missing_fields() {
        // Test parsing hook event with only some fields
        let json_str = r#"{"hook_event_name": "UserPromptSubmit", "session_id": "abc123", "cwd": "/home/user"}"#;
        let hook_event: HookEventInput = serde_json::from_str(json_str).unwrap();

        assert_eq!(hook_event.hook_event_name, Some("UserPromptSubmit".to_string()));
        assert_eq!(hook_event.session_id, Some("abc123".to_string()));
        assert_eq!(hook_event.cwd, Some("/home/user".to_string()));
        assert_eq!(hook_event.tool_name, None);
        assert_eq!(hook_event.tool_input, None);
    }
}