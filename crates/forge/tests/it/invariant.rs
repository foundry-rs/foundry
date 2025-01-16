//! Invariant tests.

use crate::{config::*, test_helpers::TEST_DATA_DEFAULT};
use alloy_primitives::U256;
use forge::fuzz::CounterExample;
use foundry_config::{Config, InvariantConfig};
use foundry_test_utils::{forgetest_init, str, Filter};
use std::collections::BTreeMap;

macro_rules! get_counterexample {
    ($runner:ident, $filter:expr) => {
        $runner
            .test_collect($filter)
            .values()
            .last()
            .expect("Invariant contract should be testable.")
            .test_results
            .values()
            .last()
            .expect("Invariant contract should be testable.")
            .counterexample
            .as_ref()
            .expect("Invariant contract should have failed with a counterexample.")
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_with_alias() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantTest1.t.sol");
    let results = TEST_DATA_DEFAULT.runner().test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantTest1.t.sol:InvariantTest",
            vec![
                ("invariant_neverFalse()", false, Some("revert: false".into()), None, None),
                (
                    "statefulFuzz_neverFalseWithInvariantAlias()",
                    false,
                    Some("revert: false".into()),
                    None,
                    None,
                ),
            ],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_filters() {
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.runs = 10;
    });

    // Contracts filter tests.
    assert_multiple(
        &runner.test_collect(&Filter::new(
            ".*",
            ".*",
            ".*fuzz/invariant/target/(ExcludeContracts|TargetContracts).t.sol",
        )),
        BTreeMap::from([
            (
                "default/fuzz/invariant/target/ExcludeContracts.t.sol:ExcludeContracts",
                vec![("invariantTrueWorld()", true, None, None, None)],
            ),
            (
                "default/fuzz/invariant/target/TargetContracts.t.sol:TargetContracts",
                vec![("invariantTrueWorld()", true, None, None, None)],
            ),
        ]),
    );

    // Senders filter tests.
    assert_multiple(
        &runner.test_collect(&Filter::new(
            ".*",
            ".*",
            ".*fuzz/invariant/target/(ExcludeSenders|TargetSenders).t.sol",
        )),
        BTreeMap::from([
            (
                "default/fuzz/invariant/target/ExcludeSenders.t.sol:ExcludeSenders",
                vec![("invariantTrueWorld()", true, None, None, None)],
            ),
            (
                "default/fuzz/invariant/target/TargetSenders.t.sol:TargetSenders",
                vec![(
                    "invariantTrueWorld()",
                    false,
                    Some("revert: false world".into()),
                    None,
                    None,
                )],
            ),
        ]),
    );

    // Interfaces filter tests.
    assert_multiple(
        &runner.test_collect(&Filter::new(
            ".*",
            ".*",
            ".*fuzz/invariant/target/TargetInterfaces.t.sol",
        )),
        BTreeMap::from([(
            "default/fuzz/invariant/target/TargetInterfaces.t.sol:TargetWorldInterfaces",
            vec![("invariantTrueWorld()", false, Some("revert: false world".into()), None, None)],
        )]),
    );

    // Selectors filter tests.
    assert_multiple(
        &runner.test_collect(&Filter::new(
            ".*",
            ".*",
            ".*fuzz/invariant/target/(ExcludeSelectors|TargetSelectors).t.sol",
        )),
        BTreeMap::from([
            (
                "default/fuzz/invariant/target/ExcludeSelectors.t.sol:ExcludeSelectors",
                vec![("invariantFalseWorld()", true, None, None, None)],
            ),
            (
                "default/fuzz/invariant/target/TargetSelectors.t.sol:TargetSelectors",
                vec![("invariantTrueWorld()", true, None, None, None)],
            ),
        ]),
    );

    // Artifacts filter tests.
    assert_multiple(
        &runner.test_collect(&Filter::new(
            ".*",
            ".*",
            ".*fuzz/invariant/targetAbi/(ExcludeArtifacts|TargetArtifacts|TargetArtifactSelectors|TargetArtifactSelectors2).t.sol",
        )),
        BTreeMap::from([
            (
                "default/fuzz/invariant/targetAbi/ExcludeArtifacts.t.sol:ExcludeArtifacts",
                vec![("invariantShouldPass()", true, None, None, None)],
            ),
            (
                "default/fuzz/invariant/targetAbi/TargetArtifacts.t.sol:TargetArtifacts",
                vec![
                    ("invariantShouldPass()", true, None, None, None),
                    (
                        "invariantShouldFail()",
                        false,
                        Some("revert: false world".into()),
                        None,
                        None,
                    ),
                ],
            ),
            (
                "default/fuzz/invariant/targetAbi/TargetArtifactSelectors.t.sol:TargetArtifactSelectors",
                vec![("invariantShouldPass()", true, None, None, None)],
            ),
            (
                "default/fuzz/invariant/targetAbi/TargetArtifactSelectors2.t.sol:TargetArtifactSelectors2",
                vec![(
                    "invariantShouldFail()",
                    false,
                    Some("revert: it's false".into()),
                    None,
                    None,
                )],
            ),
        ]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_override() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantReentrancy.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.fail_on_revert = false;
        config.invariant.call_override = true;
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantReentrancy.t.sol:InvariantReentrancy",
            vec![("invariantNotStolen()", false, Some("revert: stolen".into()), None, None)],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_fail_on_revert() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantHandlerFailure.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 1;
        config.invariant.depth = 10;
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantHandlerFailure.t.sol:InvariantHandlerFailure",
            vec![(
                "statefulFuzz_BrokenInvariant()",
                false,
                Some("revert: failed on revert".into()),
                None,
                None,
            )],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_invariant_storage() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/storage/InvariantStorageTest.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.depth = 100;
        if cfg!(windows) {
            config.invariant.depth += 50;
        }
        config.fuzz.seed = Some(U256::from(6u32));
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/storage/InvariantStorageTest.t.sol:InvariantStorageTest",
            vec![
                ("invariantChangeAddress()", false, Some("changedAddr".to_string()), None, None),
                ("invariantChangeString()", false, Some("changedString".to_string()), None, None),
                ("invariantChangeUint()", false, Some("changedUint".to_string()), None, None),
                ("invariantPush()", false, Some("pushUint".to_string()), None, None),
            ],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_inner_contract() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantInnerContract.t.sol");
    let results = TEST_DATA_DEFAULT.runner().test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantInnerContract.t.sol:InvariantInnerContract",
            vec![(
                "invariantHideJesus()",
                false,
                Some("revert: jesus betrayed".into()),
                None,
                None,
            )],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(windows, ignore = "for some reason there's different rng")]
async fn test_invariant_shrink() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantInnerContract.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
        config.optimizer = Some(true);
    });

    match get_counterexample!(runner, &filter) {
        CounterExample::Single(_) => panic!("CounterExample should be a sequence."),
        // `fuzz_seed` at 119 makes this sequence shrinkable from 4 to 2.
        CounterExample::Sequence(sequence) => {
            assert!(sequence.len() <= 3);

            if sequence.len() == 2 {
                // call order should always be preserved
                let create_fren_sequence = sequence[0].clone();
                assert_eq!(
                    create_fren_sequence.contract_name.unwrap(),
                    "default/fuzz/invariant/common/InvariantInnerContract.t.sol:Jesus"
                );
                assert_eq!(create_fren_sequence.signature.unwrap(), "create_fren()");

                let betray_sequence = sequence[1].clone();
                assert_eq!(
                    betray_sequence.contract_name.unwrap(),
                    "default/fuzz/invariant/common/InvariantInnerContract.t.sol:Judas"
                );
                assert_eq!(betray_sequence.signature.unwrap(), "betray()");
            }
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(windows, ignore = "for some reason there's different rng")]
async fn test_invariant_assert_shrink() {
    // ensure assert shrinks to same sequence of 2 as require
    check_shrink_sequence("invariant_with_assert", 2).await;
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(windows, ignore = "for some reason there's different rng")]
async fn test_invariant_require_shrink() {
    // ensure require shrinks to same sequence of 2 as assert
    check_shrink_sequence("invariant_with_require", 2).await;
}

async fn check_shrink_sequence(test_pattern: &str, expected_len: usize) {
    let filter =
        Filter::new(test_pattern, ".*", ".*fuzz/invariant/common/InvariantShrinkWithAssert.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.fuzz.seed = Some(U256::from(100u32));
        config.invariant.runs = 1;
        config.invariant.depth = 15;
    });

    match get_counterexample!(runner, &filter) {
        CounterExample::Single(_) => panic!("CounterExample should be a sequence."),
        CounterExample::Sequence(sequence) => {
            assert_eq!(sequence.len(), expected_len);
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(windows, ignore = "for some reason there's different rng")]
async fn test_shrink_big_sequence() {
    let filter =
        Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantShrinkBigSequence.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
        config.invariant.runs = 1;
        config.invariant.depth = 1000;
    });

    let initial_counterexample = runner
        .test_collect(&filter)
        .values()
        .last()
        .expect("Invariant contract should be testable.")
        .test_results
        .values()
        .last()
        .expect("Invariant contract should be testable.")
        .counterexample
        .clone()
        .unwrap();

    let initial_sequence = match initial_counterexample {
        CounterExample::Single(_) => panic!("CounterExample should be a sequence."),
        CounterExample::Sequence(sequence) => sequence,
    };
    // ensure shrinks to same sequence of 77
    assert_eq!(initial_sequence.len(), 77);

    // test failure persistence
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantShrinkBigSequence.t.sol:ShrinkBigSequenceTest",
            vec![(
                "invariant_shrink_big_sequence()",
                false,
                Some("invariant_shrink_big_sequence replay failure".into()),
                None,
                None,
            )],
        )]),
    );
    let new_sequence = match results
        .values()
        .last()
        .expect("Invariant contract should be testable.")
        .test_results
        .values()
        .last()
        .expect("Invariant contract should be testable.")
        .counterexample
        .clone()
        .unwrap()
    {
        CounterExample::Single(_) => panic!("CounterExample should be a sequence."),
        CounterExample::Sequence(sequence) => sequence,
    };
    // ensure shrinks to same sequence of 77
    assert_eq!(new_sequence.len(), 77);
    // ensure calls within failed sequence are the same as initial one
    for index in 0..77 {
        let new_call = new_sequence.get(index).unwrap();
        let initial_call = initial_sequence.get(index).unwrap();
        assert_eq!(new_call.sender, initial_call.sender);
        assert_eq!(new_call.addr, initial_call.addr);
        assert_eq!(new_call.calldata, initial_call.calldata);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(windows, ignore = "for some reason there's different rng")]
