//! Custom tracing layer that writes all log events to JSONL files

use crate::log_writer::LogWriter;
use crate::schema::LogEntry;
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use uuid::Uuid;

/// A tracing layer that writes all log events to daily JSONL files
pub struct JsonlTracingLayer {
    log_writer: LogWriter,
    session_id: String,
}

impl JsonlTracingLayer {
    /// Create a new JSONL tracing layer
    pub fn new(log_writer: LogWriter) -> Self {
        Self {
            log_writer,
            session_id: Uuid::new_v4().to_string(),
        }
    }

    /// Write a log entry to the daily JSONL file
    fn write_log(&self, entry: LogEntry) {
        // Use the unified LogWriter for consistency
        // Ignore errors in tracing layer to avoid panics
        let _ = self.log_writer.write_sync(&entry);
    }
}

impl<S> Layer<S> for JsonlTracingLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();

        let level = metadata.level().to_string().to_uppercase();
        let target = metadata.target();
        let module = metadata.module_path();
        let file = metadata.file();
        let line = metadata.line();

        // Format the message from the event
        let mut message = String::new();
        let mut visitor = MessageVisitor(&mut message);
        event.record(&mut visitor);

        // Extract module name from target for cleaner logging
        let module_name = target
            .split("::")
            .last()
            .map(String::from)
            .or_else(|| module.and_then(|m| m.split("::").last()).map(String::from));

        let entry = LogEntry::new_proxy_debug(
            self.session_id.clone(),
            level,
            message,
            module_name,
            Some(target.to_string()),
            file.map(String::from),
            line,
        );

        self.write_log(entry);
    }
}

/// A visitor for extracting the message from tracing events
struct MessageVisitor<'a>(&'a mut String);

impl<'a> tracing::field::Visit for MessageVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use core::fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.0, "{:?}", value);
        } else {
            if !self.0.is_empty() {
                self.0.push_str(", ");
            }
            let _ = write!(self.0, "{} = {:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        } else {
            if !self.0.is_empty() {
                self.0.push_str(", ");
            }
            self.0.push_str(&format!("{} = \"{}\"", field.name(), value));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        if !self.0.is_empty() {
            self.0.push_str(", ");
        }
        self.0.push_str(&format!("{} = {}", field.name(), value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if !self.0.is_empty() {
            self.0.push_str(", ");
        }
        self.0.push_str(&format!("{} = {}", field.name(), value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if !self.0.is_empty() {
            self.0.push_str(", ");
        }
        self.0.push_str(&format!("{} = {}", field.name(), value));
    }
}