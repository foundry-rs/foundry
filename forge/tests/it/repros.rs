//! Tests for reproducing issues

use crate::{
    config::*,
    test_helpers::{filter::Filter, PROJECT},
};
use ethers::abi::{Address, Event, EventParam, Log, LogParam, ParamType, RawLog, Token};
use foundry_config::{fs_permissions::PathPermission, Config, FsPermissions};
use std::str::FromStr;

/// A macro that tests a single pattern (".*/repros/<issue>")
macro_rules! test_repro {
    ($issue:expr) => {
        test_repro!($issue, false, None)
    };
    ($issue:expr, $should_fail:expr, $sender:expr) => {
        let pattern = concat!(".*repros/", $issue);
        let filter = Filter::path(pattern);

        let mut config = Config::with_root(PROJECT.root());
        config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
        if let Some(sender) = $sender {
            config.sender = sender;
        }

        let mut config = TestConfig::with_filter(runner_with_config(config).await, filter)
            .set_should_fail($should_fail);
        config.run().await;
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

        let mut config = TestConfig::with_filter(runner_with_config(config).await, filter)
            .set_should_fail($should_fail);
        config.test().await
    }};
}

// <https://github.com/foundry-rs/foundry/issues/2623>
#[tokio::test]
async fn test_issue_2623() {
    test_repro!("Issue2623");
}

// <https://github.com/foundry-rs/foundry/issues/2629>
#[tokio::test]
async fn test_issue_2629() {
    test_repro!("Issue2629");
}

// <https://github.com/foundry-rs/foundry/issues/2723>
#[tokio::test]
async fn test_issue_2723() {
    test_repro!("Issue2723");
}

// <https://github.com/foundry-rs/foundry/issues/2898>
#[tokio::test]
async fn test_issue_2898() {
    test_repro!("Issue2898");
}

// <https://github.com/foundry-rs/foundry/issues/2956>
#[tokio::test]
async fn test_issue_2956() {
    test_repro!("Issue2956");
}

// <https://github.com/foundry-rs/foundry/issues/2984>
#[tokio::test]
async fn test_issue_2984() {
    test_repro!("Issue2984");
}

// <https://github.com/foundry-rs/foundry/issues/4640>
#[tokio::test]
async fn test_issue_4640() {
    test_repro!("Issue4640");
}

// <https://github.com/foundry-rs/foundry/issues/3077>
#[tokio::test]
async fn test_issue_3077() {
    test_repro!("Issue3077");
}

// <https://github.com/foundry-rs/foundry/issues/3055>
#[tokio::test]
async fn test_issue_3055() {
    test_repro_fail!("Issue3055");
}

// <https://github.com/foundry-rs/foundry/issues/3192>
#[tokio::test]
async fn test_issue_3192() {
    test_repro!("Issue3192");
}

// <https://github.com/foundry-rs/foundry/issues/3110>
#[tokio::test]
async fn test_issue_3110() {
    test_repro!("Issue3110");
}

// <https://github.com/foundry-rs/foundry/issues/3189>
#[tokio::test]
async fn test_issue_3189() {
    test_repro_fail!("Issue3189");
}

// <https://github.com/foundry-rs/foundry/issues/3119>
#[tokio::test]
async fn test_issue_3119() {
    test_repro!("Issue3119");
}

// <https://github.com/foundry-rs/foundry/issues/3190>
#[tokio::test]
async fn test_issue_3190() {
    test_repro!("Issue3190");
}

// <https://github.com/foundry-rs/foundry/issues/3221>
#[tokio::test]
async fn test_issue_3221() {
    test_repro!("Issue3221");
}

// <https://github.com/foundry-rs/foundry/issues/3708>
#[tokio::test]
async fn test_issue_3708() {
    test_repro!("Issue3708");
}

// <https://github.com/foundry-rs/foundry/issues/3221>
#[tokio::test]
async fn test_issue_3223() {
    test_repro_with_sender!(
        "Issue3223",
        Address::from_str("0xF0959944122fb1ed4CfaBA645eA06EED30427BAA").unwrap()
    );
}

// <https://github.com/foundry-rs/foundry/issues/3220>
#[tokio::test]
async fn test_issue_3220() {
    test_repro!("Issue3220");
}

// <https://github.com/foundry-rs/foundry/issues/3347>
#[tokio::test]
async fn test_issue_3347() {
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

// <https://github.com/foundry-rs/foundry/issues/3685>
#[tokio::test]
async fn test_issue_3685() {
    test_repro!("Issue3685");
}

// <https://github.com/foundry-rs/foundry/issues/3653>
#[tokio::test]
async fn test_issue_3653() {
    test_repro!("Issue3653");
}

// <https://github.com/foundry-rs/foundry/issues/3596>
#[tokio::test]
async fn test_issue_3596() {
    test_repro!("Issue3596", true, None);
}

// <https://github.com/foundry-rs/foundry/issues/3661>
#[tokio::test]
async fn test_issue_3661() {
    test_repro!("Issue3661");
}

// <https://github.com/foundry-rs/foundry/issues/3674>
#[tokio::test]
async fn test_issue_3674() {
    test_repro_with_sender!(
        "Issue3674",
        Address::from_str("0xF0959944122fb1ed4CfaBA645eA06EED30427BAA").unwrap()
    );
}

// <https://github.com/foundry-rs/foundry/issues/3703>
#[tokio::test]
async fn test_issue_3703() {
    test_repro!("Issue3703");
}

// <https://github.com/foundry-rs/foundry/issues/3753>
#[tokio::test]
async fn test_issue_3753() {
    test_repro!("Issue3753");
}

// <https://github.com/foundry-rs/foundry/issues/4630>
#[tokio::test]
async fn test_issue_4630() {
    test_repro!("Issue4630");
}

// <https://github.com/foundry-rs/foundry/issues/4586>
#[tokio::test]
async fn test_issue_4586() {
    test_repro!("Issue4586");
}

// <https://github.com/foundry-rs/foundry/issues/5038>
#[tokio::test]
async fn test_issue_5038() {
    test_repro!("Issue5038");
}
