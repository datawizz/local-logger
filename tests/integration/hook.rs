//! Integration tests for hook mode

use tempfile;

// Import common test utilities from parent
use crate::common::*;

#[test]
fn test_hook_mode_basic() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let input = mock_hook_event("PreToolUse");

    let output = run_hook_mode(&input, &temp_dir.path().to_path_buf());

    assert!(output.status.success(), "Hook failed: {:?}",
            String::from_utf8_lossy(&output.stderr));

    // Verify log was created
    let entries = verify_jsonl_file(
        &temp_dir.path().join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d"))),
        1
    );

    assert_eq!(entries[0].event.as_hook().unwrap().event_type, "PreToolUse");
}

#[test]
fn test_hook_mode_minimal_json() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let input = minimal_hook_event();

    let output = run_hook_mode(&input, &temp_dir.path().to_path_buf());

    assert!(output.status.success());

    let entries = verify_jsonl_file(
        &temp_dir.path().join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d"))),
        1
    );

    assert_eq!(entries[0].event.as_hook().unwrap().event_type, "TestEvent");
    assert!(entries[0].event.as_hook().unwrap().tool_name.is_none());
}

#[test]
fn test_hook_mode_complex_json() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let input = complex_hook_event();

    let output = run_hook_mode(&input, &temp_dir.path().to_path_buf());

    assert!(output.status.success());

    let entries = verify_jsonl_file(
        &temp_dir.path().join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d"))),
        1
    );

    let hook_event = entries[0].event.as_hook().unwrap();
    assert_eq!(hook_event.event_type, "ComplexEvent");

    // Verify complex nested data was preserved
    let tool_input = hook_event.tool_input.as_ref().unwrap();
    assert!(tool_input["nested"]["deeply"]["nested"]["value"].is_string());
    assert!(tool_input["unicode"].as_str().unwrap().contains("世界"));
}

#[test]
fn test_hook_mode_malformed_json() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let inputs = vec![
        "{invalid json",
        "not even json",
        r#"{"partial": "#,
        "",
    ];

    for input in inputs {
        let output = run_hook_mode(input, &temp_dir.path().to_path_buf());

        // Should fail gracefully
        assert!(!output.status.success(),
                "Should fail for malformed input: {}", input);
    }
}

#[test]
fn test_hook_mode_unicode_handling() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let input = serde_json::json!({
        "hook_event_name": "UnicodeTest",
        "session_id": "unicode-session",
        "tool_name": "🔧 Tool",
        "tool_input": {
            "arabic": "مرحبا بالعالم",
            "chinese": "你好世界",
            "emoji": "🌍🌎🌏",
            "russian": "Привет мир",
            "japanese": "こんにちは世界",
            "hebrew": "שלום עולם",
            "mixed": "Hello 世界 🌍 مرحبا мир"
        }
    }).to_string();

    let output = run_hook_mode(&input, &temp_dir.path().to_path_buf());

    assert!(output.status.success());

    let entries = verify_jsonl_file(
        &temp_dir.path().join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d"))),
        1
    );

    let hook_event = entries[0].event.as_hook().unwrap();
    assert_eq!(hook_event.tool_name.as_ref().unwrap(), "🔧 Tool");

    let tool_input = hook_event.tool_input.as_ref().unwrap();
    assert_eq!(tool_input["chinese"].as_str().unwrap(), "你好世界");
    assert_eq!(tool_input["emoji"].as_str().unwrap(), "🌍🌎🌏");
}

#[test]
fn test_hook_mode_concurrent_calls() {
    use std::thread;
    use std::sync::Arc;

    let temp_dir = Arc::new(tempfile::TempDir::new().unwrap());
    let mut handles = vec![];

    // Spawn 10 concurrent hook calls
    for i in 0..10 {
        let temp_dir_clone = temp_dir.clone();
        let handle = thread::spawn(move || {
            let input = serde_json::json!({
                "hook_event_name": "ConcurrentTest",
                "session_id": format!("concurrent-{}", i),
                "tool_name": format!("Tool{}", i)
            }).to_string();

            run_hook_mode(&input, &temp_dir_clone.path().to_path_buf())
        });

        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        let output = handle.join().unwrap();
        assert!(output.status.success());
    }

    // Verify all 10 entries were written
    let entries = verify_jsonl_file(
        &temp_dir.path().join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d"))),
        10
    );

    // Check that all sessions are unique
    let mut session_ids: Vec<String> = entries
        .iter()
        .map(|e| e.session_id.clone())
        .collect();
    session_ids.sort();
    session_ids.dedup();
    assert_eq!(session_ids.len(), 10, "All session IDs should be unique");
}

