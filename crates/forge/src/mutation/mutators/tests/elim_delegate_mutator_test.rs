use crate::mutation::mutators::{
    elim_delegate_mutator::ElimDelegateMutator, tests::helper::mutator_tests,
};

// The mutator narrows the replacement span to just the `delegatecall`
// identifier and rewrites it to `call`, so the emitted mutation text is
// just `"call"`. It only matches plain `<expr>.delegatecall(args)` Call
// expressions; the variant with `{value: ...}` parses as a `CallOptions`
// wrapper and is intentionally left to a follow-up.
mutator_tests!(ElimDelegateMutator;
    delegate_expr: "target.delegatecall(data)" => Some(vec!["call"]);
    non_delegate:  "target.call(data)"         => None;
);
