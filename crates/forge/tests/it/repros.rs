//! Regression tests for previous issues.

use crate::{config::*, test_helpers::PROJECT};
use alloy_primitives::{address, Address};
use ethers_core::abi::{Event, EventParam, Log, LogParam, ParamType, RawLog, Token};
use forge::result::TestStatus;
use foundry_common::types::ToEthers;
use foundry_config::{fs_permissions::PathPermission, Config, FsPermissions};
use foundry_evm::{
    constants::HARDHAT_CONSOLE_ADDRESS,
    traces::{CallKind, CallTraceDecoder, DecodedCallData, TraceKind},
};
use foundry_test_utils::Filter;

/// Creates a test that runs `testdata/repros/Issue{issue}.t.sol`.
macro_rules! test_repro {
    ($issue_number:literal $(,)?) => {
        test_repro!($issue_number, false, None);
    };
    ($issue_number:literal, $should_fail:expr $(,)?) => {
        test_repro!($issue_number, $should_fail, None);
    };
    ($issue_number:literal, $should_fail:expr, $sender:expr $(,)?) => {
        paste::paste! {
            #[tokio::test(flavor = "multi_thread")]
            async fn [< issue_ $issue_number >]() {
                repro_config($issue_number, $should_fail, $sender.into()).await.run().await;
            }
        }
    };
    ($issue_number:literal, $should_fail:expr, $sender:expr, |$res:ident| $e:expr $(,)?) => {
        paste::paste! {
            #[tokio::test(flavor = "multi_thread")]
            async fn [< issue_ $issue_number >]() {
                let mut $res = repro_config($issue_number, $should_fail, $sender.into()).await.test().await;
                $e
            }
        }
    };
    ($issue_number:literal; |$config:ident| $e:expr $(,)?) => {
        paste::paste! {
            #[tokio::test(flavor = "multi_thread")]
            async fn [< issue_ $issue_number >]() {
                let mut $config = repro_config($issue_number, false, None).await;
                $e
                $config.run().await;
            }
        }
    };
}

async fn repro_config(issue: usize, should_fail: bool, sender: Option<Address>) -> TestConfig {
    foundry_test_utils::init_tracing();
    let filter = Filter::path(&format!(".*repros/Issue{issue}.t.sol"));

    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions =
        FsPermissions::new(vec![PathPermission::read("./fixtures"), PathPermission::read("out")]);
    if let Some(sender) = sender {
        config.sender = sender;
    }

    let runner = runner_with_config(config).await;
    TestConfig::with_filter(runner, filter).set_should_fail(should_fail)
}

// https://github.com/foundry-rs/foundry/issues/2623
test_repro!(2623);

// https://github.com/foundry-rs/foundry/issues/2629
test_repro!(2629);

// https://github.com/foundry-rs/foundry/issues/2723
test_repro!(2723);

// https://github.com/foundry-rs/foundry/issues/2898
test_repro!(2898);

// https://github.com/foundry-rs/foundry/issues/2956
test_repro!(2956);

// https://github.com/foundry-rs/foundry/issues/2984
test_repro!(2984);

// https://github.com/foundry-rs/foundry/issues/3055
test_repro!(3055, true);

// https://github.com/foundry-rs/foundry/issues/3077
test_repro!(3077);

// https://github.com/foundry-rs/foundry/issues/3110
test_repro!(3110);

// https://github.com/foundry-rs/foundry/issues/3119
test_repro!(3119);

// https://github.com/foundry-rs/foundry/issues/3189
test_repro!(3189, true);

// https://github.com/foundry-rs/foundry/issues/3190
test_repro!(3190);

// https://github.com/foundry-rs/foundry/issues/3192
test_repro!(3192);

// https://github.com/foundry-rs/foundry/issues/3220
test_repro!(3220);

// https://github.com/foundry-rs/foundry/issues/3221
test_repro!(3221);

// https://github.com/foundry-rs/foundry/issues/3223
test_repro!(3223, false, address!("F0959944122fb1ed4CfaBA645eA06EED30427BAA"));

// https://github.com/foundry-rs/foundry/issues/3347
test_repro!(3347, false, None, |res| {
    let mut res = res.remove("repros/Issue3347.t.sol:Issue3347Test").unwrap();
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
    let raw_log = RawLog {
        topics: test.logs[0].data.topics().iter().map(|t| t.to_ethers()).collect(),
        data: test.logs[0].data.data.clone().to_vec(),
    };
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
});

// https://github.com/foundry-rs/foundry/issues/3437
// 1.0 related
// test_repro!(3437);

// https://github.com/foundry-rs/foundry/issues/3596
test_repro!(3596, true, None);

// https://github.com/foundry-rs/foundry/issues/3653
test_repro!(3653);

// https://github.com/foundry-rs/foundry/issues/3661
test_repro!(3661);

// https://github.com/foundry-rs/foundry/issues/3674
test_repro!(3674, false, address!("F0959944122fb1ed4CfaBA645eA06EED30427BAA"));

