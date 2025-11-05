//! Common test utilities and helpers

use local_logger::log_writer::LogWriter;
use local_logger::schema::LogEntry;
use serde_json::json;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::TempDir;
use uuid::Uuid;

/// Create a test LogWriter with a temporary directory
pub fn create_test_log_writer() -> (LogWriter, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let log_writer = LogWriter::new(temp_dir.path().to_path_buf())
        .expect("Failed to create LogWriter");
    (log_writer, temp_dir)
}

/// Create a log file with a specified number of entries
pub fn create_log_with_entries(dir: &PathBuf, num_entries: usize) -> PathBuf {
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let log_path = dir.join(format!("{}.jsonl", date));

    let mut file = BufWriter::new(
        File::create(&log_path).expect("Failed to create log file")
    );

    for i in 0..num_entries {
        let entry = LogEntry::new_mcp(
            format!("session-{}", i),
            if i % 3 == 0 { "ERROR" } else { "INFO" }.to_string(),
            format!("Test message number {} with some additional text to make it realistic", i),
        );

        serde_json::to_writer(&mut file, &entry).expect("Failed to write entry");
        writeln!(file).expect("Failed to write newline");
    }

    file.flush().expect("Failed to flush file");
    log_path
}

/// Assert that a LogEntry is valid and well-formed
pub fn assert_log_entry_valid(entry: &LogEntry) {
    assert_eq!(entry.schema_version, 1, "Invalid schema version");
    assert!(!entry.session_id.is_empty(), "Empty session ID");
    assert!(!entry.correlation_id.is_empty(), "Empty correlation ID");
    assert!(!entry.date.is_empty(), "Empty date");

    // Validate date format (YYYY-MM-DD)
    assert_eq!(entry.date.len(), 10, "Invalid date length");
    assert_eq!(&entry.date[4..5], "-", "Invalid date format");
    assert_eq!(&entry.date[7..8], "-", "Invalid date format");

    // Validate timestamp is recent (within last hour)
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(entry.timestamp);
    assert!(
        diff.num_seconds() >= 0 && diff.num_seconds() < 3600,
        "Timestamp not recent: {:?}",
        entry.timestamp
    );
}

/// Create a mock hook event JSON string
#[allow(dead_code)]
pub fn mock_hook_event(event_type: &str) -> String {
    json!({
        "hook_event_name": event_type,
        "session_id": format!("test-{}", Uuid::new_v4()),
        "tool_name": "TestTool",
        "tool_input": {
            "param1": "value1",
            "param2": 42
        },
        "transcript_path": "/tmp/transcript.jsonl",
        "cwd": "/tmp/test",
        "extra_field": "extra_value"
    }).to_string()
}

/// Create a minimal hook event JSON string
#[allow(dead_code)]
pub fn minimal_hook_event() -> String {
    json!({
        "hook_event_name": "TestEvent",
        "session_id": "minimal-test"
    }).to_string()
}

/// Create a complex hook event with nested data
#[allow(dead_code)]
pub fn complex_hook_event() -> String {
    json!({
        "hook_event_name": "ComplexEvent",
        "session_id": "complex-test",
        "tool_name": "ComplexTool",
        "tool_input": {
            "nested": {
                "deeply": {
                    "nested": {
                        "value": "found"
                    }
                },
                "array": [1, 2, 3, 4, 5]
            },
            "unicode": "Hello 世界 🌍",
            "large_string": "x".repeat(10000)
        },
        "metadata": {
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "version": "1.0.0"
        }
    }).to_string()
}

