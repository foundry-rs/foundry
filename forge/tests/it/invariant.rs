//! Tests for invariants

use crate::{config::*, test_helpers::filter::Filter};
use ethers::types::U256;
use forge::fuzz::CounterExample;
use std::collections::BTreeMap;

#[test]
fn test_invariant() {
    let mut runner = runner();

    let results = runner
        .test(
            &Filter::new(".*", ".*", ".*fuzz/invariant/(target|targetAbi|common)"),
            None,
            TEST_OPTS,
        )
        .unwrap();

    assert_multiple(
        &results,
        BTreeMap::from([
            (
                "fuzz/invariant/common/InvariantInnerContract.t.sol:InvariantInnerContract",
                vec![("invariantHideJesus()", false, Some("jesus betrayed.".into()), None, None)],
            ),
            (
                "fuzz/invariant/common/InvariantReentrancy.t.sol:InvariantReentrancy",
                vec![("invariantNotStolen()", true, None, None, None)],
            ),
            (
                "fuzz/invariant/common/InvariantTest1.t.sol:InvariantTest",
                vec![("invariant_neverFalse()", false, Some("false.".into()), None, None)],
            ),
            (
                "fuzz/invariant/target/ExcludeContracts.t.sol:ExcludeContracts",
                vec![("invariantTrueWorld()", true, None, None, None)],
            ),
            (
                "fuzz/invariant/target/TargetContracts.t.sol:TargetContracts",
                vec![("invariantTrueWorld()", true, None, None, None)],
            ),
            (
                "fuzz/invariant/target/TargetSenders.t.sol:TargetSenders",
                vec![("invariantTrueWorld()", false, Some("false world.".into()), None, None)],
            ),
            (
                "fuzz/invariant/target/ExcludeSenders.t.sol:ExcludeSenders",
                vec![("invariantTrueWorld()", true, None, None, None)],
            ),
            (
                "fuzz/invariant/target/TargetSelectors.t.sol:TargetSelectors",
                vec![("invariantTrueWorld()", true, None, None, None)],
            ),
            (
                "fuzz/invariant/targetAbi/ExcludeArtifacts.t.sol:ExcludeArtifacts",
                vec![("invariantShouldPass()", true, None, None, None)],
            ),
            (
                "fuzz/invariant/targetAbi/TargetArtifacts.t.sol:TargetArtifacts",
                vec![
                    ("invariantShouldPass()", true, None, None, None),
                    ("invariantShouldFail()", false, Some("false world.".into()), None, None),
                ],
            ),
            (
                "fuzz/invariant/targetAbi/TargetArtifactSelectors.t.sol:TargetArtifactSelectors",
                vec![("invariantShouldPass()", true, None, None, None)],
            ),
            (
                "fuzz/invariant/targetAbi/TargetArtifactSelectors2.t.sol:TargetArtifactSelectors2",
                vec![("invariantShouldFail()", false, Some("its false.".into()), None, None)],
            ),
        ]),
    );
}

#[test]
fn test_invariant_override() {
    let mut runner = runner();

    let mut opts = TEST_OPTS;
    opts.invariant.call_override = true;
    runner.test_options = opts;

    let results = runner
        .test(
            &Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantReentrancy.t.sol"),
            None,
            opts,
        )
        .unwrap();

    assert_multiple(
        &results,
        BTreeMap::from([(
            "fuzz/invariant/common/InvariantReentrancy.t.sol:InvariantReentrancy",
            vec![("invariantNotStolen()", false, Some("stolen.".into()), None, None)],
        )]),
    );
}

#[test]
fn test_invariant_storage() {
    let mut runner = runner();

    let mut opts = TEST_OPTS;
    opts.invariant.depth = 100;
    opts.fuzz.seed = Some(U256::from(6u32));
    runner.test_options = opts;

    let results = runner
        .test(
            &Filter::new(".*", ".*", ".*fuzz/invariant/storage/InvariantStorageTest.t.sol"),
            None,
            opts,
        )
        .unwrap();

    assert_multiple(
        &results,
        BTreeMap::from([(
            "fuzz/invariant/storage/InvariantStorageTest.t.sol:InvariantStorageTest",
            vec![
                ("invariantChangeAddress()", false, Some("changedAddr".into()), None, None),
                ("invariantChangeString()", false, Some("changedStr".into()), None, None),
                ("invariantChangeUint()", false, Some("changedUint".into()), None, None),
                ("invariantPush()", false, Some("pushUint".into()), None, None),
            ],
        )]),
    );
}

#[test]
// for some reason there's different rng
#[cfg(not(windows))]
fn test_invariant_shrink() {
    let mut runner = runner();

    let mut opts = TEST_OPTS;
    opts.fuzz.seed = Some(U256::from(102u32));
    runner.test_options = opts;

    let results = runner
        .test(
            &Filter::new(".*", ".*", ".*fuzz/invariant/common/InvariantInnerContract.t.sol"),
            None,
            opts,
        )
        .unwrap();

    let results =
        results.values().last().expect("`InvariantInnerContract.t.sol` should be testable.");

    let result =
        results.test_results.values().last().expect("`InvariantInnerContract` should be testable.");

    let counter = result
        .counterexample
        .as_ref()
        .expect("`InvariantInnerContract` should have failed with a counterexample.");

    match counter {
        CounterExample::Single(_) => panic!("CounterExample should be a sequence."),
        // `fuzz_seed` at 100 makes this sequence shrinkable from 4 to 2.
        CounterExample::Sequence(sequence) => assert_eq!(sequence.len(), 2),
    };
}
