//! Tests for reproducing issues

use crate::{config::*, test_helpers::filter::Filter};
use ethers::abi::{Address, Event, EventParam, Log, LogParam, ParamType, RawLog, Token};
use foundry_config::Config;
use std::str::FromStr;

/// A macro that tests a single pattern (".*/repros/<issue>")
macro_rules! test_repro {
    ($issue:expr) => {
        test_repro!($issue, false, None)
    };
    ($issue:expr, $should_fail:expr, $sender:expr) => {
        let pattern = concat!(".*repros/", $issue);
        let filter = Filter::path(pattern);

        let mut config = Config::default();
        if let Some(sender) = $sender {
            config.sender = sender;
        }

        let mut config = TestConfig::with_filter(runner_with_config(config), filter)
            .set_should_fail($should_fail);
        config.run();
    };
}

macro_rules! test_repro_fail {
    ($issue:expr) => {
        test_repro!($issue, true, None)
    };
}

macro_rules! test_repro_with_sender {
    ($issue:expr, $sender:expr) => {
        test_repro!($issue, false, Some($sender))
    };
}

macro_rules! run_test_repro {
    ($issue:expr) => {
        run_test_repro!($issue, false, None)
    };
    ($issue:expr, $should_fail:expr, $sender:expr) => {{
        let pattern = concat!(".*repros/", $issue);
        let filter = Filter::path(pattern);

        let mut config = Config::default();
        if let Some(sender) = $sender {
            config.sender = sender;
        }

        let mut config = TestConfig::with_filter(runner_with_config(config), filter)
            .set_should_fail($should_fail);
        config.test()
    }};
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

// <https://github.com/foundry-rs/foundry/issues/3221>
#[test]
fn test_issue_3223() {
    test_repro_with_sender!(
        "Issue3223",
        Address::from_str("0xF0959944122fb1ed4CfaBA645eA06EED30427BAA").unwrap()
    );
}

// <https://github.com/foundry-rs/foundry/issues/3220>
#[test]
fn test_issue_3220() {
    test_repro!("Issue3220");
}

// <https://github.com/foundry-rs/foundry/issues/3347>
#[test]
fn test_issue_3347() {
    let mut res = run_test_repro!("Issue3347");
    let mut res = res.remove("repros/Issue3347.sol:Issue3347Test").unwrap();
    let test = res.test_results.remove("test()").unwrap();
    assert_eq!(test.logs.len(), 1);
    let event = Event {
        name: "log2".to_string(),
        inputs: vec![
            EventParam { name: "x".to_string(), kind: ParamType::Uint(256), indexed: false },
            EventParam { name: "y".to_string(), kind: ParamType::Uint(256), indexed: false },
        ],
        anonymous: false,
    };
    let raw_log =
        RawLog { topics: test.logs[0].topics.clone(), data: test.logs[0].data.clone().to_vec() };
    let log = event.parse_log(raw_log).unwrap();
    assert_eq!(
        log,
        Log {
            params: vec![
                LogParam { name: "x".to_string(), value: Token::Uint(1u64.into()) },
                LogParam { name: "y".to_string(), value: Token::Uint(2u64.into()) }
            ]
        }
    );
}
