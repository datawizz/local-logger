//! Strongly-typed log schema for local-logger
//!
//! This module defines the complete type hierarchy for all log events.
//! The schema is versioned to enable future migrations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Current schema version
pub const SCHEMA_VERSION: u32 = 1;

/// Sensitive headers that should be redacted in logs
pub const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "api-key",
    "x-api-key",
    "x-auth-token",
    "x-session-token",
    "proxy-authorization",
    "www-authenticate",
    "authentication",
];

/// Root log entry structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Schema version for migration support
    pub schema_version: u32,
    /// ISO 8601 timestamp
    pub timestamp: DateTime<Utc>,
    /// Date in YYYY-MM-DD format for file organization
    pub date: String,
    /// Session identifier (from hooks or generated)
    pub session_id: String,
    /// Correlation ID for linking related events (e.g., request/response pairs)
    pub correlation_id: String,
    /// The actual log event
    pub event: LogEvent,
}

/// Discriminated union of all possible log event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LogEvent {
    /// MCP server log event
    Mcp(McpLogEvent),
    /// Claude Code hook event
    Hook(HookLogEvent),
    /// Proxy request event
    ProxyRequest(ProxyRequestEvent),
    /// Proxy response event
    ProxyResponse(ProxyResponseEvent),
    /// Proxy debug/info/error log event
    ProxyDebug(ProxyDebugEvent),
}

/// MCP server log event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpLogEvent {
    /// Log level (INFO, ERROR, WARN, etc.)
    pub level: String,
    /// Log message
    pub message: String,
}

/// Proxy debug/info/error log event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyDebugEvent {
    /// Log level: TRACE, DEBUG, INFO, WARN, ERROR
    pub level: String,
    /// Log message
    pub message: String,
    /// Module that generated the log (e.g., "proxy_server", "certificate_manager")
    pub module: Option<String>,
    /// The rust module path
    pub target: Option<String>,
    /// Source file
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
}

/// Claude Code hook event with rich metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookLogEvent {
    /// Event type: "PreToolUse", "PostToolUse", etc.
    pub event_type: String,
    /// Name of the tool being invoked (e.g., "Bash", "Read", "Write")
    pub tool_name: Option<String>,
    /// Tool input parameters (varies by tool)
    pub tool_input: Option<serde_json::Value>,
    /// Transcript path
    pub transcript_path: Option<String>,
    /// Current working directory
    pub cwd: Option<String>,
    /// Additional fields from the hook payload
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// HTTP/HTTPS proxy request event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyRequestEvent {
    /// Unique request ID for correlation
    pub id: Uuid,
    /// HTTP method (GET, POST, etc.)
    pub method: String,
    /// Full request URI
    pub uri: String,
    /// Request headers (sensitive headers redacted)
    pub headers: HashMap<String, String>,
    /// Request body with metadata
    pub body: BodyData,
    /// Optional TLS handshake time in milliseconds (for first request to host)
    pub tls_handshake_ms: Option<u64>,
    /// Parsed URL components for API replay
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_components: Option<UrlComponents>,
    /// curl command template for replaying this request (auth placeholder)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curl_command: Option<String>,
    /// Detected API endpoint pattern (e.g., "/v1/messages")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_pattern: Option<String>,
    /// API version detected from URL or headers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
}

/// Parsed URL components for API replay
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlComponents {
    /// URL scheme (http, https)
    pub scheme: String,
    /// Hostname
    pub host: String,
    /// Port number
    pub port: Option<u16>,
    /// URL path
    pub path: String,
    /// Parsed query parameters
    pub query_params: HashMap<String, String>,
}

/// HTTP/HTTPS proxy response event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyResponseEvent {
    /// References the request ID
    pub request_id: Uuid,
    /// HTTP status code
    pub status: u16,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Response body with metadata
    pub body: BodyData,
    /// Time from request to response in milliseconds
    pub duration_ms: u64,
}

/// Intelligent body data handling with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BodyData {
    /// Original encoding (gzip, deflate, br, etc.)
    pub original_encoding: Option<String>,
    /// Content type from headers
    pub content_type: Option<String>,
    /// Original body size in bytes (before decompression)
    pub size_bytes: usize,
    /// Stored body size in bytes (after decompression/truncation)
    pub stored_size_bytes: usize,
    /// Whether the body was truncated
    pub truncated: bool,
    /// The actual body content
    pub content: BodyContent,
}

