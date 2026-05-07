use crate::mutation::mutators::{
    elim_delegate_mutator::ElimDelegateMutator, tests::helper::mutator_tests,
};

mutator_tests!(ElimDelegateMutator;
    delegate_expr: "address(this).delegatecall{value: 1 ether}(0)" => Some(vec!["address(this).call{value: 1 ether}(0)"]);
    non_delegate:  "address(this).call{value: 1 ether}(0)"         => None;
);