async fn test_shrink_fail_on_revert() {
    let filter =
        Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantShrinkFailOnRevert.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 1;
        config.invariant.depth = 200;
    });

    match get_counterexample!(runner, &filter) {
        CounterExample::Single(_) => panic!("CounterExample should be a sequence."),
        CounterExample::Sequence(sequence) => {
            // ensure shrinks to sequence of 10
            assert_eq!(sequence.len(), 10);
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_preserve_state() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantPreserveState.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.fail_on_revert = true;
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantPreserveState.t.sol:InvariantPreserveState",
            vec![(
                "invariant_preserve_state()",
                false,
                Some("EvmError: Revert".into()),
                None,
                None,
            )],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_with_address_fixture() {
    let mut runner = TEST_DATA_DEFAULT.runner();
    let results = runner.test_collect(&Filter::new(
        ".*",
        ".*",
        ".*fuzz/invariant/common/InvariantCalldataDictionary.t.sol",
    ));
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantCalldataDictionary.t.sol:InvariantCalldataDictionary",
            vec![(
                "invariant_owner_never_changes()",
                false,
                Some("<empty revert data>".into()),
                None,
                None,
            )],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_assume_does_not_revert() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantAssume.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        // Should not treat vm.assume as revert.
        config.invariant.fail_on_revert = true;
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantAssume.t.sol:InvariantAssume",
            vec![("invariant_dummy()", true, None, None, None)],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_assume_respects_restrictions() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantAssume.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 10;
        config.invariant.max_assume_rejects = 1;
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantAssume.t.sol:InvariantAssume",
            vec![(
                "invariant_dummy()",
                false,
                Some("`vm.assume` rejected too many inputs (1 allowed)".into()),
                None,
                None,
            )],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_decode_custom_error() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantCustomError.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.fail_on_revert = true;
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantCustomError.t.sol:InvariantCustomError",
            vec![(
                "invariant_decode_error()",
                false,
                Some("InvariantCustomError(111, \"custom\")".into()),
                None,
                None,
            )],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_fuzzed_selected_targets() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/target/FuzzedTargetContracts.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.fail_on_revert = true;
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([
            (
                "default/fuzz/invariant/target/FuzzedTargetContracts.t.sol:ExplicitTargetContract",
                vec![("invariant_explicit_target()", true, None, None, None)],
            ),
            (
                "default/fuzz/invariant/target/FuzzedTargetContracts.t.sol:DynamicTargetContract",
                vec![(
                    "invariant_dynamic_targets()",
                    false,
                    Some("revert: wrong target selector called".into()),
                    None,
                    None,
                )],
            ),
        ]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_fixtures() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantFixtures.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 100;
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantFixtures.t.sol:InvariantFixtures",
            vec![(
                "invariant_target_not_compromised()",
                false,
                Some("<empty revert data>".into()),
                None,
                None,
            )],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_scrape_values() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantScrapeValues.t.sol");
    let results = TEST_DATA_DEFAULT.runner().test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([
            (
                "default/fuzz/invariant/common/InvariantScrapeValues.t.sol:FindFromReturnValueTest",
                vec![(
                    "invariant_value_not_found()",
                    false,
                    Some("revert: value from return found".into()),
                    None,
                    None,
                )],
            ),
            (
                "default/fuzz/invariant/common/InvariantScrapeValues.t.sol:FindFromLogValueTest",
                vec![(
                    "invariant_value_not_found()",
                    false,
                    Some("revert: value from logs found".into()),
                    None,
                    None,
                )],
            ),
        ]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_roll_fork_handler() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantRollFork.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([
            (
                "default/fuzz/invariant/common/InvariantRollFork.t.sol:InvariantRollForkBlockTest",
                vec![(
                    "invariant_fork_handler_block()",
                    false,
                    Some("revert: too many blocks mined".into()),
                    None,
                    None,
                )],
            ),
            (
                "default/fuzz/invariant/common/InvariantRollFork.t.sol:InvariantRollForkStateTest",
                vec![(
                    "invariant_fork_handler_state()",
                    false,
                    Some("revert: wrong supply".into()),
                    None,
                    None,
                )],
            ),
        ]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_excluded_senders() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantExcludedSenders.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.fail_on_revert = true;
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantExcludedSenders.t.sol:InvariantExcludedSendersTest",
            vec![("invariant_check_sender()", true, None, None, None)],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_after_invariant() {
    // Check failure on passing invariant and failed `afterInvariant` condition
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantAfterInvariant.t.sol");
    let results = TEST_DATA_DEFAULT.runner().test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantAfterInvariant.t.sol:InvariantAfterInvariantTest",
            vec![
                (
                    "invariant_after_invariant_failure()",
                    false,
                    Some("revert: afterInvariant failure".into()),
                    None,
                    None,
                ),
                (
                    "invariant_failure()",
                    false,
                    Some("revert: invariant failure".into()),
                    None,
                    None,
                ),
                ("invariant_success()", true, None, None, None),
            ],
        )]),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invariant_selectors_weight() {
    let filter = Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantSelectorsWeight.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
        config.invariant.runs = 1;
        config.invariant.depth = 10;
    });
    let results = runner.test_collect(&filter);
    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/invariant/common/InvariantSelectorsWeight.t.sol:InvariantSelectorsWeightTest",
            vec![("invariant_selectors_weight()", true, None, None, None)],
        )]),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn test_no_reverts_in_counterexample() {
    let filter =
        Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantSequenceNoReverts.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.fail_on_revert = false;
        // Use original counterexample to test sequence len.
        config.invariant.shrink_run_limit = 0;
    });

    match get_counterexample!(runner, &filter) {
        CounterExample::Single(_) => panic!("CounterExample should be a sequence."),
        CounterExample::Sequence(sequence) => {
            // ensure original counterexample len is 10 (even without shrinking)
            assert_eq!(sequence.len(), 10);
        }
    };
}

