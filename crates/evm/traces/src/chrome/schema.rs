//! Chrome Trace Event Format types.
//!
//! This module provides Rust types that serialize to the Chrome Trace Event Format.
//! See: <https://docs.google.com/document/d/1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU>
//!
//! The format is supported by:
//! - [Perfetto](https://ui.perfetto.dev)
//! - Chrome's built-in trace viewer (`chrome://tracing`)

use serde::{Deserialize, Serialize};

use std::{borrow::Cow, collections::BTreeMap};

/// Root container for a Chrome trace file.
///
/// Uses the object format `{"traceEvents": [...]}` for maximum compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceFile<'a> {
    /// List of trace events.
    pub trace_events: Vec<TraceEvent<'a>>,

    /// Display time unit (e.g., "ns", "ms").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_time_unit: Option<&'static str>,
}

impl<'a> TraceFile<'a> {
    /// Creates a new empty trace file.
    pub fn new() -> Self {
        Self { trace_events: Vec::new(), display_time_unit: Some("ns") }
    }

    /// Adds an event to the trace.
    pub fn add_event(&mut self, event: TraceEvent<'a>) {
        self.trace_events.push(event);
    }
}

impl Default for TraceFile<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// A single trace event.
///
/// We primarily use complete events (`ph: "X"`) which represent a duration
/// with a start timestamp and duration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent<'a> {
    /// Event name (function name, opcode, etc.).
    pub name: Cow<'a, str>,

    /// Event category (for filtering/coloring).
    pub cat: Cow<'a, str>,

    /// Event phase type.
    pub ph: Phase,

    /// Timestamp (start of event).
    pub ts: u64,

    /// Duration (for complete events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dur: Option<u64>,

    /// Event arguments (key-value pairs). Used for counter values and metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<BTreeMap<Cow<'a, str>, serde_json::Value>>,

    /// Process ID. We use 1 for all events.
    pub pid: u32,

    /// Thread ID. Can be used to separate execution phases.
    pub tid: u32,
}

impl<'a> TraceEvent<'a> {
    /// Creates a new complete event with the given parameters.
    pub fn complete(
        name: impl Into<Cow<'a, str>>,
        cat: impl Into<Cow<'a, str>>,
        ts: u64,
        dur: u64,
    ) -> Self {
        Self {
            name: name.into(),
            cat: cat.into(),
            ph: Phase::Complete,
            ts,
            dur: Some(dur),
            args: None,
            pid: 1,
            tid: 1,
        }
    }

    /// Creates a metadata event to set the process name.
    pub fn process_name(name: impl Into<Cow<'a, str>>) -> Self {
        Self {
            name: name.into(),
            cat: Cow::Borrowed("__metadata"),
            ph: Phase::Metadata,
            ts: 0,
            dur: None,
            args: None,
            pid: 1,
            tid: 0,
        }
    }

    /// Creates an instant event (point-in-time marker) for logs.
    pub fn instant(name: impl Into<Cow<'a, str>>, cat: impl Into<Cow<'a, str>>, ts: u64) -> Self {
        Self {
            name: name.into(),
            cat: cat.into(),
            ph: Phase::Instant,
            ts,
            dur: None,
            args: None,
            pid: 1,
            tid: 1,
        }
    }

    /// Creates a counter event to record a numeric value at a point in time.
    pub fn counter(name: impl Into<Cow<'a, str>>, ts: u64, value: u64) -> Self {
        let name = name.into();
        let mut args = BTreeMap::new();
        args.insert(name.clone(), serde_json::Value::Number(value.into()));
        Self {
            name,
            cat: Cow::Borrowed("counter"),
            ph: Phase::Counter,
            ts,
            dur: None,
            args: Some(args),
            pid: 1,
            tid: 1,
        }
    }
}

/// Event phase types.
///
/// We primarily use Complete (`X`) events for function calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// Complete event (duration). Has both timestamp and duration.
    #[serde(rename = "X")]
    Complete,

    /// Instant event (point in time). Used for logs and markers.
    #[serde(rename = "i")]
    Instant,

    /// Counter event. Records numeric values over time.
    #[serde(rename = "C")]
    Counter,

    /// Metadata event. Used for process/thread naming.
    #[serde(rename = "M")]
    Metadata,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_trace_file() {
        let mut file = TraceFile::new();
        file.add_event(TraceEvent::complete("foo", "test", 0, 100));
        file.add_event(TraceEvent::complete("bar", "external", 100, 50));

        let json = serde_json::to_string(&file).unwrap();
        assert!(json.contains("\"traceEvents\""));
        assert!(json.contains("\"ph\":\"X\""));
        assert!(json.contains("\"name\":\"foo\""));
    }

    #[test]
    fn test_complete_event() {
        let event = TraceEvent::complete("test_func", "test", 1000, 500);
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"name\":\"test_func\""));
        assert!(json.contains("\"cat\":\"test\""));
        assert!(json.contains("\"ph\":\"X\""));
        assert!(json.contains("\"ts\":1000"));
        assert!(json.contains("\"dur\":500"));
    }

    #[test]
    fn test_counter_event() {
        let event = TraceEvent::counter("Gas Used", 1000, 21000);
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"name\":\"Gas Used\""));
        assert!(json.contains("\"ph\":\"C\""));
        assert!(json.contains("\"ts\":1000"));
        assert!(json.contains("\"args\":{\"Gas Used\":21000}"));
    }
}