/// Run the local-logger binary in hook mode with input
#[allow(dead_code)]
pub fn run_hook_mode(input: &str, log_dir: &PathBuf) -> std::process::Output {
    let binary_path = get_binary_path();

    let mut child = Command::new(&binary_path)
        .arg("hook")
        .env("CLAUDE_MCP_LOCAL_LOGGER_DIR", log_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn hook process");

    // Write input to stdin
    child.stdin
        .as_mut()
        .expect("Failed to get stdin")
        .write_all(input.as_bytes())
        .expect("Failed to write to stdin");

    child.wait_with_output().expect("Failed to wait for process")
}

/// Get the path to the compiled binary
#[allow(dead_code)]
pub fn get_binary_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push(if cfg!(debug_assertions) { "debug" } else { "release" });
    path.push("local-logger");

    // Ensure binary exists
    if !path.exists() {
        // Try to build it
        Command::new("cargo")
            .args(&["build", if cfg!(debug_assertions) { "" } else { "--release" }])
            .output()
            .expect("Failed to build binary");
    }

    path
}

/// Create a large log file for performance testing
#[allow(dead_code)]
pub fn create_large_log_file(dir: &PathBuf, size_mb: usize) -> PathBuf {
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let log_path = dir.join(format!("{}.jsonl", date));

    let mut file = BufWriter::with_capacity(
        1024 * 1024, // 1MB buffer
        File::create(&log_path).expect("Failed to create log file")
    );

    // Calculate approximate entries needed
    // Each entry is roughly 200-300 bytes
    let entries_needed = (size_mb * 1024 * 1024) / 250;

    for i in 0..entries_needed {
        let entry = LogEntry::new_mcp(
            format!("session-{}", i % 1000),
            ["INFO", "WARN", "ERROR", "DEBUG"][i % 4].to_string(),
            format!(
                "Log message {} with some padding text to reach typical size: {}",
                i,
                "x".repeat(50)
            ),
        );

        serde_json::to_writer(&mut file, &entry).expect("Failed to write entry");
        writeln!(file).expect("Failed to write newline");
    }

    file.flush().expect("Failed to flush file");
    log_path
}

/// Verify that a JSONL file is valid and contains expected number of entries
pub fn verify_jsonl_file(path: &PathBuf, expected_entries: usize) -> Vec<LogEntry> {
    assert!(path.exists(), "Log file does not exist: {:?}", path);

    let content = fs::read_to_string(path).expect("Failed to read log file");
    let lines: Vec<&str> = content.trim().split('\n').collect();

    assert_eq!(
        lines.len(),
        expected_entries,
        "Expected {} entries, found {}",
        expected_entries,
        lines.len()
    );

    let mut entries = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        match serde_json::from_str::<LogEntry>(line) {
            Ok(entry) => {
                assert_log_entry_valid(&entry);
                entries.push(entry);
            }
            Err(e) => {
                panic!("Failed to parse entry {} as JSON: {}\nLine: {}", i, e, line);
            }
        }
    }

    entries
}

/// Helper to measure execution time
#[allow(dead_code)]
pub fn measure_time<F, R>(f: F) -> (R, std::time::Duration)
where
    F: FnOnce() -> R,
{
    let start = std::time::Instant::now();
    let result = f();
    let duration = start.elapsed();
    (result, duration)
}

/// Assert that an operation completes within a time limit
#[allow(dead_code)]
pub fn assert_duration<F, R>(f: F, max_millis: u128, operation: &str) -> R
where
    F: FnOnce() -> R,
{
    let (result, duration) = measure_time(f);
    assert!(
        duration.as_millis() <= max_millis,
        "{} took {}ms, expected <= {}ms",
        operation,
        duration.as_millis(),
        max_millis
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_test_log_writer() {
        let (writer, _temp_dir) = create_test_log_writer();
        let entry = LogEntry::new_mcp(
            "test".to_string(),
            "INFO".to_string(),
            "Test".to_string(),
        );
        assert!(writer.write_sync(&entry).is_ok());
    }

    #[test]
    fn test_mock_hook_event() {
        let json = mock_hook_event("PreToolUse");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["hook_event_name"], "PreToolUse");
        assert!(parsed["session_id"].as_str().unwrap().starts_with("test-"));
    }

    #[test]
    fn test_create_log_with_entries() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = create_log_with_entries(&temp_dir.path().to_path_buf(), 10);
        let entries = verify_jsonl_file(&log_path, 10);
        assert_eq!(entries.len(), 10);
    }
}