// Tests that a persisted failure doesn't fail due to assume revert if test driver is changed.
forgetest_init!(should_not_fail_replay_assume, |prj, cmd| {
    let config = Config {
        invariant: {
            InvariantConfig { fail_on_revert: true, max_assume_rejects: 10, ..Default::default() }
        },
        ..Default::default()
    };
    prj.write_config(config);

    // Add initial test that breaks invariant.
    prj.add_test(
        "AssumeTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract AssumeHandler is Test {
    function fuzzMe(uint256 a) public {
        require(false, "Invariant failure");
    }
}

contract AssumeTest is Test {
    function setUp() public {
        AssumeHandler handler = new AssumeHandler();
    }
    function invariant_assume() public {}
}
     "#,
    )
    .unwrap();

    cmd.args(["test", "--mt", "invariant_assume"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: revert: Invariant failure]
...
"#]]);

    // Change test to use assume instead require. Same test should fail with too many inputs
    // rejected message instead persisted failure revert.
    prj.add_test(
        "AssumeTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract AssumeHandler is Test {
    function fuzzMe(uint256 a) public {
        vm.assume(false);
    }
}

contract AssumeTest is Test {
    function setUp() public {
        AssumeHandler handler = new AssumeHandler();
    }
    function invariant_assume() public {}
}
     "#,
    )
    .unwrap();

    cmd.assert_failure().stdout_eq(str![[r#"
...
[FAIL: `vm.assume` rejected too many inputs (10 allowed)] invariant_assume() (runs: 0, calls: 0, reverts: 0)
...
"#]]);
});

// Test too many inputs rejected for `assumePrecompile`/`assumeForgeAddress`.
// <https://github.com/foundry-rs/foundry/issues/9054>
forgetest_init!(should_revert_with_assume_code, |prj, cmd| {
    let config = Config {
        optimizer: Some(true),
        invariant: {
            InvariantConfig { fail_on_revert: true, max_assume_rejects: 10, ..Default::default() }
        },
        ..Default::default()
    };
    prj.write_config(config);

    // Add initial test that breaks invariant.
    prj.add_test(
        "AssumeTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract BalanceTestHandler is Test {
    address public ref = address(1412323);
    address alice;

    constructor(address _alice) {
        alice = _alice;
    }

    function increment(uint256 amount_, address addr) public {
        assumeNotPrecompile(addr);
        assumeNotForgeAddress(addr);
        assertEq(alice.balance, 100_000 ether);
    }
}

contract BalanceAssumeTest is Test {
    function setUp() public {
        address alice = makeAddr("alice");
        vm.deal(alice, 100_000 ether);
        targetSender(alice);
        BalanceTestHandler handler = new BalanceTestHandler(alice);
        targetContract(address(handler));
    }

    function invariant_balance() public {}
}
     "#,
    )
    .unwrap();

    cmd.args(["test", "--mt", "invariant_balance"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: `vm.assume` rejected too many inputs (10 allowed)] invariant_balance() (runs: 0, calls: 0, reverts: 0)
...
"#]]);
});

// Test proper message displayed if `targetSelector`/`excludeSelector` called with empty selectors.
// <https://github.com/foundry-rs/foundry/issues/9066>
forgetest_init!(should_not_panic_if_no_selectors, |prj, cmd| {
    prj.add_test(
        "NoSelectorTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract TestHandler is Test {}

contract NoSelectorTest is Test {
    bytes4[] selectors;

    function setUp() public {
        TestHandler handler = new TestHandler();
        targetSelector(FuzzSelector({addr: address(handler), selectors: selectors}));
        excludeSelector(FuzzSelector({addr: address(handler), selectors: selectors}));
    }

    function invariant_panic() public {}
}
     "#,
    )
    .unwrap();

    cmd.args(["test", "--mt", "invariant_panic"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: failed to set up invariant testing environment: No contracts to fuzz.] invariant_panic() (runs: 0, calls: 0, reverts: 0)
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/3607>
forgetest_init!(should_show_invariant_metrics, |prj, cmd| {
    prj.add_test(
        "SelectorMetricsTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function setUp() public {
        CounterHandler handler = new CounterHandler();
        AnotherCounterHandler handler1 = new AnotherCounterHandler();
        // targetContract(address(handler1));
    }

    /// forge-config: default.invariant.runs = 10
    /// forge-config: default.invariant.show-metrics = true
    function invariant_counter() public {}

    /// forge-config: default.invariant.runs = 10
    /// forge-config: default.invariant.show-metrics = true
    function invariant_counter2() public {}
}

contract CounterHandler is Test {
    function doSomething(uint256 a) public {
        vm.assume(a < 10_000_000);
        require(a < 100_000);
    }

    function doAnotherThing(uint256 a) public {
        vm.assume(a < 10_000_000);
        require(a < 100_000);
    }
}

contract AnotherCounterHandler is Test {
    function doWork(uint256 a) public {
        vm.assume(a < 10_000_000);
        require(a < 100_000);
    }

    function doWorkThing(uint256 a) public {
        vm.assume(a < 10_000_000);
        require(a < 100_000);
    }
}
     "#,
    )
    .unwrap();

    cmd.args(["test", "--mt", "invariant_"]).assert_success().stdout_eq(str![[r#"
...
[PASS] invariant_counter() (runs: 10, calls: 5000, reverts: [..])

╭-----------------------+----------------+-------+---------+----------╮
| Contract              | Selector       | Calls | Reverts | Discards |
+=====================================================================+
| AnotherCounterHandler | doWork         | [..]  | [..]    | [..]     |
|-----------------------+----------------+-------+---------+----------|
| AnotherCounterHandler | doWorkThing    | [..]  | [..]    | [..]     |
|-----------------------+----------------+-------+---------+----------|
| CounterHandler        | doAnotherThing | [..]  | [..]    | [..]     |
|-----------------------+----------------+-------+---------+----------|
| CounterHandler        | doSomething    | [..]  | [..]    | [..]     |
╰-----------------------+----------------+-------+---------+----------╯

[PASS] invariant_counter2() (runs: 10, calls: 5000, reverts: [..])

╭-----------------------+----------------+-------+---------+----------╮
| Contract              | Selector       | Calls | Reverts | Discards |
+=====================================================================+
| AnotherCounterHandler | doWork         | [..]  | [..]    | [..]     |
|-----------------------+----------------+-------+---------+----------|
| AnotherCounterHandler | doWorkThing    | [..]  | [..]    | [..]     |
|-----------------------+----------------+-------+---------+----------|
| CounterHandler        | doAnotherThing | [..]  | [..]    | [..]     |
|-----------------------+----------------+-------+---------+----------|
| CounterHandler        | doSomething    | [..]  | [..]    | [..]     |
╰-----------------------+----------------+-------+---------+----------╯

Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
});

// Tests that invariant exists with success after configured timeout.
forgetest_init!(should_apply_configured_timeout, |prj, cmd| {
    // Add initial test that breaks invariant.
    prj.add_test(
        "TimeoutTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract TimeoutHandler is Test {
    uint256 public count;

    function increment() public {
        count++;
    }
}

contract TimeoutTest is Test {
    TimeoutHandler handler;

    function setUp() public {
        handler = new TimeoutHandler();
    }

    /// forge-config: default.invariant.runs = 10000
    /// forge-config: default.invariant.depth = 20000
    /// forge-config: default.invariant.timeout = 1
    function invariant_counter_timeout() public view {
        // Invariant will fail if more than 10000 increments.
        // Make sure test timeouts after one second and remaining runs are canceled.
        require(handler.count() < 10000);
    }
}
     "#,
    )
    .unwrap();

    cmd.args(["test", "--mt", "invariant_counter_timeout"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/TimeoutTest.t.sol:TimeoutTest
[PASS] invariant_counter_timeout() (runs: 0, calls: 0, reverts: 0)
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});
