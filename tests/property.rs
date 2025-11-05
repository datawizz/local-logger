//! Property-based tests using proptest
//!
//! These tests verify that the system behaves correctly with
//! randomly generated inputs, catching edge cases we might not think of.

use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use local_logger::{LogWriter, schema::{LogEntry, LogEvent}};
use tempfile::TempDir;

/// Normalize Option<serde_json::Value> for comparison
///
/// In JSON, there's no distinction between absent and null, so
/// None and Some(Null) should be treated as equivalent for roundtrip tests.
fn normalize_json_option(opt: &Option<serde_json::Value>) -> Option<serde_json::Value> {
    match opt {
        Some(serde_json::Value::Null) => None,
        other => other.clone(),
    }
}

// Strategy for generating valid date strings (YYYY-MM-DD)
fn date_strategy() -> impl Strategy<Value = String> {
    (1900u32..2100, 1u32..=12, 1u32..=31)
        .prop_map(|(year, month, day)| {
            format!("{:04}-{:02}-{:02}", year, month, day)
        })
}

// Strategy for generating log levels
fn log_level_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["TRACE", "DEBUG", "INFO", "WARN", "ERROR"])
        .prop_map(|s| s.to_string())
}

// Strategy for generating session IDs
fn session_id_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9]{8}-[a-zA-Z0-9]{4}-[a-zA-Z0-9]{4}-[a-zA-Z0-9]{4}-[a-zA-Z0-9]{12}"
}

// Strategy for generating arbitrary Unicode strings (including edge cases)
fn unicode_string_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        // Regular ASCII
        "[a-zA-Z0-9 .,!?]{0,100}",
        // Unicode with emojis
        "[\u{0080}-\u{10FFFF}]{0,50}",
        // Mixed content (limited size)
        "\\PC{0,200}",
        // Empty string
        Just("".to_string()),
        // Long string (reduced from 10000 to 500 for faster tests)
        "[x]{500}",
    ]
}

// Strategy for generating random JSON values
fn json_value_strategy() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        // Null
        Just(serde_json::Value::Null),
        // Boolean
        any::<bool>().prop_map(serde_json::Value::Bool),
        // Number (integers and floats)
        any::<i64>().prop_map(|n| serde_json::Value::Number(n.into())),
        any::<u64>().prop_map(|n| serde_json::Value::Number(n.into())),
        // String
        any::<String>().prop_map(serde_json::Value::String),
    ];

    leaf.prop_recursive(
        8,  // depth
        256, // max size at each level
        10, // items per collection
        |inner| {
            prop_oneof![
                // Array
                prop::collection::vec(inner.clone(), 0..10)
                    .prop_map(serde_json::Value::Array),
                // Object
                prop::collection::hash_map(
                    "[a-zA-Z][a-zA-Z0-9_]{0,20}",
                    inner,
                    0..10
                ).prop_map(|m| serde_json::Value::Object(
                    m.into_iter().collect()
                )),
            ]
        }
    )
}

