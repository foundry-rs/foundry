//! Inline configuration tests.

use crate::test_helpers::TEST_DATA_DEFAULT;
use forge::result::TestKind;
use foundry_test_utils::Filter;

#[tokio::test(flavor = "multi_thread")]
async fn inline_config_run_fuzz() {
    let filter = Filter::new(".*", ".*", ".*inline/FuzzInlineConf.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.optimizer = Some(true);
    });
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
