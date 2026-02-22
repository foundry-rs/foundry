use rstest::*;

use crate::mutation::mutators::{
    elim_delegate_mutator::ElimDelegateMutator,
    tests::helper::{MutatorTestCase, MutatorTester},
};

impl MutatorTester for ElimDelegateMutator {}

#[rstest]
#[case::delegate_expr("address(this).delegatecall{value: 1 ether}(0)", Some(vec!["address(this).call{value: 1 ether}(0)"]))]
#[case::non_delegate("address(this).call{value: 1 ether}(0)", None)]
fn test_mutator_delegate_expr(
    #[case] input: &'static str,
    #[case] expected_mutations: Option<Vec<&'static str>>,
) {
    let mutator: ElimDelegateMutator = ElimDelegateMutator;
    let test_case = MutatorTestCase { input, expected_mutations };
    ElimDelegateMutator::test_mutator(mutator, test_case);
}