/// Body content with explicit handling of different cases
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BodyContent {
    /// Text body (UTF-8)
    Text { data: String },
    /// Binary body (base64 encoded)
    Binary { data: String },
    /// Truncated body with preview
    Truncated { preview: String, reason: String },
    /// Decompression failed
    DecompressionFailed { error: String },
    /// Empty body
    Empty,
}

/// Helper function to redact sensitive headers
pub fn redact_sensitive_headers(headers: &HashMap<String, String>) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(key, value)| {
            let key_lower = key.to_lowercase();
            if SENSITIVE_HEADERS.contains(&key_lower.as_str()) {
                // Preserve the auth type but redact the value
                let redacted_value = if key_lower == "authorization" && value.contains(' ') {
                    let parts: Vec<&str> = value.splitn(2, ' ').collect();
                    format!("[REDACTED:{}]", parts[0])
                } else {
                    "[REDACTED]".to_string()
                };
                (key.clone(), redacted_value)
            } else {
                (key.clone(), value.clone())
            }
        })
        .collect()
}

impl LogEntry {
    /// Create a new MCP log entry
    pub fn new_mcp(session_id: String, level: String, message: String) -> Self {
        let now = Utc::now();
        Self {
            schema_version: SCHEMA_VERSION,
            timestamp: now,
            date: now.format("%Y-%m-%d").to_string(),
            session_id: session_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            event: LogEvent::Mcp(McpLogEvent { level, message }),
        }
    }

    /// Create a new hook log entry
    pub fn new_hook(
        session_id: String,
        event_type: String,
        tool_name: Option<String>,
        tool_input: Option<serde_json::Value>,
        transcript_path: Option<String>,
        cwd: Option<String>,
        extra: HashMap<String, serde_json::Value>,
    ) -> Self {
        let now = Utc::now();
        Self {
            schema_version: SCHEMA_VERSION,
            timestamp: now,
            date: now.format("%Y-%m-%d").to_string(),
            session_id: session_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            event: LogEvent::Hook(HookLogEvent {
                event_type,
                tool_name,
                tool_input,
                transcript_path,
                cwd,
                extra,
            }),
        }
    }

    /// Create a new proxy request log entry
    pub fn new_proxy_request(
        session_id: String,
        correlation_id: String,
        request_id: Uuid,
        method: String,
        uri: String,
        headers: HashMap<String, String>,
        body: BodyData,
        tls_handshake_ms: Option<u64>,
        url_components: Option<UrlComponents>,
        curl_command: Option<String>,
        endpoint_pattern: Option<String>,
        api_version: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            schema_version: SCHEMA_VERSION,
            timestamp: now,
            date: now.format("%Y-%m-%d").to_string(),
            session_id,
            correlation_id,
            event: LogEvent::ProxyRequest(ProxyRequestEvent {
                id: request_id,
                method,
                uri,
                headers,
                body,
                tls_handshake_ms,
                url_components,
                curl_command,
                endpoint_pattern,
                api_version,
            }),
        }
    }

    /// Create a new proxy response log entry
    pub fn new_proxy_response(
        session_id: String,
        correlation_id: String,
        request_id: Uuid,
        status: u16,
        headers: HashMap<String, String>,
        body: BodyData,
        duration_ms: u64,
    ) -> Self {
        let now = Utc::now();
        Self {
            schema_version: SCHEMA_VERSION,
            timestamp: now,
            date: now.format("%Y-%m-%d").to_string(),
            session_id,
            correlation_id,
            event: LogEvent::ProxyResponse(ProxyResponseEvent {
                request_id,
                status,
                headers,
                body,
                duration_ms,
            }),
        }
    }

    /// Create a new proxy debug log entry
    pub fn new_proxy_debug(
        session_id: String,
        level: String,
        message: String,
        module: Option<String>,
        target: Option<String>,
        file: Option<String>,
        line: Option<u32>,
    ) -> Self {
        let now = Utc::now();
        Self {
            schema_version: SCHEMA_VERSION,
            timestamp: now,
            date: now.format("%Y-%m-%d").to_string(),
            session_id,
            correlation_id: Uuid::new_v4().to_string(),
            event: LogEvent::ProxyDebug(ProxyDebugEvent {
                level,
                message,
                module,
                target,
                file,
                line,
            }),
        }
    }
}

