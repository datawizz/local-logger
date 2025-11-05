use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use local_logger::schema::LogEntry;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write, BufReader, BufRead, Seek, SeekFrom};
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

/// Create a temporary log file with n entries for benchmarking
fn create_test_log_file(dir: &PathBuf, entries: usize) -> PathBuf {
    let file_path = dir.join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d")));
    let mut file = BufWriter::new(File::create(&file_path).unwrap());

    for i in 0..entries {
        let entry = LogEntry::new_mcp(
            format!("session-{}", i),
            "INFO".to_string(),
            format!("Test message number {}", i),
        );
        serde_json::to_writer(&mut file, &entry).unwrap();
        writeln!(file).unwrap();
    }

    file.flush().unwrap();
    file_path
}

/// Benchmark single log write without buffering (old approach)
fn bench_write_unbuffered(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let log_path = temp_dir.path().join("test.jsonl");

    c.bench_function("write_unbuffered", |b| {
        b.iter(|| {
            let entry = LogEntry::new_mcp(
                Uuid::new_v4().to_string(),
                "INFO".to_string(),
                "Test message".to_string(),
            );

            // Old approach: open, write, close every time
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .unwrap();

            serde_json::to_writer(&mut file, &entry).unwrap();
            writeln!(file).unwrap();
        })
    });
}

/// Benchmark single log write with buffering (new approach)
fn bench_write_buffered(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let log_path = temp_dir.path().join("test.jsonl");

    c.bench_function("write_buffered", |b| {
        b.iter(|| {
            let entry = LogEntry::new_mcp(
                Uuid::new_v4().to_string(),
                "INFO".to_string(),
                "Test message".to_string(),
            );

            // New approach: use BufWriter even for single writes
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .unwrap();

            let mut writer = BufWriter::with_capacity(8192, file);
            serde_json::to_writer(&mut writer, &entry).unwrap();
            writer.write_all(b"\n").unwrap();
            writer.flush().unwrap();
        })
    });
}

/// Benchmark JSON parsing for hook events
fn bench_hook_json_parsing(c: &mut Criterion) {
    let json_simple = r#"{"hook_event_name":"PreToolUse","session_id":"test-123","tool_name":"Bash"}"#;
    let json_complex = r#"{"hook_event_name":"PreToolUse","session_id":"test-123","tool_name":"Bash","tool_input":{"command":"ls -la","description":"List files"},"transcript_path":"/path/to/transcript.jsonl","cwd":"/home/user","extra_field":"value"}"#;

    let mut group = c.benchmark_group("hook_json_parsing");

    group.bench_function("simple", |b| {
        b.iter(|| {
            let _: serde_json::Value = serde_json::from_str(black_box(json_simple)).unwrap();
        })
    });

    group.bench_function("complex", |b| {
        b.iter(|| {
            let _: serde_json::Value = serde_json::from_str(black_box(json_complex)).unwrap();
        })
    });

    group.finish();
}

/// Benchmark reading entire file (old approach)
fn bench_read_entire_file(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();

    let mut group = c.benchmark_group("read_entire_file");

    for size in [100, 1000, 10000].iter() {
        let log_file = create_test_log_file(&temp_dir.path().to_path_buf(), *size);

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                let file = File::open(&log_file).unwrap();
                let reader = BufReader::new(file);
                let mut entries = Vec::new();

                // Old approach: read entire file
                for line in reader.lines() {
                    if let Ok(line) = line {
                        if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
                            entries.push(entry);
                        }
                    }
                }

                // Get last 50
                let _last_50: Vec<_> = entries.into_iter().rev().take(50).collect();
            })
        });
    }

    group.finish();
}

