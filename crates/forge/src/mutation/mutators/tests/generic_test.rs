use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{
        assignement_mutator::AssignmentMutator, tests::helper::*, MutationContext, Mutator,
    },
    visitor::AssignVarTypes,
    Session,
};
use solar_parse::{
    ast::{
        interface::source_map::FileName, Arena, ElementaryType, Expr, ExprKind, Ident, Item, Lit,
        LitKind, Span, Symbol, Type, TypeKind, VariableDefinition,
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
    /// True if the mutator should be appliable to the input
    pub should_apply: bool,
    /// All the mutations expected for this input, using this mutator
    pub expected_mutations: Option<Vec<&'static str>>,
}

pub fn create_test_context<'a>(input: &'a Item) -> MutationContext<'a> {
    todo!()
}

pub trait MutatorTester {
    fn test_mutator<M: Mutator>(mutator: M, test_cases: Vec<MutatorTestCase>) {
        for case in test_cases {
            let arena = Arena::new();
            let span = create_span(10, 20);
            let sess = Session::builder().with_silent_emitter(None).build();

            let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
                let context = create_test_context(case.input);

                let mutants = mutator.generate_mutants(&context).unwrap();

                println!("Testing case: {}", case.name);

                let mut parser = Parser::from_lazy_source_code(
                    &sess,
                    &arena,
                    FileName::Real(PathBuf::from(case.input)),
                    || Ok(case.input.to_string()),
                )?;

                let ast = parser.parse_file().map_err(|e| e.emit())?;

                let context = create_test_context(&ast.items[0]);

                assert_eq!(
                    mutator.is_applicable(&context),
                    case.should_apply,
                    "is_applicable failed for case: {}",
                    case.name
                );

                // @todo test mutants content...
                if let Some(expected) = case.expected_mutations {
                    if case.should_apply {
                        let mutants = mutator.generate_mutants(&context)?;
                        assert_eq!(
                            mutants.len(),
                            expected.len(),
                            "Wrong number of mutants generated for case: {}",
                            case.name
                        );
                    }
                }

                Ok(())
            });
        }
    }
}

// Implement for unit test module
impl MutatorTester for () {}
