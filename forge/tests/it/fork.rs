//! forge tests for cheat codes

use crate::{
    config::*,
    test_helpers::{filter::Filter, RE_PATH_SEPARATOR},
};
use forge::result::SuiteResult;

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
                "Contract 0xCe71065D4017F316EC606Fe4422e11eB2c47c246 does not exist on active fork with id `1`\n        But exists on non active forks: `[0]`"
            );
        }
    }
}

/// Executes all non-reverting fork cheatcodes
#[test]
fn test_cheats_fork() {
    let filter = Filter::new(".*", ".*", &format!(".*cheats{}Fork", RE_PATH_SEPARATOR))
        .exclude_tests(".*Revert");
    TestConfig::filter(filter).run();
}

/// Tests that we can launch in forking mode
#[test]
fn test_launch_fork() {
    let rpc_url = foundry_utils::rpc::next_http_archive_rpc_endpoint();
    let runner = forked_runner(&rpc_url);
    let filter = Filter::new(".*", ".*", &format!(".*fork{}Launch", RE_PATH_SEPARATOR));
    TestConfig::with_filter(runner, filter).run();
}

/// Tests that we can transact transactions in forking mode
#[test]
fn test_transact_fork() {
    let filter = Filter::new(".*", ".*", &format!(".*fork{}Transact", RE_PATH_SEPARATOR));
    TestConfig::filter(filter).run();
}
