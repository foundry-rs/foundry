//! Inline configuration tests.

use crate::{
    config::runner,
    test_helpers::{COMPILED, PROJECT},
};
use forge::{
    result::{SuiteResult, TestKind, TestResult},
    TestOptions, TestOptionsBuilder,
};
use foundry_config::{FuzzConfig, InvariantConfig};
use foundry_test_utils::Filter;

#[tokio::test(flavor = "multi_thread")]
async fn inline_config_run_fuzz() {
    let opts = default_test_options();

    let filter = Filter::new(".*", ".*", ".*inline/FuzzInlineConf.t.sol");

    let mut runner = runner().await;
    runner.test_options = opts.clone();

    let result = runner.test(&filter, None, opts).await;
    let suite_result: &SuiteResult =
        result.get("inline/FuzzInlineConf.t.sol:FuzzInlineConf").unwrap();
    let test_result: &TestResult =
        suite_result.test_results.get("testInlineConfFuzz(uint8)").unwrap();
    match &test_result.kind {
        TestKind::Fuzz { runs, .. } => {
            assert_eq!(runs, &1024);
        }
        _ => {
            unreachable!()
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn inline_config_run_invariant() {
    const ROOT: &str = "inline/InvariantInlineConf.t.sol";

    let opts = default_test_options();
    let filter = Filter::new(".*", ".*", ".*inline/InvariantInlineConf.t.sol");
    let mut runner = runner().await;
    runner.test_options = opts.clone();

    let result = runner.test(&filter, None, opts).await;

    let suite_result_1 = result.get(&format!("{ROOT}:InvariantInlineConf")).expect("Result exists");
    let suite_result_2 =
        result.get(&format!("{ROOT}:InvariantInlineConf2")).expect("Result exists");

    let test_result_1 = suite_result_1.test_results.get("invariant_neverFalse()").unwrap();
    let test_result_2 = suite_result_2.test_results.get("invariant_neverFalse()").unwrap();

    match &test_result_1.kind {
        TestKind::Invariant { runs, .. } => {
            assert_eq!(runs, &333);
        }
        _ => {
            unreachable!()
        }
    }

    match &test_result_2.kind {
        TestKind::Invariant { runs, .. } => {
            assert_eq!(runs, &42);
        }
        _ => {
            unreachable!()
        }
    }
}

#[test]
fn build_test_options() {
    let root = &PROJECT.paths.root;
    let profiles = vec!["default".to_string(), "ci".to_string()];
    let build_result = TestOptionsBuilder::default()
        .fuzz(FuzzConfig::default())
        .invariant(InvariantConfig::default())
        .profiles(profiles)
        .build(&COMPILED, root);

    assert!(build_result.is_ok());
}

#[test]
fn build_test_options_just_one_valid_profile() {
    let root = &PROJECT.paths.root;
    let valid_profiles = vec!["profile-sheldon-cooper".to_string()];
    let build_result = TestOptionsBuilder::default()
        .fuzz(FuzzConfig::default())
        .invariant(InvariantConfig::default())
        .profiles(valid_profiles)
        .build(&COMPILED, root);

    // We expect an error, since COMPILED contains in-line
    // per-test configs for "default" and "ci" profiles
    assert!(build_result.is_err());
}

/// Returns the [TestOptions] for the testing [PROJECT].
pub fn default_test_options() -> TestOptions {
    let root = &PROJECT.paths.root;
    TestOptionsBuilder::default()
        .fuzz(FuzzConfig::default())
        .invariant(InvariantConfig::default())
        .build(&COMPILED, root)
        .expect("Config loaded")
}
