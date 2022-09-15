//! Tests for reproducing issues

use crate::{config::*, test_helpers::filter::Filter};

/// A macro that tests a single pattern (".*/repros/<issue>")
macro_rules! test_repro {
    ($issue:expr) => {
        test_repro!($issue, false)
    };
    ($issue:expr, $should_fail:expr) => {
        let pattern = concat!(".*repros/", $issue);
        let filter = Filter::path(pattern);
        let mut config = TestConfig::filter(filter).set_should_fail($should_fail);
        config.run();
    };
}

macro_rules! test_repro_fail {
    ($issue:expr) => {
        test_repro!($issue, true)
    };
}

// <https://github.com/foundry-rs/foundry/issues/2623>
#[test]
fn test_issue_2623() {
    test_repro!("Issue2623");
}

// <https://github.com/foundry-rs/foundry/issues/2629>
#[test]
fn test_issue_2629() {
    test_repro!("Issue2629");
}

// <https://github.com/foundry-rs/foundry/issues/2723>
#[test]
fn test_issue_2723() {
    test_repro!("Issue2723");
}

// <https://github.com/foundry-rs/foundry/issues/2898>
#[test]
fn test_issue_2898() {
    test_repro!("Issue2898");
}

// <https://github.com/foundry-rs/foundry/issues/2956>
#[test]
fn test_issue_2956() {
    test_repro!("Issue2956");
}

// <https://github.com/foundry-rs/foundry/issues/2984>
#[test]
fn test_issue_2984() {
    test_repro!("Issue2984");
}

// <https://github.com/foundry-rs/foundry/issues/3077>
#[test]
fn test_issue_3077() {
    test_repro!("Issue3077");
}

// <https://github.com/foundry-rs/foundry/issues/3055>
#[test]
fn test_issue_3055() {
    test_repro_fail!("Issue3055");
}
// <https://github.com/foundry-rs/foundry/issues/3192>
#[test]
fn test_issue_3192() {
    test_repro!("Issue3192");
}

// <https://github.com/foundry-rs/foundry/issues/3110>
#[test]
fn test_issue_3110() {
    test_repro!("Issue3110");
}

// <https://github.com/foundry-rs/foundry/issues/3189>
#[test]
fn test_issue_3189() {
    test_repro_fail!("Issue3189");
}

// <https://github.com/foundry-rs/foundry/issues/3119>
#[test]
fn test_issue_3119() {
    test_repro!("Issue3119");
}

// <https://github.com/foundry-rs/foundry/issues/3190>
#[test]
fn test_issue_3190() {
    test_repro!("Issue3190");
}

// <https://github.com/foundry-rs/foundry/issues/3221>
#[test]
fn test_issue_3221() {
    test_repro!("Issue3221");
}
