use crate::mutation::mutators::{
    delete_expression_mutator::DeleteExpressionMutator, tests::helper::mutator_tests,
};

mutator_tests!(DeleteExpressionMutator;
    delete_expr: "delete x"  => Some(vec!["x"]);
    non_delete:  "a = b + c" => None;
);
