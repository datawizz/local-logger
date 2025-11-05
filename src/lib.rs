//! Local Logger library components
//!
//! This module exposes the core components needed for benchmarking
//! and external usage.

pub mod log_writer;
pub mod schema;
pub mod tail_reader;

// Re-export commonly used types
pub use log_writer::LogWriter;
pub use schema::{LogEntry, LogEvent};
pub use tail_reader::read_last_n_lines;