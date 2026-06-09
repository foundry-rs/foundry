use std::{
    collections::{BTreeMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::mutation::{
    mutant::{Mutant, MutationResult},
    visitor::MutantVisitor,
};
pub use crate::mutation::{
    orchestrator::{MutationRunConfig, MutationRunResult, run_mutation_testing},
    progress::MutationProgress,
    reporter::MutationReporter,
    runner::run_mutations_parallel_with_progress,
};
use eyre::eyre;
use foundry_common::sh_warn;
use serde::{Deserialize, Serialize};
use solar::{
    ast::{
        Span,
        interface::{Session, source_map::FileName},
        visit::Visit,
    },
    parse::Parser,
};

fn failed_to_parse(path: &Path) -> eyre::Report {
    eyre!("failed to parse {}", path.display())
}

#[derive(Clone, Copy)]
enum CacheKind<'a> {
    Mutants,
    Results { execution_key: &'a str },
    Survived { execution_key: &'a str },
}

#[derive(Serialize, Deserialize)]
struct CachedMutationResults {
    mutant_count: usize,
    mutant_hash: u64,
    results: Vec<(Mutant, MutationResult)>,
}

fn mutant_set_hash(mutants: &[Mutant]) -> u64 {
    let mut entries: Vec<_> = mutants
        .iter()
        .map(|mutant| {
            (
                mutant.span.lo().0,
                mutant.span.hi().0,
                mutant.mutation.to_string(),
                mutant.original.clone(),
            )
        })
        .collect();
    entries.sort();

    let mut hasher = DefaultHasher::new();
    for entry in entries {
        entry.hash(&mut hasher);
    }
    hasher.finish()
}

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
    /// Mutants whose compile-and-test work exceeded the configured timeout.
    /// Tracked separately so they are not counted toward survived/killed.
    timed_out: Vec<Mutant>,
}

impl Default for MutationsSummary {
    fn default() -> Self {
        Self::new()
    }
}

impl MutationsSummary {
    pub const fn new() -> Self {
        Self {
            dead: Vec::new(),
            survived: Vec::new(),
            invalid: Vec::new(),
            skipped: Vec::new(),
            timed_out: Vec::new(),
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

    pub fn add_timed_out_mutant(&mut self, mutant: Mutant) {
        self.timed_out.push(mutant);
    }

    pub const fn total_mutants(&self) -> usize {
        self.dead.len()
            + self.survived.len()
            + self.invalid.len()
            + self.skipped.len()
            + self.timed_out.len()
    }

    pub const fn total_dead(&self) -> usize {
        self.dead.len()
    }

    pub const fn total_survived(&self) -> usize {
        self.survived.len()
    }

    pub const fn total_invalid(&self) -> usize {
        self.invalid.len()
    }

    pub const fn total_skipped(&self) -> usize {
        self.skipped.len()
    }

    pub const fn total_timed_out(&self) -> usize {
        self.timed_out.len()
    }

    pub const fn get_dead(&self) -> &Vec<Mutant> {
        &self.dead
    }

    pub const fn get_survived(&self) -> &Vec<Mutant> {
        &self.survived
    }

    pub const fn get_invalid(&self) -> &Vec<Mutant> {
        &self.invalid
    }

    pub const fn get_timed_out(&self) -> &Vec<Mutant> {
        &self.timed_out
    }

    /// Merge another MutationsSummary into this one
    pub fn merge(&mut self, other: &Self) {
        self.dead.extend(other.dead.clone());
        self.survived.extend(other.survived.clone());
        self.invalid.extend(other.invalid.clone());
        self.skipped.extend(other.skipped.clone());
        self.timed_out.extend(other.timed_out.clone());
    }

    /// Calculate mutation score (percentage of dead mutants out of valid mutants)
    /// Higher scores indicate better test coverage
    pub fn mutation_score(&self) -> f64 {
        let valid_mutants = self.dead.len() + self.survived.len();
        if valid_mutants == 0 { 0.0 } else { self.dead.len() as f64 / valid_mutants as f64 * 100.0 }
    }

    /// Mutants that reached a test verdict and can contribute to the score.
    pub const fn total_evaluated(&self) -> usize {
        self.dead.len() + self.survived.len()
    }

    /// Whether the score is useful enough to present as a coverage signal.
    pub const fn has_reliable_score(&self) -> bool {
        self.total_evaluated() > 0 && self.timed_out.len() < self.total_evaluated()
    }

    /// Convert to JSON output format.
    ///
    /// Output is sorted deterministically: files in lexicographic order
    /// (`BTreeMap` keys), and survived mutants within each file sorted by
    /// `(line, column, original, mutant)`. Without this, parallel worker
    /// completion order leaks into the JSON and breaks downstream diffing,
    /// snapshot tests, and reproducibility.
    pub fn to_json_output(&self, duration_secs: f64) -> MutationJsonOutput {
        let mut survived_mutants: BTreeMap<String, Vec<SurvivedMutantJson>> = BTreeMap::new();

        for mutant in &self.survived {
            let file_path = mutant.relative_path();
            let entry = survived_mutants.entry(file_path).or_default();
            entry.push(SurvivedMutantJson::from_mutant(mutant));
        }

        for entries in survived_mutants.values_mut() {
            entries.sort_by(|a, b| {
                (a.line, a.column, &a.original, &a.mutant).cmp(&(
                    b.line,
                    b.column,
                    &b.original,
                    &b.mutant,
                ))
            });
        }

        MutationJsonOutput {
            summary: MutationSummaryJson {
                total: self.total_mutants(),
                killed: self.total_dead(),
                survived: self.total_survived(),
                invalid: self.total_invalid(),
                skipped: self.total_skipped(),
                timed_out: self.total_timed_out(),
                mutation_score: self.mutation_score(),
                duration_secs,
            },
            survived_mutants,
        }
    }
}

/// JSON output for mutation testing results.
///
/// Uses [`BTreeMap`] for `survived_mutants` so file ordering in the emitted
/// JSON is deterministic.
#[derive(Debug, Clone, Serialize)]
pub struct MutationJsonOutput {
    pub summary: MutationSummaryJson,
    pub survived_mutants: BTreeMap<String, Vec<SurvivedMutantJson>>,
}

/// Summary section of JSON output
#[derive(Debug, Clone, Serialize)]
pub struct MutationSummaryJson {
    pub total: usize,
    pub killed: usize,
    pub survived: usize,
    pub invalid: usize,
    pub skipped: usize,
    pub timed_out: usize,
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

    /// Check if any survived parent span contains this span.
    ///
    /// Exact span matches are not skipped: a persisted survived-span cache only
    /// records byte ranges, not which mutant at that range survived. Re-testing
    /// exact spans after an interrupted run keeps known survivors from being
    /// converted into `Skipped` results in the next complete cache.
    pub fn should_skip(&self, span: Span) -> bool {
        let (lo, hi) = (span.lo().0, span.hi().0);

        self.spans.iter().any(|&(parent_lo, parent_hi)| {
            parent_lo <= lo && hi <= parent_hi && (parent_lo != lo || parent_hi != hi)
        })
    }

    /// Check if any survived span contains this span, including exact matches.
    ///
    /// Live workers know exact same-span mutants are siblings in the current
    /// run, so once one survives the remaining siblings can be skipped.
    pub fn should_skip_in_live_run(&self, span: Span) -> bool {
        let (lo, hi) = (span.lo().0, span.hi().0);

        self.spans.iter().any(|&(parent_lo, parent_hi)| parent_lo <= lo && hi <= parent_hi)
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
    /// Optional regex used to restrict mutation to specific contracts within
    /// the file (matches against contract name).
    contract_filter: Option<regex::Regex>,
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
            contract_filter: None,
        }
    }

    /// Restrict mutation to contracts whose name matches `filter`.
    pub fn with_contract_filter(mut self, filter: regex::Regex) -> Self {
        self.contract_filter = Some(filter);
        self
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

    pub fn add_timed_out_mutant(&mut self, mutant: Mutant) {
        self.report.add_timed_out_mutant(mutant);
    }

    /// Get a reference to the current report
    pub const fn get_report(&self) -> &MutationsSummary {
        &self.report
    }

    // Note: we now get the build hash directly from the recent compile output (see test flow)

    /// Returns the cache file path for the given build hash and cache kind.
    /// The filename encodes a hash of the full contract path to prevent collisions
    /// between files with the same stem in different directories, and a hash of
    /// the active mutation config so changes to enabled operators invalidate
    /// previously cached mutants. Result-like caches also include an execution
    /// key so stale outcomes are not reused after test/config/EVM changes.
    fn cache_file_path(&self, hash: &str, kind: CacheKind<'_>) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        self.contract_to_mutate.hash(&mut hasher);
        let path_hash = hasher.finish();

        // Hash the effective set of enabled mutation operators so mutant cache
        // entries are invalidated when the user changes `include_operators` /
        // `exclude_operators` in their config.
        //
        // Also fold in the active `--mutate-contract` regex pattern, because
        // running with vs. without that filter produces a different mutant set
        // for the same file.
        let mut mutant_cfg_hasher = DefaultHasher::new();
        // Version salt for this mutant-set cache schema. Bump this if the
        // inputs that define generated mutants change.
        "mutant-set-v2".hash(&mut mutant_cfg_hasher);
        for op in self.config.mutation.enabled_operators() {
            op.to_string().hash(&mut mutant_cfg_hasher);
        }
        match self.contract_filter.as_ref() {
            Some(re) => {
                "filter:".hash(&mut mutant_cfg_hasher);
                re.as_str().hash(&mut mutant_cfg_hasher);
            }
            None => "nofilter".hash(&mut mutant_cfg_hasher),
        }
        let mutant_cfg_hash = mutant_cfg_hasher.finish();

        let (ext, execution_suffix) = match kind {
            CacheKind::Mutants => ("mutants", String::new()),
            CacheKind::Results { execution_key } => ("results", format!("_{execution_key}")),
            CacheKind::Survived { execution_key } => ("survived", format!("_{execution_key}")),
        };

        let stem =
            self.contract_to_mutate.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
        self.config.root.join(&self.config.mutation_dir).join(format!(
            "{hash}_{stem}_{path_hash:x}_{mutant_cfg_hash:x}{execution_suffix}.{ext}"
        ))
    }

    /// Persists cached mutants using build hash for cache invalidation.
    pub fn persist_cached_mutants(&self, hash: &str, mutants: &[Mutant]) -> std::io::Result<()> {
        let cache_file = self.cache_file_path(hash, CacheKind::Mutants);
        if let Some(dir) = cache_file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(mutants).map_err(std::io::Error::other)?;
        std::fs::write(cache_file, json)
    }

    /// Persists results for mutants using build hash for cache invalidation.
    pub fn persist_cached_results(
        &self,
        hash: &str,
        execution_key: &str,
        mutants: &[Mutant],
        results: &[(Mutant, crate::mutation::mutant::MutationResult)],
    ) -> std::io::Result<()> {
        let cache_file = self.cache_file_path(hash, CacheKind::Results { execution_key });
        if let Some(dir) = cache_file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let cached = CachedMutationResults {
            mutant_count: mutants.len(),
            mutant_hash: mutant_set_hash(mutants),
            results: results.to_vec(),
        };
        let json = serde_json::to_string_pretty(&cached).map_err(std::io::Error::other)?;
        std::fs::write(cache_file, json)
    }

    /// Read a source string, and for each contract found, gets its ast and visit it to list
    /// all mutations to conduct.
    pub async fn generate_ast(&mut self) -> eyre::Result<()> {
        let path = &self.contract_to_mutate;
        let target_content = Arc::clone(&self.src);
        let sess = Session::builder().with_silent_emitter(None).build();

        let contract_filter = self.contract_filter.clone();

        let result = sess.enter(|| -> eyre::Result<Vec<Mutant>> {
            let arena = solar::ast::Arena::new();
            let mut parser =
                Parser::from_lazy_source_code(&sess, &arena, FileName::from(path.clone()), || {
                    Ok((*target_content).clone())
                })
                .map_err(|_e| failed_to_parse(path))?;

            let ast = parser.parse_file().map_err(|e| {
                e.emit();
                failed_to_parse(path)
            })?;
            drop(parser);

            let operators = self.config.mutation.enabled_operators();
            let mut mutant_visitor = MutantVisitor::with_operators(path.clone(), &operators)
                .with_source(&target_content);

            if let Some(filter) = contract_filter {
                mutant_visitor =
                    mutant_visitor.with_contract_filter(move |name| filter.is_match(name));
            }
            let _ = mutant_visitor.visit_source_unit(&ast);

            for err in mutant_visitor.take_errors() {
                let _ = sh_warn!("{err:?}");
            }

            Ok(mutant_visitor.mutation_to_conduct)
        });

        match result {
            Ok(mutations) => {
                self.mutations.extend(mutations);
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    /// Retrieves cached mutants using build hash.
    pub fn retrieve_cached_mutants(&self, hash: &str) -> Option<Vec<Mutant>> {
        let cache_file = self.cache_file_path(hash, CacheKind::Mutants);
        let data = std::fs::read_to_string(cache_file).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Retrieves cached results using build hash.
    pub fn retrieve_cached_mutant_results(
        &self,
        hash: &str,
        execution_key: &str,
        mutants: &[Mutant],
    ) -> Option<Vec<(Mutant, MutationResult)>> {
        let cache_file = self.cache_file_path(hash, CacheKind::Results { execution_key });
        let data = std::fs::read_to_string(cache_file).ok()?;
        let cached: CachedMutationResults = serde_json::from_str(&data).ok()?;
        (cached.mutant_count == mutants.len() && cached.mutant_hash == mutant_set_hash(mutants))
            .then_some(cached.results)
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
    pub fn persist_survived_spans(&self, hash: &str, execution_key: &str) -> std::io::Result<()> {
        let cache_file = self.cache_file_path(hash, CacheKind::Survived { execution_key });
        if let Some(dir) = cache_file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let spans = self.survived_spans.to_vec();
        let json = serde_json::to_string_pretty(&spans).map_err(std::io::Error::other)?;
        std::fs::write(cache_file, json)
    }

    /// Retrieve survived spans from cache.
    pub fn retrieve_survived_spans(&mut self, hash: &str, execution_key: &str) -> bool {
        let cache_file = self.cache_file_path(hash, CacheKind::Survived { execution_key });

        if let Ok(data) = std::fs::read_to_string(cache_file)
            && let Ok(pairs) = serde_json::from_str::<Vec<(u32, u32)>>(&data)
        {
            self.survived_spans = SurvivedSpans::from_vec(pairs);
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_config::Config;
    use solar::ast::interface::BytePos;
    use tempfile::TempDir;

    fn test_handler(config: Config) -> MutationHandler {
        let source = config.root.join("src").join("Counter.sol");
        MutationHandler::new(source, Arc::new(config))
    }

    fn test_config() -> (TempDir, Config) {
        let temp = TempDir::new().unwrap();
        let config = Config {
            root: temp.path().to_path_buf(),
            mutation_dir: "cache/mutation".into(),
            ..Default::default()
        };
        (temp, config)
    }

    fn mutant(lo: u32, hi: u32, original: &str) -> Mutant {
        Mutant {
            path: PathBuf::from("src/Counter.sol"),
            span: Span::new(BytePos(lo), BytePos(hi)),
            mutation: mutant::MutationType::DeleteExpression,
            original: original.to_string(),
            source_line: "number++;".to_string(),
            line_number: 1,
            column_number: 1,
        }
    }

    #[test]
    fn result_cache_path_includes_execution_key() {
        let (_temp, config) = test_config();
        let handler = test_handler(config);

        let first =
            handler.cache_file_path("build", CacheKind::Results { execution_key: "exec-a" });
        let second =
            handler.cache_file_path("build", CacheKind::Results { execution_key: "exec-b" });
        let mutants = handler.cache_file_path("build", CacheKind::Mutants);

        assert_ne!(first, second);
        assert_ne!(first, mutants);
        assert_ne!(second, mutants);
    }

    #[test]
    fn survived_span_cache_path_includes_execution_key() {
        let (_temp, config) = test_config();
        let handler = test_handler(config);

        let first =
            handler.cache_file_path("build", CacheKind::Survived { execution_key: "exec-a" });
        let second =
            handler.cache_file_path("build", CacheKind::Survived { execution_key: "exec-b" });

        assert_ne!(first, second);
    }

    #[test]
    fn mutant_cache_path_ignores_execution_only_timeout() {
        let (_temp, mut first_config) = test_config();
        let mut second_config = first_config.clone();

        first_config.mutation.timeout = Some(1);
        second_config.mutation.timeout = Some(99);

        let first = test_handler(first_config).cache_file_path("build", CacheKind::Mutants);
        let second = test_handler(second_config).cache_file_path("build", CacheKind::Mutants);

        assert_eq!(first, second);
    }

    #[test]
    fn result_cache_validates_current_mutant_set() {
        let (_temp, config) = test_config();
        let handler = test_handler(config);
        let mutants = vec![mutant(10, 20, "number++")];
        let results = vec![(mutants[0].clone(), MutationResult::Dead)];

        handler.persist_cached_results("build", "exec", &mutants, &results).unwrap();

        assert!(handler.retrieve_cached_mutant_results("build", "exec", &mutants).is_some());

        let changed_mutants = vec![mutant(10, 20, "number--")];
        assert!(
            handler.retrieve_cached_mutant_results("build", "exec", &changed_mutants).is_none()
        );
    }

    #[test]
    fn mutation_score_is_unreliable_when_evaluated_mutants_equal_timeouts() {
        let mut summary = MutationsSummary::new();
        summary.add_dead_mutant(mutant(10, 20, "number++"));
        summary.add_timed_out_mutant(mutant(30, 40, "number--"));

        assert_eq!(summary.total_evaluated(), 1);
        assert!(!summary.has_reliable_score());
    }

    #[test]
    fn mutation_score_is_unreliable_when_timeouts_dominate() {
        let mut summary = MutationsSummary::new();
        summary.add_dead_mutant(mutant(10, 20, "number++"));
        summary.add_timed_out_mutant(mutant(30, 40, "number--"));
        summary.add_timed_out_mutant(mutant(50, 60, "number += 1"));

        assert_eq!(summary.total_evaluated(), 1);
        assert!(!summary.has_reliable_score());
    }

    #[test]
    fn mutation_score_is_unreliable_with_no_evaluated_mutants() {
        let mut summary = MutationsSummary::new();
        summary.add_timed_out_mutant(mutant(10, 20, "number++"));

        assert_eq!(summary.total_evaluated(), 0);
        assert!(!summary.has_reliable_score());
        assert_eq!(summary.mutation_score(), 0.0);
    }
}
