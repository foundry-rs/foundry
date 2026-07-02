//! Symbolic parity tests against the standard fuzzer corpora.
//!
//! Each submodule mirrors one upstream corpus (Echidna, Medusa, Halmos, hevm,
//! KEVM, Manticore, ItyFuzz, crytic/properties, devdacian, OpenZeppelin,
//! Scribble, SWC). Cases are tiny, bounded reproductions so they can run in
//! CI as unit tests against the symbolic engine. The goal is to verify the
//! engine finds the same counterexamples (or proves the property) the
//! corresponding fuzzers do on these benchmarks.

pub use crate::test_cmd::symbolic_helpers::{
    assert_relevant_lines, assert_symbolic, assert_symbolic_witness, json_test_result,
    read_artifact_ref,
};

mod crytic;
mod devdacian;
mod echidna;
mod halmos;
mod hevm;
mod ityfuzz;
mod kevm;
mod manticore;
mod medusa;
mod openzeppelin;
mod scribble;
mod swc;
