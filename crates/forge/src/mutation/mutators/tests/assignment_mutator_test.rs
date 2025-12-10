use crate::mutation::mutators::{
    assignment_mutator::AssignmentMutator,
    tests::helper::{MutatorTestCase, MutatorTester},
};

use rstest::*;

impl MutatorTester for AssignmentMutator {}

#[rstest]
#[case::assign_lit("x = y", Some(vec!["x = 0", "x = -y"]))]
#[case::assign_number("x = 123", Some(vec!["x = 0", "x = -123"]))]
#[case::assign_bool("x = true", Some(vec!["x = false"]))]
#[case::assign_bool("x = false", Some(vec!["x = true"]))]
#[case::assign_declare("uint256 x = 123", Some(vec!["uint256 x = 0", "uint256 x = -123"]))]
#[case::non_assign("a = b + c", None)]
fn test_mutator_assignment(
    #[case] input: &'static str,
    #[case] expected_mutations: Option<Vec<&'static str>>,
) {
    let mutator: AssignmentMutator = AssignmentMutator;
    let test_case = MutatorTestCase { input, expected_mutations };
    AssignmentMutator::test_mutator(mutator, test_case);
}
