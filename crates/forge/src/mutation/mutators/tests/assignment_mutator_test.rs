use crate::mutation::mutators::{
    assignment_mutator::AssignmentMutator, tests::helper::mutator_tests,
};

mutator_tests!(AssignmentMutator;
    assign_lit:     "x = y"            => Some(vec!["x = 0", "x = -y"]);
    assign_number:  "x = 123"          => Some(vec!["x = 0", "x = -123"]);
    assign_true:    "x = true"         => Some(vec!["x = false"]);
    assign_false:   "x = false"        => Some(vec!["x = true"]);
    assign_declare: "uint256 x = 123"  => Some(vec!["uint256 x = 0", "uint256 x = -123"]);
    non_assign:     "a = b + c"        => None;
);
