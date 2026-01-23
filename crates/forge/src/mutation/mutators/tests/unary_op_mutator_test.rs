use rstest::*;

use crate::mutation::mutators::{
    tests::helper::{MutatorTestCase, MutatorTester},
    unary_op_mutator::UnaryOperatorMutator,
};

impl MutatorTester for UnaryOperatorMutator {}

#[rstest]
#[case::pre_inc("++x", Some(vec!["--x", "~x", "-x", "x++", "x--"]))]
#[case::pre_dec("--x", Some(vec!["++x", "~x", "-x", "x++", "x--"]))]
#[case::neg("-x",      Some(vec!["++x", "--x", "~x", "x++", "x--"]))]
#[case::bit_not("~x",  Some(vec!["++x", "--x", "-x", "x++", "x--"]))]
#[case::post_inc("x++",Some(vec!["++x", "--x", "~x", "-x", "x--"]))]
#[case::post_dec("x--",Some(vec!["++x", "--x", "~x", "-x", "x++"]))]
#[case::bool("!x", Some(vec!["x"]))]
#[case::non_unary("a = b + c", None)]
fn test_unary_op_mutator_arithmetic(
    #[case] input: &'static str,
    #[case] expected_mutations: Option<Vec<&'static str>>,
) {
    let mutator: UnaryOperatorMutator = UnaryOperatorMutator;
    let test_case = MutatorTestCase { input, expected_mutations };
    UnaryOperatorMutator::test_mutator(mutator, test_case);
}
