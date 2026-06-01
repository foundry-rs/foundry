use crate::mutation::mutators::{require_mutator::RequireMutator, tests::helper::mutator_tests};

mutator_tests!(RequireMutator;
    require_true:   "require(true)" => Some(vec!["require(false)", "require(!(true))"]);
    require_false:  "require(false)" => Some(vec!["require(true)", "require(!(false))"]);
    require_not:    "require(!paused, \"paused\")" => Some(vec![
        "require(false, \"paused\")",
        "require(paused, \"paused\")",
        "require(true, \"paused\")",
    ]);
);
