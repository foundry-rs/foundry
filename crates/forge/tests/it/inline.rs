//! Inline configuration tests.

use crate::test_helpers::{ForgeTestData, ForgeTestProfile, TEST_DATA_DEFAULT};
use forge::{result::TestKind, TestOptionsBuilder};
use foundry_config::{FuzzConfig, InvariantConfig};
use foundry_test_utils::Filter;

#[tokio::test(flavor = "multi_thread")]
async fn inline_config_run_fuzz() {
    let filter = Filter::new(".*", ".*", ".*inline/FuzzInlineConf.t.sol");
    // Fresh runner to make sure there's no persisted failure from previous tests.
    let mut runner = ForgeTestData::new(ForgeTestProfile::Default).runner();
    let result = runner.test_collect(&filter);
    let results = result
        .into_iter()
        .flat_map(|(path, r)| {
            r.test_results.into_iter().map(move |(name, t)| {
                let runs = match t.kind {
                    TestKind::Fuzz { runs, .. } => runs,
                    _ => unreachable!(),
                };
                (path.clone(), name, runs)
            })
        })
        .collect::<Vec<_>>();

    assert_eq!(
        results,
        vec![
            (
                "default/inline/FuzzInlineConf.t.sol:FuzzInlineConf".to_string(),
                "testInlineConfFuzz(uint8)".to_string(),
                1024
            ),
            (
                "default/inline/FuzzInlineConf.t.sol:FuzzInlineConf2".to_string(),
                "testInlineConfFuzz1(uint8)".to_string(),
                1
            ),
            (
                "default/inline/FuzzInlineConf.t.sol:FuzzInlineConf2".to_string(),
                "testInlineConfFuzz2(uint8)".to_string(),
                10
            ),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn inline_config_run_invariant() {
    const ROOT: &str = "default/inline/InvariantInlineConf.t.sol";

    let filter = Filter::new(".*", ".*", ".*inline/InvariantInlineConf.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner();
    let result = runner.test_collect(&filter);

    let suite_result_1 = result.get(&format!("{ROOT}:InvariantInlineConf")).expect("Result exists");
    let suite_result_2 =
        result.get(&format!("{ROOT}:InvariantInlineConf2")).expect("Result exists");

    let test_result_1 = suite_result_1.test_results.get("invariant_neverFalse()").unwrap();
    match test_result_1.kind {
        TestKind::Invariant { runs, .. } => assert_eq!(runs, 333),
        _ => unreachable!(),
    }

    let test_result_2 = suite_result_2.test_results.get("invariant_neverFalse()").unwrap();
    match test_result_2.kind {
        TestKind::Invariant { runs, .. } => assert_eq!(runs, 42),
        _ => unreachable!(),
    }
}

#[test]
fn build_test_options() {
    let root = &TEST_DATA_DEFAULT.project.paths.root;
    let profiles = vec!["default".to_string(), "ci".to_string()];
    let build_result = TestOptionsBuilder::default()
        .fuzz(FuzzConfig::default())
        .invariant(InvariantConfig::default())
        .profiles(profiles)
        .build(&TEST_DATA_DEFAULT.output, root);

    assert!(build_result.is_ok());
}

#[test]
fn build_test_options_just_one_valid_profile() {
    let root = &TEST_DATA_DEFAULT.project.root();
    let valid_profiles = vec!["profile-sheldon-cooper".to_string()];
    let build_result = TestOptionsBuilder::default()
        .fuzz(FuzzConfig::default())
        .invariant(InvariantConfig::default())
        .profiles(valid_profiles)
        .build(&TEST_DATA_DEFAULT.output, root);

    // We expect an error, since COMPILED contains in-line
    // per-test configs for "default" and "ci" profiles
    assert!(build_result.is_err());
}
