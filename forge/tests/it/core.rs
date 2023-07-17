//! forge tests for core functionality

use crate::{config::*, test_helpers::filter::Filter};

use forge::result::SuiteResult;

use foundry_evm::trace::TraceKind;
use std::{collections::BTreeMap, env};

#[tokio::test(flavor = "multi_thread")]
async fn test_core() {
    let mut runner = runner().await;
    let results = runner.test(&Filter::new(".*", ".*", ".*core"), None, test_opts()).await;

    assert_multiple(
        &results,
        BTreeMap::from([
            (
                "core/FailingSetup.t.sol:FailingSetupTest",
                vec![(
                    "setUp()",
                    false,
                    Some("Setup failed: setup failed predictably".to_string()),
                    None,
                    None,
                )],
            ),
            (
                "core/MultipleSetup.t.sol:MultipleSetup",
                vec![(
                    "setUp()",
                    false,
                    Some("Multiple setUp functions".to_string()),
                    None,
                    Some(1),
                )],
            ),
            (
                "core/Reverting.t.sol:RevertingTest",
                vec![("testFailRevert()", true, None, None, None)],
            ),
            (
                "core/SetupConsistency.t.sol:SetupConsistencyCheck",
                vec![
                    ("testAdd()", true, None, None, None),
                    ("testMultiply()", true, None, None, None),
                ],
            ),
            (
                "core/DSStyle.t.sol:DSStyleTest",
                vec![("testFailingAssertions()", true, None, None, None)],
            ),
            (
                "core/ContractEnvironment.t.sol:ContractEnvironmentTest",
                vec![
                    ("testAddresses()", true, None, None, None),
                    ("testEnvironment()", true, None, None, None),
                ],
            ),
            (
                "core/PaymentFailure.t.sol:PaymentFailureTest",
                vec![("testCantPay()", false, Some("EvmError: Revert".to_string()), None, None)],
            ),
            ("core/Abstract.t.sol:AbstractTest", vec![("testSomething()", true, None, None, None)]),
            (
                "core/FailingTestAfterFailedSetup.t.sol:FailingTestAfterFailedSetupTest",
                vec![(
                    "setUp()",
                    false,
                    Some("Setup failed: execution error".to_string()),
                    None,
                    None,
                )],
            ),
        ]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_linking() {
    let mut runner = runner().await;
    let results = runner.test(&Filter::new(".*", ".*", ".*linking"), None, test_opts()).await;

    assert_multiple(
        &results,
        BTreeMap::from([
            (
                "linking/simple/Simple.t.sol:SimpleLibraryLinkingTest",
                vec![("testCall()", true, None, None, None)],
            ),
            (
                "linking/nested/Nested.t.sol:NestedLibraryLinkingTest",
                vec![
                    ("testDirect()", true, None, None, None),
                    ("testNested()", true, None, None, None),
                ],
            ),
            (
                "linking/duplicate/Duplicate.t.sol:DuplicateLibraryLinkingTest",
                vec![
                    ("testA()", true, None, None, None),
                    ("testB()", true, None, None, None),
                    ("testC()", true, None, None, None),
                    ("testD()", true, None, None, None),
                    ("testE()", true, None, None, None),
                ],
            ),
        ]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_logs() {
    let mut runner = runner().await;
    let results = runner.test(&Filter::new(".*", ".*", ".*logs"), None, test_opts()).await;

    assert_multiple(
        &results,
        BTreeMap::from([
            (
                "logs/DebugLogs.t.sol:DebugLogsTest",
                vec![
                    (
                        "test1()",
                        true,
                        None,
                        Some(vec!["0".into(), "1".into(), "2".into()]),
                        None,
                    ),
                    (
                        "test2()",
                        true,
                        None,
                        Some(vec!["0".into(), "1".into(), "3".into()]),
                        None,
                    ),
                    (
                        "testFailWithRequire()",
                        true,
                        None,
                        Some(vec!["0".into(), "1".into(), "5".into()]),
                        None,
                    ),
                    (
                        "testFailWithRevert()",
                        true,
                        None,
                        Some(vec!["0".into(), "1".into(), "4".into(), "100".into()]),
                        None,
                    ),
                    (
                        "testLog()",
                        true,
                        None,
                        Some(vec!["0".into(), "1".into(), "Error: Assertion Failed".into()]),
                        None,
                    ),
                    (
                        "testLogs()",
                        true,
                        None,
                        Some(vec!["0".into(), "1".into(), "0x61626364".into()]),
                        None,
                    ),
                    (
                        "testLogAddress()",
                        true,
                        None,
                        Some(vec![
                            "0".into(),
                            "1".into(),
                            "0x0000000000000000000000000000000000000001".into(),
                        ]),
                        None,
                    ),
                    (
                        "testLogBytes32()",
                        true,
                        None,
                        Some(vec![
                            "0".into(),
                            "1".into(),
                            "0x6162636400000000000000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogInt()",
                        true,
                        None,
                        Some(vec!["0".into(), "1".into(), "-31337".into()]),
                        None,
                    ),
                    (
                        "testLogBytes()",
                        true,
                        None,
                        Some(vec!["0".into(), "1".into(), "0x61626364".into()]),
                        None,
                    ),
                    (
                        "testLogString()",
                        true,
                        None,
                        Some(vec!["0".into(), "1".into(), "here".into()]),
                        None,
                    ),
                    (
                        "testLogNamedAddress()",
                        true,
                        None,
                        Some(vec![
                            "0".into(),
                            "1".into(),
                            "address: 0x0000000000000000000000000000000000000001".into()]),
                        None,
                    ),
                    (
                        "testLogNamedBytes32()",
                        true,
                        None,
                        Some(vec![
                            "0".into(),
                            "1".into(),
                            "abcd: 0x6162636400000000000000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogNamedDecimalInt()",
                        true,
                        None,
                        Some(vec![
                            "0".into(),
                            "1".into(),
                            "amount: -0.000000000000031337".into()]),
                        None,
                    ),
                    (
                        "testLogNamedDecimalUint()",
                        true,
                        None,
                        Some(vec![
                            "0".into(),
                            "1".into(),
                            "amount: 1.000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogNamedInt()",
                        true,
                        None,
                        Some(vec![
                            "0".into(),
                            "1".into(),
                            "amount: -31337".into()]),
                        None,
                    ),
                    (
                        "testLogNamedUint()",
                        true,
                        None,
                        Some(vec![
                            "0".into(),
                            "1".into(),
                            "amount: 1000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogNamedBytes()",
                        true,
                        None,
                        Some(vec![
                            "0".into(),
                            "1".into(),
                            "abcd: 0x61626364".into()]),
                        None,
                    ),
                    (
                        "testLogNamedString()",
                        true,
                        None,
                        Some(vec![
                            "0".into(),
                            "1".into(),
                            "key: val".into()]),
                        None,
                    ),
                ],
            ),
            (
                "logs/HardhatLogs.t.sol:HardhatLogsTest",
                vec![
                    (
                        "testInts()",
                        true,
                        None,
                        Some(vec![
                            "constructor".into(),
                            "0".into(),
                            "1".into(),
                            "2".into(),
                            "3".into(),
                        ]),
                        None,
                    ),
                    (
                        "testMisc()",
                        true,
                        None,
                        Some(vec![
                            "constructor".into(),
                            "testMisc 0x0000000000000000000000000000000000000001".into(),
                            "testMisc 42".into(),
                        ]),
                        None,
                    ),
                    (
                        "testStrings()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "testStrings".into()]),
                        None,
                    ),
                    (
                        "testConsoleLog()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "test".into()]),
                        None,
                    ),
                    (
                        "testLogInt()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "-31337".into()]),
                        None,
                    ),
                    (
                        "testLogUint()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "1".into()]),
                        None,
                    ),
                    (
                        "testLogString()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "test".into()]),
                        None,
                    ),
                    (
                        "testLogBool()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "false".into()]),
                        None,
                    ),
                    (
                        "testLogAddress()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x0000000000000000000000000000000000000001".into()]),
                        None,
                    ),
                    (
                        "testLogBytes()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61".into()]),
                        None,
                    ),
                    (
                        "testLogBytes1()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61".into()]),
                        None,
                    ),
                    (
                        "testLogBytes2()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100".into()]),
                        None,
                    ),
                    (
                        "testLogBytes3()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x610000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes4()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes5()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes6()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x610000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes7()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes8()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes9()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x610000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes10()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes11()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes12()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x610000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes13()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes14()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes15()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x610000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes16()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes17()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes18()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x610000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes19()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes20()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes21()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x610000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes22()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes23()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes24()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x610000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes25()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61000000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes26()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100000000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes27()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x610000000000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes28()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61000000000000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes29()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100000000000000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes30()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x610000000000000000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes31()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x61000000000000000000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testLogBytes32()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x6100000000000000000000000000000000000000000000000000000000000000".into()]),
                        None,
                    ),
                    (
                        "testConsoleLogUint()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "1".into()]),
                        None,
                    ),
                    (
                        "testConsoleLogString()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "test".into()]),
                        None,
                    ),
                    (
                        "testConsoleLogBool()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "false".into()]),
                        None,
                    ),
                    (
                        "testConsoleLogAddress()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "0x0000000000000000000000000000000000000001".into()]),
                        None,
                    ),
                    (
                        "testConsoleLogFormatString()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "formatted log str=test".into()]),
                        None,
                    ),
                    (
                        "testConsoleLogFormatUint()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "formatted log uint=1".into()]),
                        None,
                    ),
                    (
                        "testConsoleLogFormatAddress()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "formatted log addr=0x0000000000000000000000000000000000000001".into()]),
                        None,
                    ),
                    (
                        "testConsoleLogFormatMulti()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "formatted log str=test uint=1".into()]),
                        None,
                    ),
                    (
                        "testConsoleLogFormatEscape()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "formatted log % test".into()]),
                        None,
                    ),
                    (
                        "testConsoleLogFormatSpill()",
                        true,
                        None,
                        Some(vec!["constructor".into(), "formatted log test 1".into()]),
                        None,
                    ),
                ],
            ),
        ]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_env_vars() {
    let mut runner = runner().await;

    // test `setEnv` first, and confirm that it can correctly set environment variables,
    // so that we can use it in subsequent `env*` tests
    runner.test(&Filter::new("testSetEnv", ".*", ".*"), None, test_opts()).await;
    let env_var_key = "_foundryCheatcodeSetEnvTestKey";
    let env_var_val = "_foundryCheatcodeSetEnvTestVal";
    let res = env::var(env_var_key);
    assert!(
        res.is_ok() && res.unwrap() == env_var_val,
        "Test `testSetEnv` did not pass as expected.
Reason: `setEnv` failed to set an environment variable `{env_var_key}={env_var_val}`"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_doesnt_run_abstract_contract() {
    let mut runner = runner().await;
    let results = runner
        .test(&Filter::new(".*", ".*", ".*Abstract.t.sol".to_string().as_str()), None, test_opts())
        .await;
    assert!(results.get("core/Abstract.t.sol:AbstractTestBase").is_none());
    assert!(results.get("core/Abstract.t.sol:AbstractTest").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace() {
    let mut runner = tracing_runner().await;
    let suite_result = runner.test(&Filter::new(".*", ".*", ".*trace"), None, test_opts()).await;

    // TODO: This trace test is very basic - it is probably a good candidate for snapshot
    // testing.
    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let deployment_traces =
                result.traces.iter().filter(|(kind, _)| *kind == TraceKind::Deployment);
            let setup_traces = result.traces.iter().filter(|(kind, _)| *kind == TraceKind::Setup);
            let execution_traces =
                result.traces.iter().filter(|(kind, _)| *kind == TraceKind::Deployment);

            assert_eq!(
                deployment_traces.count(),
                1,
                "Test {test_name} did not have exactly 1 deployment trace."
            );
            assert!(setup_traces.count() <= 1, "Test {test_name} had more than 1 setup trace.");
            assert_eq!(
                execution_traces.count(),
                1,
                "Test {test_name} did not not have exactly 1 execution trace."
            );
        }
    }
}
