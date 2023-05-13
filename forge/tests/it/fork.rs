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
                &format!(".*cheats{RE_PATH_SEPARATOR}Fork"),
            ),
            None,
            test_opts(),
        )
        .unwrap();
    assert_eq!(suite_result.len(), 1);

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (_, result) in test_results {
            assert_eq!(
                result.reason.unwrap(),
                "Contract 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f does not exist on active fork with id `1`\n        But exists on non active forks: `[0]`"
            );
        }
    }
}

/// Executes all non-reverting fork cheatcodes
#[test]
fn test_cheats_fork() {
    let filter = Filter::new(".*", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}Fork"))
        .exclude_tests(".*Revert");
    TestConfig::filter(filter).run();
}

/// Tests that we can launch in forking mode
#[test]
fn test_launch_fork() {
    let rpc_url = foundry_utils::rpc::next_http_archive_rpc_endpoint();
    let runner = forked_runner(&rpc_url);
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}Launch"));
    TestConfig::with_filter(runner, filter).run();
}

/// Tests that we can transact transactions in forking mode
#[test]
fn test_transact_fork() {
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}Transact"));
    TestConfig::filter(filter).run();
}

/// Tests that we can create the same fork (provider,block) concurretnly in different tests
#[test]
fn test_create_same_fork() {
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}ForkSame"));
    TestConfig::filter(filter).run();
}
