//! Speedscope JSON format types for profile generation.
//!
//! This module provides Rust types that serialize to the speedscope file format.
//! See: <https://github.com/jlfwong/speedscope/wiki/Importing-from-custom-sources>
//!
//! The format supports two profile types:
//! - **Evented**: Open/close frame events with timestamps (like function calls)
//! - **Sampled**: Periodic stack samples with weights
//!
//! For EVM traces, we use the evented format where each function call/return
//! is an event, and gas consumption is encoded as time.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// The speedscope JSON schema URL.
pub const SCHEMA: &str = "https://www.speedscope.app/file-format-schema.json";

/// Root container for a speedscope profile file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedscopeFile<'a> {
    /// JSON schema reference.
    #[serde(rename = "$schema")]
    pub schema: &'static str,

    /// Shared data between profiles (primarily the frame definitions).
    pub shared: Shared<'a>,

    /// List of profiles in this file.
    pub profiles: Vec<Profile<'a>>,

    /// Optional name for the profile file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Cow<'a, str>>,

    /// Index of the initially active profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_profile_index: Option<usize>,

    /// Name of the tool that exported this profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exporter: Option<Cow<'a, str>>,
}

impl<'a> SpeedscopeFile<'a> {
    /// Creates a new speedscope file with the given name.
    pub fn new(name: impl Into<Cow<'a, str>>) -> Self {
        Self {
            schema: SCHEMA,
            shared: Shared::default(),
            profiles: Vec::new(),
            name: Some(name.into()),
            active_profile_index: None,
            exporter: Some(Cow::Borrowed("foundry")),
        }
    }

    /// Adds a frame and returns its index.
    pub fn add_frame(&mut self, frame: Frame<'a>) -> usize {
        let idx = self.shared.frames.len();
        self.shared.frames.push(frame);
        idx
    }

    /// Adds a profile to this file.
    pub fn add_profile(&mut self, profile: Profile<'a>) {
        self.profiles.push(profile);
    }
}

/// Shared data between profiles, primarily frame definitions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Shared<'a> {
    /// All frames referenced by profiles in this file.
    pub frames: Vec<Frame<'a>>,
}

/// A stack frame definition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Frame<'a> {
    /// The name of the frame (function name, etc.).
    pub name: Cow<'a, str>,

    /// Optional source file path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<Cow<'a, str>>,

    /// Optional line number in the source file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,

    /// Optional column number in the source file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,
}

impl<'a> Frame<'a> {
    /// Creates a new frame with just a name.
    pub fn new(name: impl Into<Cow<'a, str>>) -> Self {
        Self { name: name.into(), file: None, line: None, col: None }
    }
}

/// A profile within a speedscope file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Profile<'a> {
    /// Event-based profile with explicit open/close events.
    #[serde(rename = "evented")]
    Evented(EventedProfile<'a>),

    /// Sample-based profile with periodic stack snapshots.
    #[serde(rename = "sampled")]
    Sampled(SampledProfile<'a>),
}

/// An event-based profile where function calls are explicit open/close events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventedProfile<'a> {
    /// Name of this profile (e.g., thread name or test name).
    pub name: Cow<'a, str>,

    /// Unit for the values (e.g., "nanoseconds", "bytes").
    pub unit: ValueUnit,

    /// Starting value (usually 0).
    pub start_value: u64,

    /// Ending value (total time/gas/etc.).
    pub end_value: u64,

    /// List of frame open/close events.
    pub events: Vec<Event>,
}

impl<'a> EventedProfile<'a> {
    /// Creates a new evented profile with the given name and unit.
    pub fn new(name: impl Into<Cow<'a, str>>, unit: ValueUnit) -> Self {
        Self { name: name.into(), unit, start_value: 0, end_value: 0, events: Vec::new() }
    }

    /// Adds an open frame event at the given timestamp.
    pub fn open_frame(&mut self, frame: usize, at: u64) {
        self.events.push(Event { event_type: EventType::Open, frame, at });
    }

    /// Adds a close frame event at the given timestamp.
    pub fn close_frame(&mut self, frame: usize, at: u64) {
        self.events.push(Event { event_type: EventType::Close, frame, at });
    }

    /// Sets the end value based on the final timestamp.
    pub fn set_end_value(&mut self, end_value: u64) {
        self.end_value = end_value;
    }
}

/// A sample-based profile with periodic stack snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampledProfile<'a> {
    /// Name of this profile.
    pub name: Cow<'a, str>,

    /// Unit for the weight values.
    pub unit: ValueUnit,

    /// Starting value.
    pub start_value: u64,

    /// Ending value.
    pub end_value: u64,

    /// Stack samples (each sample is a list of frame indices, bottom to top).
    pub samples: Vec<Vec<usize>>,

    /// Weights for each sample.
    pub weights: Vec<u64>,
}

/// Units for profile values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueUnit {
    /// No unit (raw counts).
    None,
    /// Nanoseconds.
    Nanoseconds,
    /// Microseconds.
    Microseconds,
    /// Milliseconds.
    Milliseconds,
    /// Seconds.
    Seconds,
    /// Bytes.
    Bytes,
}

/// A frame open or close event in an evented profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Type of event (open or close).
    #[serde(rename = "type")]
    pub event_type: EventType,

    /// Index into the shared frames array.
    pub frame: usize,

    /// Timestamp when this event occurred.
    pub at: u64,
}

/// Type of frame event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    /// Frame was opened (function entered).
    #[serde(rename = "O")]
    Open,
    /// Frame was closed (function returned).
    #[serde(rename = "C")]
    Close,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_empty_file() {
        let file = SpeedscopeFile::new("test");
        let json = serde_json::to_string(&file).unwrap();
        assert!(
            json.contains("\"$schema\":\"https://www.speedscope.app/file-format-schema.json\"")
        );
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"exporter\":\"foundry\""));
    }

    #[test]
    fn test_serialize_evented_profile() {
        let mut file = SpeedscopeFile::new("test");
        let frame_a = file.add_frame(Frame::new("a"));
        let frame_b = file.add_frame(Frame::new("b"));

        let mut profile = EventedProfile::new("main", ValueUnit::Nanoseconds);
        profile.open_frame(frame_a, 0);
        profile.open_frame(frame_b, 0);
        profile.close_frame(frame_b, 100);
        profile.close_frame(frame_a, 200);
        profile.set_end_value(200);

        file.add_profile(Profile::Evented(profile));

        let json = serde_json::to_string_pretty(&file).unwrap();
        assert!(json.contains("\"type\": \"evented\""));
        assert!(json.contains("\"unit\": \"nanoseconds\""));
    }
}
