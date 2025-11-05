//! Performance regression tests
//!
//! These tests ensure that performance optimizations are maintained
//! and catch any regressions in critical paths.

mod common;
use common::*;
use local_logger::schema::LogEntry;
use std::time::{Duration, Instant};

/// Maximum acceptable time for hook mode execution
const MAX_HOOK_MODE_MS: u128 = 5; // 5ms (was 15-20ms before optimization)

/// Maximum acceptable time for writing a single log entry
const MAX_WRITE_MS: u128 = 2; // 2ms

/// Maximum acceptable time for reading last N entries
const MAX_TAIL_READ_MS: u128 = 20; // 20ms even for large files

#[test]
fn test_hook_mode_performance_regression() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let input = mock_hook_event("PerformanceTest");

    // Warm up (first run might be slower)
    run_hook_mode(&input, &temp_dir.path().to_path_buf());

    // Measure multiple runs
    let mut durations = vec![];
    for _ in 0..10 {
        let start = Instant::now();
        let output = run_hook_mode(&input, &temp_dir.path().to_path_buf());
        let duration = start.elapsed();

        assert!(output.status.success());
        durations.push(duration.as_millis());
    }

    // Calculate average
    let avg_duration = durations.iter().sum::<u128>() / durations.len() as u128;

    assert!(
        avg_duration <= MAX_HOOK_MODE_MS,
        "Hook mode average duration {}ms exceeds maximum {}ms",
        avg_duration,
        MAX_HOOK_MODE_MS
    );

    // Also check that no single run was too slow
    for duration in &durations {
        assert!(
            *duration <= MAX_HOOK_MODE_MS * 2,
            "Hook mode single run {}ms exceeds 2x maximum",
            duration
        );
    }
}

#[test]
fn test_write_performance_regression() {
    let (writer, _temp_dir) = create_test_log_writer();

    // Warm up
    let entry = LogEntry::new_mcp(
        "perf-test".to_string(),
        "INFO".to_string(),
        "Performance test message".to_string(),
    );
    writer.write_sync(&entry).unwrap();

    // Measure write performance
    let mut durations = vec![];
    for i in 0..100 {
        let entry = LogEntry::new_mcp(
            format!("perf-test-{}", i),
            "INFO".to_string(),
            format!("Performance test message {}", i),
        );

        let start = Instant::now();
        writer.write_sync(&entry).unwrap();
        let duration = start.elapsed();

        durations.push(duration.as_millis());
    }

    let avg_duration = durations.iter().sum::<u128>() / durations.len() as u128;

    assert!(
        avg_duration <= MAX_WRITE_MS,
        "Write average duration {}ms exceeds maximum {}ms",
        avg_duration,
        MAX_WRITE_MS
    );
}

#[test]
fn test_tail_reading_performance_regression() {

    let temp_dir = tempfile::TempDir::new().unwrap();

    // Create log files of various sizes
    let sizes = vec![
        (100, "small"),
        (1000, "medium"),
        (10000, "large"),
        (100000, "very_large"),
    ];

    for (num_entries, label) in sizes {
        // Create a log file with specified entries
        let log_path = create_log_with_entries(&temp_dir.path().to_path_buf(), num_entries);

        // Measure tail reading performance
        let start = Instant::now();
        let entries = read_last_n_lines(&log_path, 50).unwrap();
        let duration = start.elapsed();

        assert_eq!(entries.len().min(50), entries.len());

        assert!(
            duration.as_millis() <= MAX_TAIL_READ_MS,
            "Tail reading {} file ({} entries) took {}ms, exceeds maximum {}ms",
            label,
            num_entries,
            duration.as_millis(),
            MAX_TAIL_READ_MS
        );

        // Verify it's actually faster than loading entire file for very large files
        // Note: For moderately-sized files, JSON parsing overhead may exceed I/O savings
        // Only check on very large files (100k+ entries) where I/O dominates
        if num_entries >= 100000 {
            // Time full file read
            let start = Instant::now();
            let _content = std::fs::read_to_string(&log_path).unwrap();
            let full_read_duration = start.elapsed();

            assert!(
                duration < full_read_duration,
                "Tail reading took {}ms vs full read {}ms for {} entries",
                duration.as_millis(),
                full_read_duration.as_millis(),
                num_entries
            );
        }
    }
}