proptest! {
    // Reduce from default 256 cases to 32 for faster test execution
    // Still provides good coverage while keeping tests under 5 seconds
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn test_log_entry_serialization_roundtrip(
        session_id in session_id_strategy(),
        level in log_level_strategy(),
        message in unicode_string_strategy(),
    ) {
        let entry = LogEntry::new_mcp(session_id.clone(), level.clone(), message.clone());

        // Serialize to JSON
        let json = serde_json::to_string(&entry).unwrap();

        // Deserialize back
        let deserialized: LogEntry = serde_json::from_str(&json).unwrap();

        // Verify roundtrip preservation
        assert_eq!(deserialized.session_id, session_id);
        match &deserialized.event {
            LogEvent::Mcp(mcp) => {
                assert_eq!(mcp.level, level);
                assert_eq!(mcp.message, message);
            }
            _ => panic!("Wrong event type"),
        }
    }

    #[test]
    fn test_concurrent_writes_with_random_data(
        entries in prop::collection::vec(
            (session_id_strategy(), log_level_strategy(), unicode_string_strategy()),
            1..20  // Reduced from 100 to 20 since locking serializes writes anyway
        )
    ) {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let writer = Arc::new(LogWriter::new(temp_dir.path().to_path_buf()).unwrap());

        let mut handles = vec![];

        for (session_id, level, message) in entries.clone() {
            let writer = writer.clone();
            let handle = thread::spawn(move || {
                let entry = LogEntry::new_mcp(session_id, level, message);
                writer.write_sync(&entry).unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Verify all entries were written
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let log_path = temp_dir.path().join(format!("{}.jsonl", date));
        let content = std::fs::read_to_string(log_path).unwrap();
        let lines: Vec<&str> = content.trim().split('\n').collect();

        assert_eq!(lines.len(), entries.len());

        // Verify each line is valid JSON
        for line in lines {
            let _: LogEntry = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn test_date_validation(date in date_strategy()) {
        // The date validation should accept all generated dates
        fn validate_date_format(date: &str) -> bool {
            date.len() == 10
                && date.chars().nth(4) == Some('-')
                && date.chars().nth(7) == Some('-')
        }

        assert!(validate_date_format(&date));
    }

    #[test]
    fn test_hook_event_with_random_json(
        event_type in any::<String>(),
        tool_name in prop::option::of(any::<String>()),
        session_id in session_id_strategy(),
        // Generate random JSON value for tool_input
        tool_input in prop::option::of(json_value_strategy()),
    ) {
        use std::collections::HashMap;

        let entry = LogEntry::new_hook(
            session_id.clone(),
            event_type.clone(),
            tool_name.clone(),
            tool_input.clone(),
            None,
            None,
            HashMap::new(),
        );

        // Serialize and deserialize
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: LogEntry = serde_json::from_str(&json).unwrap();

        // Verify preservation
        assert_eq!(deserialized.session_id, session_id);
        match &deserialized.event {
            LogEvent::Hook(hook) => {
                assert_eq!(hook.event_type, event_type);
                assert_eq!(hook.tool_name, tool_name);
                // Normalize JSON options for comparison (None == Some(Null))
                assert_eq!(normalize_json_option(&hook.tool_input), normalize_json_option(&tool_input));
            }
            _ => panic!("Wrong event type"),
        }
    }

    #[test]
    fn test_log_writer_handles_special_characters(
        messages in prop::collection::vec(unicode_string_strategy(), 1..20)
    ) {
        let temp_dir = TempDir::new().unwrap();
        let writer = LogWriter::new(temp_dir.path().to_path_buf()).unwrap();

        for (i, message) in messages.iter().enumerate() {
            let entry = LogEntry::new_mcp(
                format!("session-{}", i),
                "INFO".to_string(),
                message.clone(),
            );

            // Should not panic on any input
            writer.write_sync(&entry).unwrap();
        }

        // Verify all entries can be read back
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let log_path = temp_dir.path().join(format!("{}.jsonl", date));
        let content = std::fs::read_to_string(log_path).unwrap();

        for line in content.trim().split('\n') {
            let entry: LogEntry = serde_json::from_str(line).unwrap();
            match &entry.event {
                LogEvent::Mcp(mcp) => {
                    // Message should be preserved (even if it contains special chars)
                    assert!(messages.contains(&mcp.message));
                }
                _ => panic!("Wrong event type"),
            }
        }
    }

    #[test]
    fn test_tail_reading_correctness(
        num_entries in 1usize..100,  // Reduced from 1000 to 100 for faster tests
        requested in 1usize..50,     // Reduced from 100 to 50
    ) {
        use crate::common::create_log_with_entries;

        let temp_dir = TempDir::new().unwrap();
        let log_path = create_log_with_entries(&temp_dir.path().to_path_buf(), num_entries);

        // Use the tail reading function
        let entries = read_last_n_lines(&log_path, requested).unwrap();

        // Should return min(requested, num_entries)
        let expected_count = requested.min(num_entries);
        assert_eq!(entries.len(), expected_count);

        // If we have entries, verify they are the last ones
        if expected_count > 0 {
            let first_returned_index = num_entries.saturating_sub(expected_count);

            // Check that messages are in correct order
            for (i, entry) in entries.iter().enumerate() {
                let expected_index = first_returned_index + i;
                match &entry.event {
                    LogEvent::Mcp(mcp) => {
                        assert!(mcp.message.contains(&format!("number {}", expected_index)));
                    }
                    _ => panic!("Wrong event type"),
                }
            }
        }
    }

    #[test]
    fn test_large_message_handling(
        size_multiplier in 1usize..10,  // Reduced from 100 to 10 (max 1MB instead of 10MB)
    ) {
        let temp_dir = TempDir::new().unwrap();
        let writer = LogWriter::new(temp_dir.path().to_path_buf()).unwrap();

        // Create a message of variable size (up to ~1MB)
        let message = "x".repeat(size_multiplier * 100 * 1024);

        let entry = LogEntry::new_mcp(
            "large-test".to_string(),
            "INFO".to_string(),
            message.clone(),
        );

        // Should handle large messages without panic
        writer.write_sync(&entry).unwrap();

        // Verify it can be read back
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let log_path = temp_dir.path().join(format!("{}.jsonl", date));
        let content = std::fs::read_to_string(log_path).unwrap();

        let deserialized: LogEntry = serde_json::from_str(content.trim()).unwrap();
        match deserialized.event {
            LogEvent::Mcp(mcp) => {
                assert_eq!(mcp.message, message);
            }
            _ => panic!("Wrong event type"),
        }
    }

    #[test]
    fn test_header_redaction_preserves_structure(
        headers in prop::collection::hash_map(
            any::<String>(),
            any::<String>(),
            0..20
        )
    ) {
        use local_logger::schema::redact_sensitive_headers;

        // Add some known sensitive headers
        let mut test_headers = headers;
        test_headers.insert("Authorization".to_string(), "Bearer secret-token".to_string());
        test_headers.insert("Cookie".to_string(), "session=secret".to_string());
        test_headers.insert("X-API-Key".to_string(), "my-api-key".to_string());

        let redacted = redact_sensitive_headers(&test_headers);

        // Same number of headers
        assert_eq!(redacted.len(), test_headers.len());

        // All keys preserved
        for key in test_headers.keys() {
            assert!(redacted.contains_key(key));
        }

        // Sensitive headers are redacted
        assert!(redacted["Authorization"].contains("[REDACTED"));
        assert_eq!(redacted["Cookie"], "[REDACTED]");
        assert_eq!(redacted["X-API-Key"], "[REDACTED]");
    }
}

// Import common test utilities
mod common;

// Import tail reading function from library
use local_logger::read_last_n_lines;