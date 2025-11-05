//! Efficient tail reading for log files

use crate::schema::LogEntry;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;

/// Read the last N lines from a file efficiently without loading the entire file
///
/// This function reads from the end of the file in chunks, making it O(k) where
/// k is the number of lines requested, rather than O(n) where n is the total
/// number of lines in the file.
pub fn read_last_n_lines(file_path: &PathBuf, n: usize) -> Result<Vec<LogEntry>, io::Error> {
    let mut file = File::open(file_path)?;
    let file_size = file.metadata()?.len();

    // Start reading from the end in chunks
    const CHUNK_SIZE: u64 = 64 * 1024; // 64KB chunks
    let mut entries = Vec::new();
    let mut buffer = Vec::new();
    let mut offset = file_size;

    while offset > 0 {
        // Calculate how much to read
        let read_size = CHUNK_SIZE.min(offset);
        offset = offset.saturating_sub(read_size);

        // Seek to position and read chunk
        file.seek(SeekFrom::Start(offset))?;
        let mut chunk = vec![0u8; read_size as usize];
        file.read_exact(&mut chunk)?;

        // Prepend to buffer (we're reading backwards)
        chunk.append(&mut buffer);
        buffer = chunk;

        // Try to parse complete lines from buffer
        let mut start = 0;
        for i in 0..buffer.len() {
            if buffer[i] == b'\n' {
                if start < i {
                    // We have a complete line
                    if let Ok(line_str) = std::str::from_utf8(&buffer[start..i]) {
                        if let Ok(entry) = serde_json::from_str::<LogEntry>(line_str) {
                            entries.push(entry);
                        }
                    }
                }
                start = i + 1;
            }
        }

        // Keep incomplete line for next iteration
        if start < buffer.len() {
            buffer = buffer[start..].to_vec();
        } else {
            buffer.clear();
        }

        // After parsing chunk, stop if we have enough entries
        // This prevents reading more chunks than necessary
        if entries.len() >= n {
            break;
        }
    }

    // Keep only the last n entries
    // Since we parsed chunks in reverse, the last entries in our vec
    // are the last entries in the file
    if entries.len() > n {
        entries.drain(0..entries.len() - n);
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_writer::LogWriter;
    use tempfile::TempDir;

    #[test]
    fn test_read_last_n_lines_basic() {
        let temp_dir = TempDir::new().unwrap();
        let writer = LogWriter::new(temp_dir.path().to_path_buf()).unwrap();

        // Write 10 entries
        for i in 0..10 {
            let entry = LogEntry::new_mcp(
                format!("session-{}", i),
                "INFO".to_string(),
                format!("Message {}", i),
            );
            writer.write_sync(&entry).unwrap();
        }

        let log_path = writer.get_log_file_path(
            &chrono::Utc::now().format("%Y-%m-%d").to_string()
        );

        // Read last 5
        let entries = read_last_n_lines(&log_path, 5).unwrap();
        assert_eq!(entries.len(), 5);

        // Verify they are the last 5
        for (i, entry) in entries.iter().enumerate() {
            let expected_index = 5 + i;
            assert_eq!(entry.session_id, format!("session-{}", expected_index));
        }
    }

    #[test]
    fn test_read_more_than_available() {
        let temp_dir = TempDir::new().unwrap();
        let writer = LogWriter::new(temp_dir.path().to_path_buf()).unwrap();

        // Write only 3 entries
        for i in 0..3 {
            let entry = LogEntry::new_mcp(
                format!("session-{}", i),
                "INFO".to_string(),
                format!("Message {}", i),
            );
            writer.write_sync(&entry).unwrap();
        }

        let log_path = writer.get_log_file_path(
            &chrono::Utc::now().format("%Y-%m-%d").to_string()
        );

        // Try to read 10 (more than available)
        let entries = read_last_n_lines(&log_path, 10).unwrap();
        assert_eq!(entries.len(), 3); // Should return only what's available
    }

    #[test]
    fn test_read_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let empty_file = temp_dir.path().join("empty.jsonl");
        std::fs::write(&empty_file, "").unwrap();

        let entries = read_last_n_lines(&empty_file, 5).unwrap();
        assert_eq!(entries.len(), 0);
    }
}