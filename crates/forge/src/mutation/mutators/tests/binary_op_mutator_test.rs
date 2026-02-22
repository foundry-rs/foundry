use rstest::*;

use crate::mutation::mutators::{
    binary_op_mutator::BinaryOpMutator,
    tests::helper::{MutatorTestCase, MutatorTester},
};

impl MutatorTester for BinaryOpMutator {}

#[rstest]
#[case::add("x + y", Some(vec!["x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]))]
#[case::sub("x - y", Some(vec!["x + y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]))]
#[case::mul("x * y", Some(vec!["x + y", "x - y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]))]
#[case::div("x / y", Some(vec!["x + y", "x - y", "x * y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]))]
#[case::modulus("x % y", Some(vec!["x + y", "x - y", "x * y", "x / y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]))]
#[case::pow("x ** y", Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x << y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]))]
#[case::bit_shift_left("x << y", Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x >> y", "x >>> y", "x & y", "x | y", "x ^ y"]))]
#[case::bit_shift_right("x >> y", Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >>> y", "x & y", "x | y", "x ^ y"]))]
#[case::bit_shift_right_unsigned("x >>> y", Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x & y", "x | y", "x ^ y"]))]
#[case::bit_and("x & y", Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x | y", "x ^ y"]))]
#[case::bit_or("x | y", Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x ^ y"]))]
#[case::bit_xor("x ^ y", Some(vec!["x + y", "x - y", "x * y", "x / y", "x % y", "x ** y", "x << y", "x >> y", "x >>> y", "x & y", "x | y"]))]
#[case::non_binary("a = true", None)]
fn test_mutator_bitwise(
    #[case] input: &'static str,
    #[case] expected_mutations: Option<Vec<&'static str>>,
) {
    let mutator: BinaryOpMutator = BinaryOpMutator;
    let test_case = MutatorTestCase { input, expected_mutations };
    BinaryOpMutator::test_mutator(mutator, test_case);
}
