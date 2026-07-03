use solar::{
    ast::{Arena, interface::source_map::FileName, visit::Visit},
    parse::Parser,
};

use std::path::PathBuf;

use crate::mutation::{Session, mutators::Mutator, visitor::MutantVisitor};

pub struct MutatorTestCase<'a> {
    /// Source code to test - should be valid Solidity code
    /// e.g., `"// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;\ncontract C { function f()
    /// { x = 1; } }"`
    pub input: &'a str,
    /// All the mutations expected for this input, using this mutator
    pub expected_mutations: Option<Vec<&'static str>>,
}

pub trait MutatorTester {
    fn test_mutator<M: Mutator + 'static>(mutator: M, test_case: MutatorTestCase<'_>) {
        // Wrap the snippet in a minimal but valid Solidity source unit so the
        // parser/visitor actually walks into expression contexts. Bare
        // fragments like `"x + y"` or `"a += b"` are not parseable on their
        // own; without wrapping, the parser would emit errors that the test
        // harness used to swallow, silently making mutator tests vacuous.
        let wrapped = format!(
            "// SPDX-License-Identifier: MIT\n\
             pragma solidity ^0.8.0;\n\
             contract __TestC {{\n\
                 function __test() public {{\n\
                     {input};\n\
                 }}\n\
             }}\n",
            input = test_case.input,
        );

        let sess = Session::builder().with_silent_emitter(None).build();

        let outcome = sess.enter(|| -> solar::interface::Result<Vec<String>> {
            let arena = Arena::new();

            let mut parser = Parser::from_lazy_source_code(
                &sess,
                &arena,
                FileName::Real(PathBuf::from("test.sol")),
                || Ok(wrapped.clone()),
            )?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;
            drop(parser);

            let mut mutant_visitor = MutantVisitor::new_with_mutators(
                PathBuf::from("test.sol"),
                vec![Box::new(mutator)],
            )
            .with_source(&wrapped);

            let _ = mutant_visitor.visit_source_unit(&ast);

            Ok(mutant_visitor
                .mutation_to_conduct
                .into_iter()
                .map(|m| m.mutation.to_string())
                .collect())
        });

        // Surface parse/visit errors instead of silently passing the test.
        let mutations = outcome.unwrap_or_else(|_| {
            panic!(
                "mutator test input failed to parse/visit; wrapped source was:\n{wrapped}\n\
                 raw input: {input:?}",
                input = test_case.input,
            )
        });

        if let Some(expected) = test_case.expected_mutations {
            let mut actual = mutations;
            actual.sort();

            let mut expected = expected.into_iter().map(str::to_string).collect::<Vec<_>>();
            expected.sort();

            assert_eq!(actual, expected, "Unexpected mutation set for input {:?}", test_case.input);
        } else {
            assert!(
                mutations.is_empty(),
                "Expected no mutations, got {}: {:?}",
                mutations.len(),
                mutations,
            );
        }
    }
}

// Implement for unit test module
impl MutatorTester for () {}

/// Generates one `#[test]` function per case for a [`Mutator`].
///
/// Each case becomes a standalone test (parallel execution, individual reporting,
/// IDE run buttons), without pulling in `rstest`.
///
/// # Example
///
/// ```ignore
/// mutator_tests!(UnaryOpMutator;
///     pre_inc:    "++x"       => Some(vec!["--x", "~x", "-x", "x++", "x--"]);
///     non_unary:  "a = b + c" => None;
/// );
/// ```
macro_rules! mutator_tests {
    ($mutator:expr; $($name:ident: $input:expr => $expected:expr);+ $(;)?) => {
        $(
            #[test]
            fn $name() {
                <() as $crate::mutation::mutators::tests::helper::MutatorTester>::test_mutator(
                    $mutator,
                    $crate::mutation::mutators::tests::helper::MutatorTestCase {
                        input: $input,
                        expected_mutations: $expected,
                    },
                );
            }
        )+
    };
}

pub(crate) use mutator_tests;