// https://github.com/foundry-rs/foundry/issues/3685
test_repro!(3685);

// https://github.com/foundry-rs/foundry/issues/3703
test_repro!(3703);

// https://github.com/foundry-rs/foundry/issues/3708
test_repro!(3708);

// https://github.com/foundry-rs/foundry/issues/3723
// 1.0 related
// test_repro!(3723);

// https://github.com/foundry-rs/foundry/issues/3753
test_repro!(3753);

// https://github.com/foundry-rs/foundry/issues/3792
test_repro!(3792);

// https://github.com/foundry-rs/foundry/issues/4586
test_repro!(4586);

// https://github.com/foundry-rs/foundry/issues/4630
test_repro!(4630);

// https://github.com/foundry-rs/foundry/issues/4640
test_repro!(4640);

// https://github.com/foundry-rs/foundry/issues/4832
// 1.0 related
// test_repro!(4832);

// https://github.com/foundry-rs/foundry/issues/5038
test_repro!(5038);

// https://github.com/foundry-rs/foundry/issues/5808
test_repro!(5808);

// <https://github.com/foundry-rs/foundry/issues/5935>
test_repro!(5935);

// <https://github.com/foundry-rs/foundry/issues/5948>
test_repro!(5948);

// https://github.com/foundry-rs/foundry/issues/6006
test_repro!(6006);

// https://github.com/foundry-rs/foundry/issues/6032
test_repro!(6032);

// https://github.com/foundry-rs/foundry/issues/6070
test_repro!(6070);

// https://github.com/foundry-rs/foundry/issues/6115
test_repro!(6115);

// https://github.com/foundry-rs/foundry/issues/6170
test_repro!(6170, false, None, |res| {
    let mut res = res.remove("repros/Issue6170.t.sol:Issue6170Test").unwrap();
    let test = res.test_results.remove("test()").unwrap();
    assert_eq!(test.status, TestStatus::Failure);
    assert_eq!(test.reason, Some("log != expected log".to_string()));
});

// <https://github.com/foundry-rs/foundry/issues/6293>
test_repro!(6293);

// https://github.com/foundry-rs/foundry/issues/6180
test_repro!(6180);

// https://github.com/foundry-rs/foundry/issues/6355
test_repro!(6355, false, None, |res| {
    let mut res = res.remove("repros/Issue6355.t.sol:Issue6355Test").unwrap();
    let test = res.test_results.remove("test_shouldFail()").unwrap();
    assert_eq!(test.status, TestStatus::Failure);

    let test = res.test_results.remove("test_shouldFailWithRevertTo()").unwrap();
    assert_eq!(test.status, TestStatus::Failure);
});

// https://github.com/foundry-rs/foundry/issues/6437
test_repro!(6437);

// Test we decode Hardhat console logs AND traces correctly.
// https://github.com/foundry-rs/foundry/issues/6501
test_repro!(6501, false, None, |res| {
    let mut res = res.remove("repros/Issue6501.t.sol:Issue6501Test").unwrap();
    let test = res.test_results.remove("test_hhLogs()").unwrap();
    assert_eq!(test.status, TestStatus::Success);
    assert_eq!(test.decoded_logs, ["a".to_string(), "1".to_string(), "b 2".to_string()]);

    let (kind, traces) = test.traces[1].clone();
    let nodes = traces.into_nodes();
    assert_eq!(kind, TraceKind::Execution);

    let test_call = nodes.first().unwrap();
    assert_eq!(test_call.idx, 0);
    assert_eq!(test_call.children, [1, 2, 3]);
    assert_eq!(test_call.trace.depth, 0);
    assert!(test_call.trace.success);

    let expected = [
        ("log(string)", vec!["\"a\""]),
        ("log(uint256)", vec!["1"]),
        ("log(string,uint256)", vec!["\"b\"", "2"]),
    ];
    for (node, expected) in nodes[1..=3].iter().zip(expected) {
        let trace = &node.trace;
        let decoded = CallTraceDecoder::new().decode_function(trace).await;
        assert_eq!(trace.kind, CallKind::StaticCall);
        assert_eq!(trace.address, HARDHAT_CONSOLE_ADDRESS);
        assert_eq!(decoded.label, Some("console".into()));
        assert_eq!(trace.depth, 1);
        assert!(trace.success);
        assert_eq!(
            decoded.func,
            Some(DecodedCallData {
                signature: expected.0.into(),
                args: expected.1.into_iter().map(ToOwned::to_owned).collect(),
            })
        );
    }
});

// https://github.com/foundry-rs/foundry/issues/6538
test_repro!(6538);

// https://github.com/foundry-rs/foundry/issues/6554
test_repro!(6554; |config| {
    let mut cheats_config = config.runner.cheats_config.as_ref().clone();
    let path = cheats_config.root.join("out/Issue6554.t.sol");
    cheats_config.fs_permissions.add(PathPermission::read_write(path));
    config.runner.cheats_config = std::sync::Arc::new(cheats_config);
});

// https://github.com/foundry-rs/foundry/issues/6759
test_repro!(6759);
