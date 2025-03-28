use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{
        assignement_mutator::AssignmentMutator, tests::helper::*, MutationContext, Mutator,
    },
    visitor::{AssignVarTypes, MutantVisitor},
    Session,
};
use solar_parse::{
    ast::{
        interface::source_map::FileName, visit::Visit, Arena, ElementaryType, Expr, ExprKind,
        Ident, Item, ItemKind, Lit, LitKind, Span, Symbol, Type, TypeKind, VariableDefinition,
    },
    interface::BytePos,
    Parser,
};

use std::path::PathBuf;
pub struct MutatorTestCase<'a> {
    /// eg AssignmentMutator
    pub name: &'static str,
    /// @dev needs to be in a function, to avoid parsing error from solar
    /// eg `let input = "function f() { x = 1; }"` to test x = 1
    pub input: &'a str,
    /// All the mutations expected for this input, using this mutator
    pub expected_mutations: Option<Vec<&'static str>>,
}

pub trait MutatorTester {
    fn test_mutator<M: Mutator + 'static>(mutator: M, test_case: MutatorTestCase<'_>) {
        let arena = Arena::new();
        let sess = Session::builder().with_silent_emitter(None).build();

        // let mut mutations: Vec<Mutant> = Vec::new();
        let mut mutant_visitor = MutantVisitor::new_with_mutators(vec![Box::new(mutator)]);

        let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
            println!("Testing case: {}", test_case.name);

            let mut parser = Parser::from_lazy_source_code(
                &sess,
                &arena,
                FileName::Real(PathBuf::from(test_case.input)),
                || Ok(test_case.input.to_string()),
            )?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            mutant_visitor.visit_source_unit(&ast);

            let mutations = mutant_visitor.mutation_to_conduct;

            // @todo test mutants content...
            if let Some(expected) = test_case.expected_mutations {
                assert_eq!(
                    mutations.len(),
                    expected.len(),
                    "Wrong number of mutants generated for case: {}",
                    test_case.name
                );

                for mutation in mutations {
                    assert!(expected.contains(&mutation.mutation.to_string().as_str()));
                }
            } else {
                assert_eq!(
                    mutations.len(),
                    0,
                    "Mutations should be empty for case: {}",
                    test_case.name
                );
            }

            Ok(())
        });
    }
}

// Implement for unit test module
impl MutatorTester for () {}