impl BodyData {
    /// Create body data from raw bytes with intelligent handling
    pub fn from_bytes(
        bytes: &[u8],
        content_encoding: Option<String>,
        content_type: Option<String>,
        max_size: usize,
    ) -> Self {
        let original_size = bytes.len();

        // Handle compression
        let (decompressed, decompression_error) = if let Some(ref encoding) = content_encoding {
            if encoding.contains("gzip") {
                match Self::decompress_gzip(bytes) {
                    Ok(data) => (Some(data), None),
                    Err(e) => (None, Some(e)),
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let working_bytes = decompressed.as_deref().unwrap_or(bytes);

        // Handle decompression failure
        if let Some(error) = decompression_error {
            return Self {
                original_encoding: content_encoding,
                content_type,
                size_bytes: original_size,
                stored_size_bytes: 0,
                truncated: false,
                content: BodyContent::DecompressionFailed {
                    error: error.to_string(),
                },
            };
        }

        // Handle empty body
        if working_bytes.is_empty() {
            return Self {
                original_encoding: content_encoding,
                content_type,
                size_bytes: original_size,
                stored_size_bytes: 0,
                truncated: false,
                content: BodyContent::Empty,
            };
        }

        // Handle truncation
        if working_bytes.len() > max_size {
            let preview = String::from_utf8_lossy(&working_bytes[..max_size.min(1024)]).to_string();
            return Self {
                original_encoding: content_encoding,
                content_type,
                size_bytes: original_size,
                stored_size_bytes: preview.len(),
                truncated: true,
                content: BodyContent::Truncated {
                    preview,
                    reason: format!("Body size {} exceeds max {}", working_bytes.len(), max_size),
                },
            };
        }

        // Try to parse as text
        match String::from_utf8(working_bytes.to_vec()) {
            Ok(text) => Self {
                original_encoding: content_encoding,
                content_type,
                size_bytes: original_size,
                stored_size_bytes: text.len(),
                truncated: false,
                content: BodyContent::Text { data: text },
            },
            Err(_) => {
                // Binary data - base64 encode
                let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, working_bytes);
                Self {
                    original_encoding: content_encoding,
                    content_type,
                    size_bytes: original_size,
                    stored_size_bytes: encoded.len(),
                    truncated: false,
                    content: BodyContent::Binary { data: encoded },
                }
            }
        }
    }

    /// Decompress gzip data
    fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let mut decoder = GzDecoder::new(data);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;
        Ok(decompressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_version() {
        let entry = LogEntry::new_mcp(
            "test-session".to_string(),
            "INFO".to_string(),
            "test message".to_string(),
        );
        assert_eq!(entry.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn test_body_data_text() {
        let body = BodyData::from_bytes(
            b"hello world",
            None,
            Some("text/plain".to_string()),
            1024,
        );

        assert_eq!(body.size_bytes, 11);
        assert_eq!(body.stored_size_bytes, 11);
        assert!(!body.truncated);

        match body.content {
            BodyContent::Text { data } => assert_eq!(data, "hello world"),
            _ => panic!("Expected Text content"),
        }
    }

    #[test]
    fn test_body_data_truncation() {
        let large_data = vec![b'a'; 2000];
        let body = BodyData::from_bytes(
            &large_data,
            None,
            None,
            1000,
        );

        assert_eq!(body.size_bytes, 2000);
        assert!(body.truncated);

        match body.content {
            BodyContent::Truncated { preview, reason } => {
                assert!(!preview.is_empty());
                assert!(reason.contains("exceeds max"));
            }
            _ => panic!("Expected Truncated content"),
        }
    }

    #[test]
    fn test_body_data_empty() {
        let body = BodyData::from_bytes(b"", None, None, 1024);

        assert_eq!(body.size_bytes, 0);
        assert_eq!(body.stored_size_bytes, 0);

        match body.content {
            BodyContent::Empty => {},
            _ => panic!("Expected Empty content"),
        }
    }

    #[test]
    fn test_log_entry_serialization() {
        let entry = LogEntry::new_mcp(
            "test-123".to_string(),
            "INFO".to_string(),
            "test".to_string(),
        );

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"schema_version\":1"));
        assert!(json.contains("\"type\":\"Mcp\""));
    }
}
