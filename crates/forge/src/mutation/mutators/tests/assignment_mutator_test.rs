use crate::mutation::mutators::{
    assignment_mutator::AssignmentMutator, tests::helper::mutator_tests,
};

// Each emitted mutation only carries the *replacement text* for the RHS
// span — not the full statement. So `x = 123` mutates to `0` (zero) and
// `-123` (signed-negation), not `x = 0` / `x = -123`.
mutator_tests!(AssignmentMutator;
    assign_lit:     "x = y"            => Some(vec!["0", "-y"]);
    assign_number:  "x = 123"          => Some(vec!["0", "-123"]);
    assign_zero:    "x = 0"            => None;
    assign_true:    "x = true"         => Some(vec!["false"]);
    assign_false:   "x = false"        => Some(vec!["true"]);
    assign_declare: "uint256 x = 123"  => Some(vec!["0", "-123"]);
    declare_zero:   "uint256 x = 0"    => None;
    non_assign:     "a = b + c"        => None;
);
