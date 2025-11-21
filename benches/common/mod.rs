//! Common utilities for benchmarks

use local_logger::schema::LogEntry;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use tempfile::TempDir;

/// Create a test LogWriter-compatible temporary directory
pub fn create_bench_dir() -> TempDir {
    TempDir::new().expect("Failed to create temp dir")
}

/// Create a log file with a specified number of entries
pub fn create_log_file_with_entries(dir: &PathBuf, num_entries: usize) -> PathBuf {
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let log_path = dir.join(format!("{}.jsonl", date));

    let mut file = BufWriter::new(File::create(&log_path).expect("Failed to create log file"));

    for i in 0..num_entries {
        let entry = LogEntry::new_mcp(
            format!("session-{}", i),
            if i % 3 == 0 { "ERROR" } else { "INFO" }.to_string(),
            format!(
                "Test message number {} with some additional text to make it realistic",
                i
            ),
        );

        serde_json::to_writer(&mut file, &entry).expect("Failed to write entry");
        writeln!(file).expect("Failed to write newline");
    }

    file.flush().expect("Failed to flush file");
    log_path
}

/// Create a log entry with a specific payload size in bytes
pub fn create_entry_with_size(size_bytes: usize) -> LogEntry {
    // Reserve space for JSON overhead (~200 bytes)
    let overhead = 200;
    let message_size = if size_bytes > overhead {
        size_bytes - overhead
    } else {
        10
    };

    LogEntry::new_mcp(
        "bench-session".to_string(),
        "INFO".to_string(),
        "x".repeat(message_size),
    )
}

/// Create a log entry with simple structure
pub fn create_simple_entry() -> LogEntry {
    LogEntry::new_mcp(
        "simple-session".to_string(),
        "INFO".to_string(),
        "Simple test message".to_string(),
    )
}

/// Create a log entry with moderate complexity
pub fn create_moderate_entry() -> LogEntry {
    let mut extra = HashMap::new();
    for i in 0..10 {
        extra.insert(
            format!("field_{}", i),
            serde_json::json!(format!("value_{}", i)),
        );
    }

    LogEntry::new_hook(
        "moderate-session".to_string(),
        "PreToolUse".to_string(),
        Some("Bash".to_string()),
        Some(serde_json::json!({
            "command": "ls -la",
            "description": "List files"
        })),
        Some("/path/to/transcript.jsonl".to_string()),
        Some("/current/working/dir".to_string()),
        extra,
    )
}

/// Create a log entry with complex nested structure
pub fn create_complex_entry() -> LogEntry {
    let mut extra = HashMap::new();
    for i in 0..100 {
        extra.insert(
            format!("field_{}", i),
            serde_json::json!(format!("value_{}", i)),
        );
    }

    LogEntry::new_hook(
        "complex-session".to_string(),
        "ComplexEvent".to_string(),
        Some("ComplexTool".to_string()),
        Some(serde_json::json!({
            "nested": {
                "deeply": {
                    "nested": {
                        "array": (0..1000).collect::<Vec<_>>()
                    }
                }
            },
            "unicode": "Hello 世界 🌍",
            "large_data": "x".repeat(5000)
        })),
        Some("/path/to/transcript.jsonl".to_string()),
        Some("/current/working/dir".to_string()),
        extra,
    )
}

/// Create a large log file for memory benchmarks
pub fn create_large_log_file(dir: &PathBuf, size_mb: usize) -> PathBuf {
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let log_path = dir.join(format!("{}.jsonl", date));

    let mut file =
        BufWriter::with_capacity(1024 * 1024, File::create(&log_path).expect("Failed to create"));

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