#[test]
fn test_hook_mode_empty_json_object() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let input = "{}";

    let output = run_hook_mode(input, &temp_dir.path().to_path_buf());

    // Should handle empty object gracefully
    assert!(output.status.success());

    let entries = verify_jsonl_file(
        &temp_dir.path().join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d"))),
        1
    );

    // Should have default values
    let hook_event = entries[0].event.as_hook().unwrap();
    assert_eq!(hook_event.event_type, "Unknown");
}

#[test]
fn test_hook_mode_very_large_input() {
    let temp_dir = tempfile::TempDir::new().unwrap();

    // Create a very large tool_input (5MB)
    let large_data = "x".repeat(5 * 1024 * 1024);
    let input = serde_json::json!({
        "hook_event_name": "LargeInputTest",
        "session_id": "large-test",
        "tool_input": {
            "large_field": large_data
        }
    }).to_string();

    let output = run_hook_mode(&input, &temp_dir.path().to_path_buf());

    assert!(output.status.success());

    // Verify the large input was preserved
    let log_file = temp_dir.path().join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d")));
    let content = std::fs::read_to_string(log_file).unwrap();
    assert!(content.len() > 5 * 1024 * 1024, "Large input should be preserved");
}

#[test]
fn test_hook_mode_special_characters_in_json() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let input = serde_json::json!({
        "hook_event_name": "SpecialCharsTest",
        "session_id": "special-test",
        "tool_input": {
            "quotes": r#"He said "Hello, World!""#,
            "backslash": r"C:\Users\Test\Path",
            "newlines": "Line 1\nLine 2\nLine 3",
            "tabs": "Col1\tCol2\tCol3",
            "control": "\u{0001}\u{001F}",
            "null_char": "before\u{0000}after"
        }
    }).to_string();

    let output = run_hook_mode(&input, &temp_dir.path().to_path_buf());

    assert!(output.status.success());

    let entries = verify_jsonl_file(
        &temp_dir.path().join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d"))),
        1
    );

    let tool_input = entries[0].event.as_hook().unwrap()
        .tool_input.as_ref().unwrap();

    assert!(tool_input["quotes"].as_str().unwrap().contains(r#""Hello, World!""#));
    assert!(tool_input["backslash"].as_str().unwrap().contains(r"\"));
    assert!(tool_input["newlines"].as_str().unwrap().contains('\n'));
}

#[test]
fn test_hook_mode_exit_code_on_success() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let input = mock_hook_event("ExitCodeTest");

    let output = run_hook_mode(&input, &temp_dir.path().to_path_buf());

    // Should return exit code 0 for success (allows tool execution)
    assert!(output.status.success());
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn test_hook_mode_preserves_extra_fields() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let input = serde_json::json!({
        "hook_event_name": "ExtraFieldsTest",
        "session_id": "extra-test",
        "tool_name": "TestTool",
        "custom_field_1": "value1",
        "custom_field_2": 42,
        "custom_field_3": {
            "nested": "data"
        }
    }).to_string();

    let output = run_hook_mode(&input, &temp_dir.path().to_path_buf());

    assert!(output.status.success());

    let entries = verify_jsonl_file(
        &temp_dir.path().join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d"))),
        1
    );

    // Extra fields should be preserved in the extra HashMap
    let hook_event = entries[0].event.as_hook().unwrap();
    assert!(!hook_event.extra.is_empty());
}

// Helper trait to get hook event from LogEvent
trait LogEventExt {
    fn as_hook(&self) -> Option<&local_logger::schema::HookLogEvent>;
}

impl LogEventExt for local_logger::schema::LogEvent {
    fn as_hook(&self) -> Option<&local_logger::schema::HookLogEvent> {
        match self {
            local_logger::schema::LogEvent::Hook(h) => Some(h),
            _ => None,
        }
    }
}