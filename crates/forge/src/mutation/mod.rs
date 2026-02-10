use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

pub use crate::mutation::{
    orchestrator::{MutationRunConfig, MutationRunResult, run_mutation_testing},
    progress::MutationProgress,
    reporter::MutationReporter,
    runner::{
        MutantTestResult, ParallelMutationRunner, run_mutations_parallel,
        run_mutations_parallel_with_progress,
    },
};
use crate::{
    mutation::{
        mutant::{Mutant, MutationResult},
        visitor::MutantVisitor,
    },
    result::TestOutcome,
};
use serde::Serialize;
use solar::{
    ast::{
        Span,
        interface::{Session, source_map::FileName},
        visit::Visit,
    },
    parse::Parser,
};

pub mod mutant;
mod mutators;
pub mod orchestrator;
pub mod progress;
mod reporter;
pub mod runner;
mod visitor;

pub struct MutationsSummary {
    dead: Vec<Mutant>,
    survived: Vec<Mutant>,
    invalid: Vec<Mutant>,
    skipped: Vec<Mutant>,
}

impl Default for MutationsSummary {
    fn default() -> Self {
        Self::new()
    }
}

impl MutationsSummary {
    pub fn new() -> Self {
        Self { dead: vec![], survived: vec![], invalid: vec![], skipped: vec![] }
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

    pub fn add_skipped_mutant(&mut self, mutant: Mutant) {
        self.skipped.push(mutant);
    }

    pub fn total_mutants(&self) -> usize {
        self.dead.len() + self.survived.len() + self.invalid.len() + self.skipped.len()
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

    pub fn total_skipped(&self) -> usize {
        self.skipped.len()
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

    pub fn skipped(&self) -> String {
        self.skipped.iter().map(|m| m.to_string()).collect::<Vec<String>>().join("\n")
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

    pub fn get_skipped(&self) -> &Vec<Mutant> {
        &self.skipped
    }

    /// Merge another MutationsSummary into this one
    pub fn merge(&mut self, other: &Self) {
        self.dead.extend(other.dead.clone());
        self.survived.extend(other.survived.clone());
        self.invalid.extend(other.invalid.clone());
        self.skipped.extend(other.skipped.clone());
    }

    /// Calculate mutation score (percentage of dead mutants out of valid mutants)
    /// Higher scores indicate better test coverage
    pub fn mutation_score(&self) -> f64 {
        let valid_mutants = self.dead.len() + self.survived.len();
        if valid_mutants == 0 { 0.0 } else { self.dead.len() as f64 / valid_mutants as f64 * 100.0 }
    }

    /// Convert to JSON output format
    pub fn to_json_output(&self, duration_secs: f64) -> MutationJsonOutput {
        let mut survived_mutants: HashMap<String, Vec<SurvivedMutantJson>> = HashMap::new();

        for mutant in &self.survived {
            let file_path = mutant.relative_path();
            let entry = survived_mutants.entry(file_path).or_default();
            entry.push(SurvivedMutantJson::from_mutant(mutant));
        }

        MutationJsonOutput {
            summary: MutationSummaryJson {
                total: self.total_mutants(),
                killed: self.total_dead(),
                survived: self.total_survived(),
                invalid: self.total_invalid(),
                skipped: self.total_skipped(),
                mutation_score: self.mutation_score(),
                duration_secs,
            },
            survived_mutants,
        }
    }
}

/// JSON output for mutation testing results
#[derive(Debug, Clone, Serialize)]
pub struct MutationJsonOutput {
    pub summary: MutationSummaryJson,
    pub survived_mutants: HashMap<String, Vec<SurvivedMutantJson>>,
}

/// Summary section of JSON output
#[derive(Debug, Clone, Serialize)]
pub struct MutationSummaryJson {
    pub total: usize,
    pub killed: usize,
    pub survived: usize,
    pub invalid: usize,
    pub skipped: usize,
    pub mutation_score: f64,
    pub duration_secs: f64,
}

/// Individual survived mutant in JSON output
#[derive(Debug, Clone, Serialize)]
pub struct SurvivedMutantJson {
    pub line: usize,
    pub column: usize,
    pub original: String,
    pub mutant: String,
}

impl SurvivedMutantJson {
    /// Create from a Mutant, using the full original expression
    pub fn from_mutant(mutant: &Mutant) -> Self {
        Self {
            line: mutant.line_number,
            column: mutant.column_number,
            original: mutant.original.clone(),
            mutant: mutant.mutation.to_string(),
        }
    }
}

/// Tracks spans where mutations have survived (weren't killed by tests).
/// Used for adaptive mutation testing to skip redundant mutations.
#[derive(Debug, Clone, Default)]
pub struct SurvivedSpans {
    spans: HashSet<(u32, u32)>, // (lo, hi) byte positions
}

impl SurvivedSpans {
    pub fn new() -> Self {
        Self { spans: HashSet::new() }
    }

    /// Mark a span as having a surviving mutation
    pub fn mark_survived(&mut self, span: Span) {
        self.spans.insert((span.lo().0, span.hi().0));
    }

    /// Check if this span or any parent span has a surviving mutation
    pub fn should_skip(&self, span: Span) -> bool {
        let (lo, hi) = (span.lo().0, span.hi().0);

        // Check if this exact span has survived
        if self.spans.contains(&(lo, hi)) {
            return true;
        }

        // Check if any parent span (that contains this span) has survived
        for &(parent_lo, parent_hi) in &self.spans {
            if parent_lo <= lo && hi <= parent_hi && (parent_lo != lo || parent_hi != hi) {
                return true;
            }
        }

        false
    }

    /// Serialize to a list of (lo, hi) pairs for caching
    fn to_vec(&self) -> Vec<(u32, u32)> {
        self.spans.iter().copied().collect()
    }

    /// Deserialize from a list of (lo, hi) pairs
    fn from_vec(pairs: Vec<(u32, u32)>) -> Self {
        Self { spans: pairs.into_iter().collect() }
    }
}

pub struct MutationHandler {
    contract_to_mutate: PathBuf,
    pub src: Arc<String>,
    pub mutations: Vec<Mutant>,
    config: Arc<foundry_config::Config>,
    report: MutationsSummary,
    survived_spans: SurvivedSpans,
}

impl MutationHandler {
    pub fn new(contract_to_mutate: PathBuf, config: Arc<foundry_config::Config>) -> Self {
        Self {
            contract_to_mutate,
            src: Arc::default(),
            mutations: vec![],
            config,
            report: MutationsSummary::new(),
            survived_spans: SurvivedSpans::new(),
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

    pub fn add_skipped_mutant(&mut self, mutant: Mutant) {
        self.report.add_skipped_mutant(mutant);
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

    fn cache_filename_prefix(&self) -> String {
        use std::{
            collections::hash_map::DefaultHasher,
            hash::{Hash, Hasher},
        };
        let mut hasher = DefaultHasher::new();
        self.contract_to_mutate.hash(&mut hasher);
        let path_hash = hasher.finish();
        let stem =
            self.contract_to_mutate.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
        format!("{stem}_{path_hash:x}")
    }

    /// Persists cached mutants using build hash for cache invalidation.
    /// Writes to `cache/mutation/<hash>_<filename>.mutants`.
    pub fn persist_cached_mutants(&self, hash: &str, mutants: &[Mutant]) -> std::io::Result<()> {
        let cache_dir = self.config.root.join(&self.config.mutation_dir);
        std::fs::create_dir_all(&cache_dir)?;

        let prefix = self.cache_filename_prefix();
        let cache_file = cache_dir.join(format!("{hash}_{prefix}.mutants"));
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

        let prefix = self.cache_filename_prefix();
        let cache_file = cache_dir.join(format!("{hash}_{prefix}.results"));
        let json = serde_json::to_string_pretty(results).map_err(std::io::Error::other)?;
        std::fs::write(cache_file, json)?;

        Ok(())
    }

    /// Read a source string, and for each contract found, gets its ast and visit it to list
    /// all mutations to conduct.
    ///
    /// When `silent` is true, suppresses informational output (for JSON mode).
    pub async fn generate_ast(&mut self, silent: bool) -> eyre::Result<()> {
        let path = &self.contract_to_mutate;
        let target_content = Arc::clone(&self.src);
        let sess = Session::builder().with_silent_emitter(None).build();

        let survived_spans_clone = self.survived_spans.clone();

        let result = sess.enter(|| -> solar::interface::Result<Vec<Mutant>> {
            let arena = solar::ast::Arena::new();
            let mut parser =
                Parser::from_lazy_source_code(&sess, &arena, FileName::from(path.clone()), || {
                    Ok((*target_content).to_string())
                })?;

            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let operators = self.config.mutation.enabled_operators();
            let mut mutant_visitor = MutantVisitor::with_operators(path.clone(), &operators)
                .with_span_filter(move |span| survived_spans_clone.should_skip(span))
                .with_source(&target_content);
            let _ = mutant_visitor.visit_source_unit(&ast);

            if mutant_visitor.skipped_count > 0 && !silent {
                let _ = sh_println!(
                    "Adaptive mutation: Skipped {} mutation points (already have surviving mutations)",
                    mutant_visitor.skipped_count
                );
            }

            Ok(mutant_visitor.mutation_to_conduct)
        });

        match result {
            Ok(mutations) => {
                self.mutations.extend(mutations);
                Ok(())
            }
            Err(_) => {
                eyre::bail!("failed to parse {}", path.display());
            }
        }
    }

    /// Based on a given mutation, emit the corresponding mutated solidity code and write it to disk
    pub fn generate_mutated_solidity(&self, mutation: &Mutant) -> Result<(), std::io::Error> {
        let span = mutation.span;
        let replacement = mutation.mutation.to_string();
        let src_content = Arc::clone(&self.src);
        let src_bytes = src_content.as_bytes();
        let start_pos = span.lo().0 as usize;
        let end_pos = span.hi().0 as usize;

        if start_pos > end_pos || end_pos > src_bytes.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "invalid span bounds for mutation: {start_pos}..{end_pos} (source len={})",
                    src_bytes.len()
                ),
            ));
        }

        let mut new_content = Vec::with_capacity(src_bytes.len() + replacement.len());
        new_content.extend_from_slice(&src_bytes[..start_pos]);
        new_content.extend_from_slice(replacement.as_bytes());
        new_content.extend_from_slice(&src_bytes[end_pos..]);

        std::fs::write(&self.contract_to_mutate, new_content)
    }

    /// Restore the original source contract to the target file (end of mutation tests)
    pub fn restore_original_source(&self) -> Result<(), std::io::Error> {
        std::fs::write(&self.contract_to_mutate, &*self.src)
    }

    /// Retrieves cached mutants using build hash.
    /// Reads from `cache/mutation/<hash>_<filename>.mutants`.
    pub fn retrieve_cached_mutants(&self, hash: &str) -> Option<Vec<Mutant>> {
        let cache_dir = self.config.root.join(&self.config.mutation_dir);
        let prefix = self.cache_filename_prefix();
        let cache_file = cache_dir.join(format!("{hash}_{prefix}.mutants"));

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
        let prefix = self.cache_filename_prefix();
        let cache_file = cache_dir.join(format!("{hash}_{prefix}.results"));

        if !cache_file.exists() {
            return None;
        }

        let data = std::fs::read_to_string(cache_file).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Mark a span as having a surviving mutation
    pub fn mark_span_survived(&mut self, span: Span) {
        self.survived_spans.mark_survived(span);
    }

    /// Check if a span should be skipped (has survived mutation or is child of survived span)
    pub fn should_skip_span(&self, span: Span) -> bool {
        self.survived_spans.should_skip(span)
    }

    /// Persist survived spans to cache for adaptive mutation testing.
    /// Writes to `cache/mutation/<hash>_<filename>.survived`.
    pub fn persist_survived_spans(&self, hash: &str) -> std::io::Result<()> {
        let cache_dir = self.config.root.join(&self.config.mutation_dir);
        std::fs::create_dir_all(&cache_dir)?;

        let prefix = self.cache_filename_prefix();
        let cache_file = cache_dir.join(format!("{hash}_{prefix}.survived"));

        let spans = self.survived_spans.to_vec();
        let json = serde_json::to_string_pretty(&spans).map_err(std::io::Error::other)?;
        std::fs::write(cache_file, json)?;

        Ok(())
    }

    /// Retrieve survived spans from cache.
    /// Reads from `cache/mutation/<hash>_<filename>.survived`.
    pub fn retrieve_survived_spans(&mut self, hash: &str) -> bool {
        let cache_dir = self.config.root.join(&self.config.mutation_dir);
        let prefix = self.cache_filename_prefix();
        let cache_file = cache_dir.join(format!("{hash}_{prefix}.survived"));

        if !cache_file.exists() {
            return false;
        }

        if let Ok(data) = std::fs::read_to_string(cache_file)
            && let Ok(pairs) = serde_json::from_str::<Vec<(u32, u32)>>(&data)
        {
            self.survived_spans = SurvivedSpans::from_vec(pairs);
            return true;
        }

        false
    }
}
