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
fn test_unary_op_mutator_arithmetic(
    #[case] input: &'static str,
    #[case] expected_mutations: Option<Vec<&'static str>>,
) {
    let mutator: UnaryOperatorMutator = UnaryOperatorMutator;
    let test_case = MutatorTestCase { input, expected_mutations };
    UnaryOperatorMutator::test_mutator(mutator, test_case);
}

#[test]
fn test_unary_op_mutator_non_unary() {
    let mutator: UnaryOperatorMutator = UnaryOperatorMutator;
    let test_case =
        MutatorTestCase { input: "function f() { a = b + c; }", expected_mutations: None };
    UnaryOperatorMutator::test_mutator(mutator, test_case);
}

#[test]
fn test_unary_op_mutator_bool() {
    let mutator: UnaryOperatorMutator = UnaryOperatorMutator;
    let test_case =
        MutatorTestCase { input: "function f() { !a; }", expected_mutations: Some(vec!["a"]) };
    UnaryOperatorMutator::test_mutator(mutator, test_case);
}
