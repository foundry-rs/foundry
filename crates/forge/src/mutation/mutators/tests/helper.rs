use crate::mutation::{Session, mutators::Mutator, visitor::MutantVisitor};
use solar_parse::{
    Parser,
    ast::{Arena, interface::source_map::FileName, visit::Visit},
};

use std::path::PathBuf;
pub struct MutatorTestCase<'a> {
    /// @dev needs to be in a function, to avoid parsing error from solar
    /// eg `let input = "function f() { x = 1; }"` to test x = 1
    pub input: &'a str,
    /// All the mutations expected for this input, using this mutator
    pub expected_mutations: Option<Vec<&'static str>>,
}

pub trait MutatorTester {
    fn test_mutator<M: Mutator + 'static>(mutator: M, test_case: MutatorTestCase<'_>) {
        let sess = Session::builder().with_silent_emitter(None).build();

        // let mut mutations: Vec<Mutant> = Vec::new();
        let mut mutant_visitor = MutantVisitor::new_with_mutators(
            PathBuf::from(test_case.input),
            vec![Box::new(mutator)],
        );

        let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
            let arena = Arena::new();

            let mut parser = Parser::from_lazy_source_code(
                &sess,
                &arena,
                FileName::Real(PathBuf::from(test_case.input)),
                || Ok(test_case.input.to_string()),
            )?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let _ = mutant_visitor.visit_source_unit(&ast);

            let mutations = mutant_visitor.mutation_to_conduct;

            // @todo test mutants content...
            if let Some(expected) = test_case.expected_mutations {
                assert_eq!(mutations.len(), expected.len());

                for mutation in mutations {
                    assert!(expected.contains(&mutation.mutation.to_string().as_str()));
                }
            } else {
                assert_eq!(mutations.len(), 0);
            }

            Ok(())
        });
    }
}

// Implement for unit test module
impl MutatorTester for () {}