/// Benchmark tail reading (new approach)
fn bench_tail_reading(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();

    let mut group = c.benchmark_group("tail_reading");

    for size in [100, 1000, 10000].iter() {
        let log_file = create_test_log_file(&temp_dir.path().to_path_buf(), *size);

        group.throughput(Throughput::Elements(50)); // We're reading 50 entries
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                let mut file = File::open(&log_file).unwrap();
                let file_size = file.metadata().unwrap().len();

                // New approach: read from end in chunks
                const CHUNK_SIZE: u64 = 64 * 1024;
                let mut entries = Vec::new();
                let mut buffer = Vec::new();
                let mut offset = file_size;

                while entries.len() < 50 && offset > 0 {
                    let read_size = CHUNK_SIZE.min(offset);
                    offset = offset.saturating_sub(read_size);

                    file.seek(SeekFrom::Start(offset)).unwrap();
                    let mut chunk = vec![0u8; read_size as usize];
                    std::io::Read::read_exact(&mut file, &mut chunk).unwrap();

                    chunk.append(&mut buffer);
                    buffer = chunk;

                    let mut start = 0;
                    for i in 0..buffer.len() {
                        if buffer[i] == b'\n' {
                            if start < i {
                                if let Ok(line_str) = std::str::from_utf8(&buffer[start..i]) {
                                    if let Ok(entry) = serde_json::from_str::<LogEntry>(line_str) {
                                        entries.push(entry);
                                        if entries.len() >= 50 {
                                            break;
                                        }
                                    }
                                }
                            }
                            start = i + 1;
                        }
                    }

                    if start < buffer.len() {
                        buffer = buffer[start..].to_vec();
                    } else {
                        buffer.clear();
                    }
                }
            })
        });
    }

    group.finish();
}

/// Benchmark full hook mode pipeline
fn bench_hook_mode_full(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();

    c.bench_function("hook_mode_full_pipeline", |b| {
        let json = r#"{"hook_event_name":"PreToolUse","session_id":"bench","tool_name":"Bash","tool_input":{"command":"ls"}}"#;

        b.iter(|| {
            // Parse JSON
            let parsed: serde_json::Value = serde_json::from_str(json).unwrap();

            // Create log entry
            let entry = LogEntry::new_hook(
                parsed.get("session_id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
                parsed.get("hook_event_name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string(),
                parsed.get("tool_name").and_then(|v| v.as_str()).map(String::from),
                parsed.get("tool_input").cloned(),
                None,
                None,
                std::collections::HashMap::new(),
            );

            // Write to file with buffering
            let log_path = temp_dir.path().join(format!("{}.jsonl", entry.date));
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .unwrap();

            let mut writer = BufWriter::with_capacity(8192, file);
            serde_json::to_writer(&mut writer, &entry).unwrap();
            writer.write_all(b"\n").unwrap();
            writer.flush().unwrap();
        })
    });
}

/// Benchmark memory usage by comparing approaches
fn bench_memory_usage(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let large_file = create_test_log_file(&temp_dir.path().to_path_buf(), 100000);

    let mut group = c.benchmark_group("memory_usage");
    group.sample_size(10); // Reduce sample size for large operations

    // This doesn't directly measure memory, but the performance difference
    // will indicate memory pressure from loading entire file
    group.bench_function("load_100k_entries", |b| {
        b.iter(|| {
            let file = File::open(&large_file).unwrap();
            let reader = BufReader::new(file);
            let mut count = 0;

            for line in reader.lines() {
                if let Ok(line) = line {
                    if serde_json::from_str::<LogEntry>(&line).is_ok() {
                        count += 1;
                    }
                }
            }

            black_box(count);
        })
    });

    group.bench_function("tail_100k_entries", |b| {
        b.iter(|| {
            let mut file = File::open(&large_file).unwrap();
            let file_size = file.metadata().unwrap().len();

            // Just seek to end and read last chunk
            let chunk_size = 64 * 1024;
            let offset = file_size.saturating_sub(chunk_size);
            file.seek(SeekFrom::Start(offset)).unwrap();

            let mut buffer = vec![0u8; chunk_size as usize];
            let _ = std::io::Read::read(&mut file, &mut buffer);

            black_box(buffer);
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_write_unbuffered,
    bench_write_buffered,
    bench_hook_json_parsing,
    bench_read_entire_file,
    bench_tail_reading,
    bench_hook_mode_full,
    bench_memory_usage
);

criterion_main!(benches);