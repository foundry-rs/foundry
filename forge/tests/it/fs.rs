//! Tests for reproducing issues

use crate::{
    config::*,
    test_helpers::{filter::Filter, PROJECT},
};
use forge::result::SuiteResult;
use foundry_config::{fs_permissions::PathPermission, Config, FsPermissions};
use foundry_evm::decode::decode_console_logs;

#[test]
fn test_fs_disabled() {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::none("./")]);
    let mut runner = runner_with_config(config);
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*fs/Disabled"), None, TEST_OPTS).unwrap();
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
