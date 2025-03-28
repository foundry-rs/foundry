use crate::mutation::mutators::{
    tests::helper::{MutatorTestCase, MutatorTester},
    unary_op_mutator::UnaryOperatorMutator,
};

use rstest::*;

impl MutatorTester for UnaryOperatorMutator {}

#[rstest]
#[case::pre_inc("function f() { ++x; }", Some(vec!["--x", "~x", "-x", "x++", "x--"]))]
#[case::pre_dec("function f() { --x; }", Some(vec!["++x", "~x", "-x", "x++", "x--"]))]
#[case::neg("function f() { -x; }",      Some(vec!["++x", "--x", "~x", "x++", "x--"]))]
#[case::bit_not("function f() { ~x; }",  Some(vec!["++x", "--x", "-x", "x++", "x--"]))]
#[case::post_inc("function f() { x++; }",Some(vec!["++x", "--x", "~x", "-x", "x--"]))]
#[case::post_dec("function f() { x--; }",Some(vec!["++x", "--x", "~x", "-x", "x++"]))]
#[case::bool("function f() { !x; }", Some(vec!["x"]))]
#[case::non_unary("function f() { a = b + c; }", None)]
fn test_unary_op_mutator_arithmetic(
    #[case] input: &'static str,
    #[case] expected_mutations: Option<Vec<&'static str>>,
) {
    let mutator: UnaryOperatorMutator = UnaryOperatorMutator;
    let test_case = MutatorTestCase { input, expected_mutations };
    UnaryOperatorMutator::test_mutator(mutator, test_case);
}
