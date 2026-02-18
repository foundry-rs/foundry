//! [Chrome Trace Event Format](https://docs.google.com/document/d/1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU) traces.
//!
//! Generates traces viewable in [Perfetto](https://ui.perfetto.dev) or `chrome://tracing`.

pub mod builder;
pub mod schema;
