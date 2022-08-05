//! Tests for invariants

use crate::{config::*, test_helpers::filter::Filter};
use std::collections::BTreeMap;

#[test]
fn test_invariant() {
    let mut runner = runner();

    let mut opts = TEST_OPTS;
    opts.invariant_call_override = true;
    runner.test_options = opts;

    let results = runner.test(&Filter::new(".*", ".*", ".*fuzz/invariant/"), None, opts).unwrap();

    assert_multiple(
        &results,
        BTreeMap::from([
            (
                "fuzz/invariant/InvariantInnerContract.t.sol:InvariantInnerContract",
                vec![("invariantHideJesus", false, Some("jesus betrayed.".into()), None, None)],
            ),
            (
                "fuzz/invariant/InvariantReentrancy.t.sol:InvariantReentrancy",
                vec![("invariantNotStolen", false, Some("stolen.".into()), None, None)],
            ),
            (
                "fuzz/invariant/InvariantTest1.t.sol:InvariantTest",
                vec![("invariant_neverFalse", false, Some("false.".into()), None, None)],
            ),
            (
                "fuzz/invariant/target/ExcludeContracts.t.sol:ExcludeContracts",
                vec![("invariantTrueWorld", true, None, None, None)],
            ),
            (
                "fuzz/invariant/target/TargetContracts.t.sol:TargetContracts",
                vec![("invariantTrueWorld", true, None, None, None)],
            ),
            (
                "fuzz/invariant/target/TargetSenders.t.sol:TargetSenders",
                vec![("invariantTrueWorld", false, Some("false world.".into()), None, None)],
            ),
            (
                "fuzz/invariant/target/TargetSelectors.t.sol:TargetSelectors",
                vec![("invariantTrueWorld", true, None, None, None)],
            ),
        ]),
    );
}
