//! Integration tests for proxy mode concurrent writes

use local_logger::log_writer::LogWriter;
use local_logger::schema::LogEntry;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::runtime::Runtime;

/// Test that concurrent proxy writes don't corrupt the JSONL file
#[test]
fn test_proxy_concurrent_writes_no_corruption() {
    let temp_dir = TempDir::new().unwrap();
    let log_writer = Arc::new(LogWriter::new(temp_dir.path().to_path_buf()).unwrap());

    // Create async runtime
    let rt = Runtime::new().unwrap();

    // Spawn multiple concurrent tasks that write log entries
    rt.block_on(async {
        let mut handles = vec![];

        // Simulate 10 concurrent proxy requests/responses
        for i in 0..10 {
            let writer = log_writer.clone();
            let handle = tokio::spawn(async move {
                // Write request
                let request_body = local_logger::schema::BodyData::from_bytes(
                    b"",
                    None,
                    None,
                    1024 * 1024,
                );
                let request_entry = LogEntry::new_proxy_request(
                    format!("session-{}", i),
                    format!("correlation-{}", i),
                    uuid::Uuid::new_v4(),
                    "GET".to_string(),
                    format!("https://api.example.com/v1/messages/{}", i),
                    std::collections::HashMap::new(),
                    request_body,
                    None,
                    None,
                    None,
                    None,
                    Some("v1".to_string()),
                );
                writer.write_async(request_entry).await.unwrap();

                // Write response
                let response_body = local_logger::schema::BodyData::from_bytes(
                    b"",
                    None,
                    None,
                    1024 * 1024,
                );
                let response_entry = LogEntry::new_proxy_response(
                    format!("session-{}", i),
                    format!("correlation-{}", i),
                    uuid::Uuid::new_v4(),
                    200,
                    std::collections::HashMap::new(),
                    response_body,
                    100,
                );
                writer.write_async(response_entry).await.unwrap();
            });
            handles.push(handle);
        }

        // Wait for all tasks to complete
        for handle in handles {
            handle.await.unwrap();
        }
    });

    // Verify the log file is valid JSONL with no corruption
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let log_file_path = temp_dir.path().join(format!("{}.jsonl", date));

    // Read and verify all lines are valid JSON
    let content = std::fs::read_to_string(&log_file_path).unwrap();
    let lines: Vec<&str> = content.trim().split('\n').collect();

    // Should have 20 entries (10 requests + 10 responses)
    assert_eq!(
        lines.len(),
        20,
        "Expected 20 log entries (10 requests + 10 responses)"
    );

    // Verify each line is valid JSON
    for (i, line) in lines.iter().enumerate() {
        let parse_result = serde_json::from_str::<serde_json::Value>(line);
        assert!(
            parse_result.is_ok(),
            "Line {} is not valid JSON: {}\nError: {:?}",
            i + 1,
            line,
            parse_result.err()
        );
    }

    // Verify no concatenated JSON (the bug we're fixing)
    for (i, line) in lines.iter().enumerate() {
        // Check for the race condition pattern: }{"schema_version"
        assert!(
            !line.contains("}{\"schema_version\""),
            "Line {} contains concatenated JSON objects (race condition bug): {}",
            i + 1,
            line
        );

        // Also check for other concatenation patterns
        assert!(
            !line.contains("}{\"timestamp\""),
            "Line {} contains concatenated JSON objects: {}",
            i + 1,
            line
        );
    }
}

/// Test high-concurrency scenario with many simultaneous writes
#[test]
fn test_proxy_high_concurrency_writes() {
    let temp_dir = TempDir::new().unwrap();
    let log_writer = Arc::new(LogWriter::new(temp_dir.path().to_path_buf()).unwrap());

    let rt = Runtime::new().unwrap();

    rt.block_on(async {
        let mut handles = vec![];

        // Simulate 50 concurrent requests (more stress)
        for i in 0..50 {
            let writer = log_writer.clone();
            let handle = tokio::spawn(async move {
                let body = local_logger::schema::BodyData::from_bytes(
                    b"test body",
                    None,
                    Some("text/plain".to_string()),
                    1024 * 1024,
                );
                let entry = LogEntry::new_proxy_request(
                    format!("session-{}", i),
                    format!("correlation-{}", i),
                    uuid::Uuid::new_v4(),
                    "POST".to_string(),
                    "https://api.example.com/v1/messages".to_string(),
                    std::collections::HashMap::new(),
                    body,
                    None,
                    None,
                    None,
                    None,
                    None,
                );
                writer.write_async(entry).await.unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }
    });

    // Verify all entries were written correctly
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let log_file_path = temp_dir.path().join(format!("{}.jsonl", date));
    let content = std::fs::read_to_string(&log_file_path).unwrap();
    let lines: Vec<&str> = content.trim().split('\n').collect();

    assert_eq!(lines.len(), 50, "Expected 50 log entries");

    // Verify no corruption
    for line in lines {
        assert!(serde_json::from_str::<serde_json::Value>(line).is_ok());
        assert!(!line.contains("}{"));
    }
}
