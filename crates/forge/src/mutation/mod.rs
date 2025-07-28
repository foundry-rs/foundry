mod mutant;
mod mutators;
mod reporter;
mod visitor;

// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to
// select mutants) Use Solar:
use solar_parse::{
    Parser,
    ast::interface::{Session, source_map::FileName},
};
use std::sync::Arc;

use crate::mutation::{mutant::Mutant, visitor::MutantVisitor};

pub use crate::mutation::reporter::MutationReporter;

use crate::result::TestOutcome;
use foundry_compilers::{ProjectCompileOutput, project::ProjectCompiler};
use foundry_config::Config;
use rayon::prelude::*;
use solar_parse::ast::visit::Visit;
use std::path::{Path, PathBuf};

pub struct MutationsSummary {
    total: usize,
    dead: Vec<Mutant>,
    survived: Vec<Mutant>,
    invalid: Vec<Mutant>,
}

impl MutationsSummary {
    pub fn new() -> Self {
        Self { total: 0, dead: vec![], survived: vec![], invalid: vec![] }
    }

    pub fn update_valid_mutant(&mut self, outcome: &TestOutcome, mutant: Mutant) {
        if outcome.failures().count() > 0 {
            self.dead.push(mutant);
        } else {
            self.survived.push(mutant);
        }
    }

    pub fn update_invalid_mutant(&mut self, mutant: Mutant) {
        self.invalid.push(mutant);
    }

    pub fn total_mutants(&self) -> usize {
        self.dead.len() + self.survived.len() + self.invalid.len()
    }

    pub fn total_dead(&self) -> usize {
        self.dead.len()
    }

    pub fn total_survived(&self) -> usize {
        self.survived.len()
    }

    pub fn total_invalid(&self) -> usize {
        self.invalid.len()
    }

    pub fn dead(&self) -> String {
        self.dead.iter().map(|m| m.to_string()).collect::<Vec<String>>().join("\n")
    }

    pub fn survived(&self) -> String {
        self.survived.iter().map(|m| m.to_string()).collect::<Vec<String>>().join("\n")
    }

    pub fn invalid(&self) -> String {
        self.invalid.iter().map(|m| m.to_string()).collect::<Vec<String>>().join("\n")
    }
}

pub struct MutationHandler {
    contract_to_mutate: PathBuf,
    src: Arc<String>,
    pub mutations: Vec<Mutant>,
    config: Arc<foundry_config::Config>,
    report: MutationsSummary,
}

impl MutationHandler {
    pub fn new(contract_to_mutate: PathBuf, config: Arc<foundry_config::Config>) -> Self {
        Self {
            contract_to_mutate,
            src: Arc::default(),
            mutations: vec![],
            config,
            report: MutationsSummary::new(),
        }
    }

    pub fn read_source_contract(&mut self) -> Result<(), std::io::Error> {
        let content = std::fs::read_to_string(&self.contract_to_mutate)?;
        self.src = Arc::new(content);
        Ok(())
    }

    /// Read a source string, and for each contract found, gets its ast and visit it to list
    /// all mutations to conduct
    pub async fn generate_ast(&mut self) {
        let path = &self.contract_to_mutate;
        let target_content = Arc::clone(&self.src);
        let sess = Session::builder().with_silent_emitter(None).build();

        let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
            let arena = solar_parse::ast::Arena::new();
            let mut parser =
                Parser::from_lazy_source_code(&sess, &arena, FileName::from(path.clone()), || {
                    Ok((*target_content).to_string())
                })?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let mut mutant_visitor = MutantVisitor::default(path.clone());
            mutant_visitor.visit_source_unit(&ast);
            self.mutations.extend(mutant_visitor.mutation_to_conduct);
            Ok(())
        });
    }

    /// Based on a given mutation, emit the corresponding mutated solidity code and write it to disk
    pub fn generate_mutated_solidity(&self, mutation: &Mutant) {
        let span = mutation.span;
        let replacement = mutation.mutation.to_string();

        let src_content = Arc::clone(&self.src);

        let start_pos = span.lo().0 as usize;
        let end_pos = span.hi().0 as usize;

        let before = &src_content[..start_pos];
        let after = &src_content[end_pos..];

        let mut new_content = String::with_capacity(before.len() + replacement.len() + after.len());
        new_content.push_str(before);
        new_content.push_str(&replacement);
        new_content.push_str(after);

        std::fs::write(&self.contract_to_mutate, new_content).unwrap_or_else(|_| {
            panic!("Failed to write to target file {:?}", &self.contract_to_mutate)
        });
    }

    // @todo src should be in a tmp dir for safety!
    /// Restore the original source contract to the target file (end of mutation tests)
    pub fn restore_original_source(&self) {
        std::fs::write(&self.contract_to_mutate, &*self.src).unwrap_or_else(|_| {
            panic!("Failed to write to target file {:?}", &self.contract_to_mutate)
        });
    }
}