#[test]
fn test_memory_efficiency_tail_reading() {
    // This test verifies that tail reading uses constant memory
    // regardless of file size

    let temp_dir = tempfile::TempDir::new().unwrap();

    // Create a very large log file (50MB)
    let large_log = create_large_log_file(&temp_dir.path().to_path_buf(), 50);

    // Memory usage should remain constant for tail reading
    // We can't directly measure memory in Rust tests easily,
    // but we can ensure the operation completes quickly
    // which indicates we're not loading the entire file

    let start = Instant::now();
    let entries = read_last_n_lines(&large_log, 100).unwrap();
    let duration = start.elapsed();

    assert_eq!(entries.len(), 100);
    assert!(
        duration.as_millis() < 100,
        "Reading last 100 entries from 50MB file took {}ms, should be < 100ms",
        duration.as_millis()
    );
}

#[test]
fn test_concurrent_write_performance() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let (writer, _temp_dir) = create_test_log_writer();
    let writer = Arc::new(writer);
    let barrier = Arc::new(Barrier::new(10));

    let start = Instant::now();

    let mut handles = vec![];
    for i in 0..10 {
        let writer = writer.clone();
        let barrier = barrier.clone();

        let handle = thread::spawn(move || {
            barrier.wait(); // Synchronize start

            for j in 0..100 {
                let entry = LogEntry::new_mcp(
                    format!("thread-{}-{}", i, j),
                    "INFO".to_string(),
                    format!("Concurrent test message {} from thread {}", j, i),
                );
                writer.write_sync(&entry).unwrap();
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let duration = start.elapsed();

    // 1000 total writes (10 threads × 100 writes) should complete quickly
    assert!(
        duration.as_secs() < 2,
        "1000 concurrent writes took {}s, should be < 2s",
        duration.as_secs()
    );
}

#[test]
fn test_large_entry_write_performance() {
    let (writer, _temp_dir) = create_test_log_writer();

    // Test writing increasingly large entries
    let sizes = vec![
        (1024, "1KB"),
        (10 * 1024, "10KB"),
        (100 * 1024, "100KB"),
        (1024 * 1024, "1MB"),
    ];

    for (size, label) in sizes {
        let large_message = "x".repeat(size);
        let entry = LogEntry::new_mcp(
            "large-test".to_string(),
            "INFO".to_string(),
            large_message,
        );

        let start = Instant::now();
        writer.write_sync(&entry).unwrap();
        let duration = start.elapsed();

        // Even 1MB entries should write quickly
        assert!(
            duration.as_millis() < 50,
            "Writing {} entry took {}ms, should be < 50ms",
            label,
            duration.as_millis()
        );
    }
}

#[test]
fn test_json_serialization_performance() {
    use local_logger::schema::LogEntry;
    use std::collections::HashMap;

    // Create a complex log entry
    let mut extra = HashMap::new();
    for i in 0..100 {
        extra.insert(format!("field_{}", i), serde_json::json!(format!("value_{}", i)));
    }

    let entry = LogEntry::new_hook(
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
            }
        })),
        Some("/path/to/transcript.jsonl".to_string()),
        Some("/current/working/dir".to_string()),
        extra,
    );

    // Measure serialization performance
    let start = Instant::now();
    for _ in 0..100 {
        let _json = serde_json::to_string(&entry).unwrap();
    }
    let duration = start.elapsed();

    assert!(
        duration.as_millis() < 100,
        "100 complex serializations took {}ms, should be < 100ms",
        duration.as_millis()
    );
}

#[test]
#[ignore] // Mark as slow test - run with `cargo test -- --ignored`
fn test_sustained_high_throughput() {
    let (writer, _temp_dir) = create_test_log_writer();

    // Simulate sustained high throughput (1000 writes/second for 10 seconds)
    let duration = Duration::from_secs(10);
    let target_rate = 1000; // writes per second

    let start = Instant::now();
    let mut count = 0;

    while start.elapsed() < duration {
        let entry = LogEntry::new_mcp(
            format!("throughput-{}", count),
            "INFO".to_string(),
            format!("Throughput test message {}", count),
        );

        writer.write_sync(&entry).unwrap();
        count += 1;

        // Simple rate limiting
        if count % target_rate == 0 {
            let elapsed = start.elapsed();
            let expected = Duration::from_secs(count / target_rate);
            if elapsed < expected {
                std::thread::sleep(expected - elapsed);
            }
        }
    }

    let actual_rate = count as f64 / start.elapsed().as_secs_f64();

    assert!(
        actual_rate >= target_rate as f64 * 0.9, // Allow 10% variance
        "Sustained throughput {:.0} writes/s is below target {} writes/s",
        actual_rate,
        target_rate
    );
}

// Import tail reading function from library
use local_logger::read_last_n_lines;