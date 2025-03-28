use crate::mutation::mutators::{
    delete_expression_mutator::DeleteExpressionMutator,
    tests::helper::{MutatorTestCase, MutatorTester},
};

use rstest::*;

impl MutatorTester for DeleteExpressionMutator {}

#[rstest]
#[case::delete_expr("delete x", Some(vec!["x"]))]
fn test_mutator_delete_expr(
    #[case] input: &'static str,
    #[case] expected_mutations: Option<Vec<&'static str>>,
) {
    let mutator: DeleteExpressionMutator = DeleteExpressionMutator;
    let test_case = MutatorTestCase { input, expected_mutations };
    DeleteExpressionMutator::test_mutator(mutator, test_case);
}
