use crate::mutation::{Session, mutators::Mutator, visitor::MutantVisitor};
use solar::{
    ast::{Arena, interface::source_map::FileName, visit::Visit},
    parse::Parser,
};

use std::path::PathBuf;

pub struct MutatorTestCase<'a> {
    /// Source code to test - should be valid Solidity code
    /// e.g., `"// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;\ncontract C { function f() { x = 1; } }"`
    pub input: &'a str,
    /// All the mutations expected for this input, using this mutator
    pub expected_mutations: Option<Vec<&'static str>>,
}

pub trait MutatorTester {
    fn test_mutator<M: Mutator + 'static>(mutator: M, test_case: MutatorTestCase<'_>) {
        let sess = Session::builder().with_silent_emitter(None).build();

        let _ = sess.enter(|| -> solar::interface::Result<()> {
            let arena = Arena::new();

            let mut parser = Parser::from_lazy_source_code(
                &sess,
                &arena,
                FileName::Real(PathBuf::from("test.sol")),
                || Ok(test_case.input.to_string()),
            )?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let mut mutant_visitor = MutantVisitor::new_with_mutators(
                PathBuf::from("test.sol"),
                vec![Box::new(mutator)],
            )
            .with_source(test_case.input);

            let _ = mutant_visitor.visit_source_unit(&ast);

            let mutations = mutant_visitor.mutation_to_conduct;

            if let Some(expected) = test_case.expected_mutations {
                assert_eq!(
                    mutations.len(),
                    expected.len(),
                    "Expected {} mutations, got {}: {:?}",
                    expected.len(),
                    mutations.len(),
                    mutations.iter().map(|m| m.mutation.to_string()).collect::<Vec<_>>()
                );

                for mutation in &mutations {
                    let mutation_str = mutation.mutation.to_string();
                    assert!(
                        expected.contains(&mutation_str.as_str()),
                        "Unexpected mutation: {mutation_str}. Expected one of: {expected:?}",
                    );
                }
            } else {
                assert_eq!(
                    mutations.len(),
                    0,
                    "Expected no mutations, got {}: {:?}",
                    mutations.len(),
                    mutations.iter().map(|m| m.mutation.to_string()).collect::<Vec<_>>()
                );
            }

            Ok(())
        });
    }
}

// Implement for unit test module
impl MutatorTester for () {}
