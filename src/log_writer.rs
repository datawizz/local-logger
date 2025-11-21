//! Unified log writer for all logging modes
//!
//! This module provides a single, optimized path for writing log entries
//! across all modes (MCP, Hook, Proxy) ensuring consistency and performance.

use crate::schema::LogEntry;
use fs2::FileExt;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

/// Unified log writer used by all modes
#[derive(Clone)]
pub struct LogWriter {
    logs_dir: PathBuf,
}

impl LogWriter {
    /// Create a new LogWriter instance
    pub fn new(logs_dir: PathBuf) -> io::Result<Self> {
        // Ensure logs directory exists
        if !logs_dir.exists() {
            fs::create_dir_all(&logs_dir)?;
        }

        Ok(Self { logs_dir })
    }

    /// Create from environment variable or default location
    pub fn from_env() -> io::Result<Self> {
        let logs_dir = match std::env::var("CLAUDE_MCP_LOCAL_LOGGER_DIR") {
            Ok(dir) => PathBuf::from(dir),
            Err(_) => {
                // Respect $HOME env var first (for tests/sandbox), fall back to dirs::home_dir()
                let home = std::env::var("HOME")
                    .ok()
                    .map(PathBuf::from)
                    .or_else(|| dirs::home_dir())
                    .ok_or_else(|| io::Error::new(
                        io::ErrorKind::NotFound,
                        "Could not determine home directory"
                    ))?;
                home.join(".local-logger")
            }
        };

        Self::new(logs_dir)
    }

    /// Get the log file path for a specific date
    pub fn get_log_file_path(&self, date: &str) -> PathBuf {
        self.logs_dir.join(format!("{}.jsonl", date))
    }

    /// Write a log entry synchronously with buffering and file locking
    ///
    /// This is the primary write method used by all modes.
    /// It uses BufWriter for efficiency and file locking for cross-process safety.
    /// The exclusive lock prevents race conditions when multiple processes
    /// (hooks, MCP server, proxy) write to the same log file concurrently.
    pub fn write_sync(&self, entry: &LogEntry) -> io::Result<()> {
        let log_file_path = self.get_log_file_path(&entry.date);

        // Open file with append mode
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file_path)?;

        // Acquire exclusive lock for cross-process safety
        // This prevents interleaved writes from multiple processes
        file.lock_exclusive()?;

        // Use BufWriter for efficiency even on single writes
        // 8KB buffer size for OS-level write coalescing
        let mut writer = BufWriter::with_capacity(8192, file);

        // Serialize directly to writer
        serde_json::to_writer(&mut writer, entry)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Write newline
        writer.write_all(b"\n")?;

        // Explicit flush to ensure data is written
        writer.flush()?;

        // Lock is automatically released when file is dropped

