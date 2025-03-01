use tracing::instrument;
use tracing_error::{ErrorLayer, SpanTrace};
use tracing_subscriber::{prelude::*, registry::Registry};

#[instrument]
fn main() {
    Registry::default().with(ErrorLayer::default()).init();

    let span_trace = one(42);
    println!("{}", color_spantrace::colorize(&span_trace));
}

#[instrument]
fn one(i: u32) -> SpanTrace {
    two()
}

#[instrument]
fn two() -> SpanTrace {
    SpanTrace::capture()
}
