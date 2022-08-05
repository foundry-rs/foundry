//! forge tests for cheat codes

use crate::{
    config::*,
    test_helpers::{filter::Filter, RE_PATH_SEPARATOR},
};
use forge::result::SuiteResult;
use foundry_evm::decode::decode_console_logs;

/// Executes all cheat code tests but not fork cheat codes
#[test]
fn test_cheats_local() {
    let mut runner = runner();
    let suite_result = runner
        .test(
            &Filter::new(".*", ".*", &format!(".*cheats{}[^Fork]", RE_PATH_SEPARATOR)),
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