        Ok(())
    }

    /// Async wrapper for tokio-based code
    ///
    /// This just calls write_sync but returns a future for compatibility
    /// with async code paths. The actual I/O is still synchronous.
    pub async fn write_async(&self, entry: LogEntry) -> io::Result<()> {
        // Clone self to move into blocking task
        let writer = self.clone();

        // Run synchronous I/O in blocking thread pool
        tokio::task::spawn_blocking(move || writer.write_sync(&entry))
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
    }

    /// Get the logs directory
    pub fn logs_dir(&self) -> &PathBuf {
        &self.logs_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema;
    use serial_test::serial;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::TempDir;

    #[test]
    fn test_log_writer_creation() {
        let temp_dir = TempDir::new().unwrap();
        let _writer = LogWriter::new(temp_dir.path().to_path_buf()).unwrap();
        assert!(temp_dir.path().exists());
    }

    #[test]
    fn test_log_writer_creates_missing_directory() {
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir.path().join("nested/deeply/logs");

        let _writer = LogWriter::new(nested_path.clone()).unwrap();
        assert!(nested_path.exists());
    }

    #[test]
    fn test_write_sync() {
        let temp_dir = TempDir::new().unwrap();
        let writer = LogWriter::new(temp_dir.path().to_path_buf()).unwrap();

        let entry = schema::LogEntry::new_mcp(
            "test-session".to_string(),
            "INFO".to_string(),
            "Test message".to_string(),
        );

        writer.write_sync(&entry).unwrap();

        // Verify file was created
        let log_path = writer.get_log_file_path(&entry.date);
        assert!(log_path.exists());

        // Verify content
        let content = std::fs::read_to_string(log_path).unwrap();
        assert!(content.contains("Test message"));
        assert!(content.ends_with("\n"));
    }

    #[test]
    fn test_concurrent_writes() {
        let temp_dir = TempDir::new().unwrap();
        let writer = Arc::new(LogWriter::new(temp_dir.path().to_path_buf()).unwrap());
        let barrier = Arc::new(Barrier::new(10));
        let mut handles = vec![];

        // Spawn 10 threads that write concurrently
        for i in 0..10 {
            let writer_clone = writer.clone();
            let barrier_clone = barrier.clone();

            let handle = thread::spawn(move || {
                barrier_clone.wait(); // Synchronize all threads to start together

                for j in 0..10 {
                    let entry = schema::LogEntry::new_mcp(
                        format!("session-{}-{}", i, j),
                        "INFO".to_string(),
                        format!("Thread {} message {}", i, j),
                    );
                    writer_clone.write_sync(&entry).unwrap();
                }
            });

            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify all entries were written
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let log_path = writer.get_log_file_path(&date);
        let content = std::fs::read_to_string(log_path).unwrap();
        let lines: Vec<&str> = content.trim().split('\n').collect();

        assert_eq!(lines.len(), 100, "Expected 100 entries from 10 threads × 10 writes");

        // Verify each line is valid JSON
        for line in lines {
            let _: serde_json::Value = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn test_write_with_unicode() {
        let temp_dir = TempDir::new().unwrap();
        let writer = LogWriter::new(temp_dir.path().to_path_buf()).unwrap();

        let entry = schema::LogEntry::new_mcp(
            "unicode-test".to_string(),
            "INFO".to_string(),
            "Hello 世界 🌍 مرحبا мир".to_string(),
        );

        writer.write_sync(&entry).unwrap();

        let log_path = writer.get_log_file_path(&entry.date);
        let content = std::fs::read_to_string(log_path).unwrap();
        assert!(content.contains("Hello 世界 🌍 مرحبا мир"));
    }

    #[test]
    fn test_write_large_entry() {
        let temp_dir = TempDir::new().unwrap();
        let writer = LogWriter::new(temp_dir.path().to_path_buf()).unwrap();

        // Create a large message (1MB)
        let large_message = "x".repeat(1024 * 1024);
        let entry = schema::LogEntry::new_mcp(
            "large-test".to_string(),
            "INFO".to_string(),
            large_message.clone(),
        );

        writer.write_sync(&entry).unwrap();

        let log_path = writer.get_log_file_path(&entry.date);
        let content = std::fs::read_to_string(log_path).unwrap();
        assert!(content.contains(&large_message));
    }

    #[test]
    #[serial]
    fn test_from_env_with_custom_dir() {
        let temp_dir = TempDir::new().unwrap();
        let custom_path = temp_dir.path().join("custom_logs");

        std::env::set_var("CLAUDE_MCP_LOCAL_LOGGER_DIR", &custom_path);
        let writer = LogWriter::from_env().unwrap();
        std::env::remove_var("CLAUDE_MCP_LOCAL_LOGGER_DIR");

        assert!(custom_path.exists());

        // Test writing to custom directory
        let entry = schema::LogEntry::new_mcp(
            "env-test".to_string(),
            "INFO".to_string(),
            "Test".to_string(),
        );
        writer.write_sync(&entry).unwrap();

        let log_path = writer.get_log_file_path(&entry.date);
        assert!(log_path.exists());
        assert!(log_path.starts_with(&custom_path));
    }

    #[tokio::test]
    async fn test_write_async() {
        let temp_dir = TempDir::new().unwrap();
        let writer = LogWriter::new(temp_dir.path().to_path_buf()).unwrap();

        let entry = schema::LogEntry::new_mcp(
            "async-test".to_string(),
            "INFO".to_string(),
            "Async test message".to_string(),
        );

        writer.write_async(entry.clone()).await.unwrap();

        // Verify file was created
        let log_path = writer.get_log_file_path(&entry.date);
        assert!(log_path.exists());

        // Verify content
        let content = std::fs::read_to_string(log_path).unwrap();
        assert!(content.contains("Async test message"));
    }

    #[test]
    fn test_multiple_dates() {
        let temp_dir = TempDir::new().unwrap();
        let writer = LogWriter::new(temp_dir.path().to_path_buf()).unwrap();

        // Write entries with different dates
        for date in ["2025-01-01", "2025-01-02", "2025-01-03"] {
            let mut entry = schema::LogEntry::new_mcp(
                "multi-date".to_string(),
                "INFO".to_string(),
                format!("Entry for {}", date),
            );
            entry.date = date.to_string();

            writer.write_sync(&entry).unwrap();
        }

        // Verify separate files were created
        for date in ["2025-01-01", "2025-01-02", "2025-01-03"] {
            let log_path = temp_dir.path().join(format!("{}.jsonl", date));
            assert!(log_path.exists(), "File for {} should exist", date);

            let content = std::fs::read_to_string(&log_path).unwrap();
            assert!(content.contains(&format!("Entry for {}", date)));
        }
    }
}