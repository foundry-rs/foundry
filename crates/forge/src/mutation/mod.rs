pub mod mutant;
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

use crate::mutation::{
    mutant::{Mutant, MutationResult},
    visitor::MutantVisitor,
};

pub use crate::mutation::reporter::MutationReporter;

use crate::result::TestOutcome;
use solar_parse::ast::visit::Visit;
use std::path::PathBuf;

pub struct MutationsSummary {
    dead: Vec<Mutant>,
    survived: Vec<Mutant>,
    invalid: Vec<Mutant>,
}

impl Default for MutationsSummary {
    fn default() -> Self {
        Self::new()
    }
}

impl MutationsSummary {
    pub fn new() -> Self {
        Self { dead: vec![], survived: vec![], invalid: vec![] }
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

    pub fn add_dead_mutant(&mut self, mutant: Mutant) {
        self.dead.push(mutant);
    }

    pub fn add_survived_mutant(&mut self, mutant: Mutant) {
        self.survived.push(mutant);
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

    pub fn get_dead(&self) -> &Vec<Mutant> {
        &self.dead
    }

    pub fn get_survived(&self) -> &Vec<Mutant> {
        &self.survived
    }

    pub fn get_invalid(&self) -> &Vec<Mutant> {
        &self.invalid
    }

    /// Merge another MutationsSummary into this one
    pub fn merge(&mut self, other: &MutationsSummary) {
        self.dead.extend(other.dead.clone());
        self.survived.extend(other.survived.clone());
        self.invalid.extend(other.invalid.clone());
    }

    /// Calculate mutation score (percentage of dead mutants out of valid mutants)
    /// Higher scores indicate better test coverage
    pub fn mutation_score(&self) -> f64 {
        let valid_mutants = self.dead.len() + self.survived.len();
        if valid_mutants == 0 { 0.0 } else { self.dead.len() as f64 / valid_mutants as f64 * 100.0 }
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

    /// Add a dead mutant to the report
    pub fn add_dead_mutant(&mut self, mutant: Mutant) {
        self.report.add_dead_mutant(mutant);
    }

    /// Add a survived mutant to the report
    pub fn add_survived_mutant(&mut self, mutant: Mutant) {
        self.report.add_survived_mutant(mutant);
    }

    /// Add an invalid mutant to the report
    pub fn add_invalid_mutant(&mut self, mutant: Mutant) {
        self.report.update_invalid_mutant(mutant);
    }

    /// Get a reference to the current report
    pub fn get_report(&self) -> &MutationsSummary {
        &self.report
    }

    /// Get a mutable reference to the current report
    pub fn get_report_mut(&mut self) -> &mut MutationsSummary {
        &mut self.report
    }

    // Note: we now get the build hash directly from the recent compile output (see test flow)

    /// Persists cached mutants using build hash for cache invalidation.
    /// Writes to `cache/mutation/<hash>_<filename>.mutants`.
    pub fn persist_cached_mutants(&self, hash: &str, mutants: &[Mutant]) -> std::io::Result<()> {
        let cache_dir = self.config.root.join(&self.config.mutation_dir);
        std::fs::create_dir_all(&cache_dir)?;

        let filename =
            self.contract_to_mutate.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
        let cache_file = cache_dir.join(format!("{}_{}.mutants", hash, filename));
        let json = serde_json::to_string_pretty(mutants).map_err(std::io::Error::other)?;
        std::fs::write(cache_file, json)?;

        Ok(())
    }

    /// Persists results for mutants using build hash for cache invalidation.
    /// Writes to `cache/mutation/<hash>_<filename>.results`.
    pub fn persist_cached_results(
        &self,
        hash: &str,
        results: &[(Mutant, crate::mutation::mutant::MutationResult)],
    ) -> std::io::Result<()> {
        let cache_dir = self.config.root.join(&self.config.mutation_dir);
        std::fs::create_dir_all(&cache_dir)?;

        let filename =
            self.contract_to_mutate.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
        let cache_file = cache_dir.join(format!("{}_{}.results", hash, filename));
        let json = serde_json::to_string_pretty(results).map_err(std::io::Error::other)?;
        std::fs::write(cache_file, json)?;

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
            let _ = mutant_visitor.visit_source_unit(&ast);
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

    // @todo src to mutate should be in a tmp dir for safety (and modify config accordingly)
    /// Restore the original source contract to the target file (end of mutation tests)
    pub fn restore_original_source(&self) {
        std::fs::write(&self.contract_to_mutate, &*self.src).unwrap_or_else(|_| {
            panic!("Failed to write to target file {:?}", &self.contract_to_mutate)
        });
    }

    /// Retrieves cached mutants using build hash.
    /// Reads from `cache/mutation/<hash>_<filename>.mutants`.
    pub fn retrieve_cached_mutants(&self, hash: &str) -> Option<Vec<Mutant>> {
        let cache_dir = self.config.root.join(&self.config.mutation_dir);
        let filename =
            self.contract_to_mutate.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
        let cache_file = cache_dir.join(format!("{}_{}.mutants", hash, filename));

        if !cache_file.exists() {
            return None;
        }

        let data = std::fs::read_to_string(cache_file).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Retrieves cached results using build hash.
    /// Reads from `cache/mutation/<hash>_<filename>.results`.
    pub fn retrieve_cached_mutant_results(
        &self,
        hash: &str,
    ) -> Option<Vec<(Mutant, MutationResult)>> {
        let cache_dir = self.config.root.join(&self.config.mutation_dir);
        let filename =
            self.contract_to_mutate.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
        let cache_file = cache_dir.join(format!("{}_{}.results", hash, filename));

        if !cache_file.exists() {
            return None;
        }

        let data = std::fs::read_to_string(cache_file).ok()?;
        serde_json::from_str(&data).ok()
    }
}
