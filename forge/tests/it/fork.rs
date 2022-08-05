//! forge tests for cheat codes

use crate::{
    config::*,
    test_helpers::{filter::Filter, RE_PATH_SEPARATOR},
};
use forge::result::SuiteResult;
use foundry_evm::decode::decode_console_logs;

/// Executes reverting fork test
#[test]
fn test_cheats_fork_revert() {
    let mut runner = runner();
    let suite_result = runner
        .test(
            &Filter::new(
                "testNonExistingContractRevert",
                ".*",
                &format!(".*cheats{}Fork", RE_PATH_SEPARATOR),
            ),
            None,
            TEST_OPTS,
        )
        .unwrap();
    assert_eq!(suite_result.len(), 1);

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (_, result) in test_results {
            assert_eq!(
                result.reason.unwrap(),
                "Contract 0xCe71065D4017F316EC606Fe4422e11eB2c47c246 does not exists on active fork with id `1`\n        But exists on non active forks: `[0]`"
            );
        }
    }
}

/// Executes all non-reverting fork cheatcodes
#[test]
fn test_cheats_fork() {
    let mut runner = runner();
    let suite_result = runner
        .test(
            &Filter::new(".*", ".*", &format!(".*cheats{}Fork", RE_PATH_SEPARATOR))
                .exclude_tests(".*Revert"),
            None,
            TEST_OPTS,
        )
        .unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

#[test]
fn test_fork() {
    let rpc_url = foundry_utils::rpc::next_http_archive_rpc_endpoint();
    let mut runner = forked_runner(&rpc_url);
    let suite_result = runner.test(&Filter::new(".*", ".*", ".*fork"), None, TEST_OPTS).unwrap();

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);

            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}
