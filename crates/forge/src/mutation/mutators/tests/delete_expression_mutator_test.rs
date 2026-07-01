use crate::mutation::mutators::{
    delete_expression_mutator::DeleteExpressionMutator, tests::helper::mutator_tests,
};

// `delete x` is replaced by `assert(true)` (a no-op statement) — the test
// expects the mutation's *replacement text*, not the original expression
// stripped of the `delete` keyword.
mutator_tests!(DeleteExpressionMutator;
    delete_expr: "delete x"  => Some(vec!["assert(true)"]);
    non_delete:  "a = b + c" => None;
);
