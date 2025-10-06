pub mod mutant;
mod mutators;
mod reporter;
mod visitor;

use alloy_primitives::U256;
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
use dunce;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json;
use solar_interface::BytePos;
use solar_parse::ast::visit::Visit;
use std::{collections::HashMap, path::PathBuf};

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

    /// Persists the mapping entry for this contract and writes the cached mutants JSON file
    /// at `cache/mutation/<hash>.mutants`.
    pub fn persist_cached_mutants(&self, hash: &str, mutants: &[Mutant]) -> std::io::Result<()> {
        #[derive(Serialize, Deserialize)]
        #[serde(tag = "kind")]
        enum MutationDtoKind {
            AssignmentLiteral { lit: String },
            AssignmentIdentifier { ident: String },
            BinaryOp { op: String },
            DeleteExpression,
            ElimDelegate,
            FunctionCall,
            Require,
            SwapArgumentsFunction,
            SwapArgumentsOperator,
            UnaryOperator { expr: String, op: String },
        }

        #[derive(Serialize, Deserialize)]
        struct MutantDto {
            path: String,
            lo: u64,
            hi: u64,
            mutation: MutationDtoKind,
        }

        let mutation_cache_dir = self.config.root.join(&self.config.mutation_dir);
        std::fs::create_dir_all(&mutation_cache_dir)?;

        // Update mapping.json with absolute path -> build hash
        let mapping_path = mutation_cache_dir.join("mapping.json");
        let mut mapping: HashMap<String, String> = std::fs::read_to_string(&mapping_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let contract_path = dunce::canonicalize(&self.contract_to_mutate)
            .unwrap_or_else(|_| self.contract_to_mutate.clone())
            .to_string_lossy()
            .into_owned();
        mapping.insert(contract_path, hash.to_string());

        let mapping_json = serde_json::to_string_pretty(&mapping).map_err(std::io::Error::other)?;
        std::fs::write(&mapping_path, mapping_json)?;

        // Write <hash>.mutants with a simple JSON array
        let dtos: Vec<MutantDto> = mutants
            .iter()
            .map(|m| {
                let mutation = match &m.mutation {
                    crate::mutation::mutant::MutationType::Assignment(assign) => match assign {
                        crate::mutation::visitor::AssignVarTypes::Literal(lit) => {
                            // todo: why not all cases?
                            let lit_s = match lit {
                                crate::mutation::mutant::OwnedLiteral::Bool(true) => "true",
                                crate::mutation::mutant::OwnedLiteral::Bool(false) => "false",
                                crate::mutation::mutant::OwnedLiteral::Number(_) => "number",
                                _ => "other",
                            };
                            MutationDtoKind::AssignmentLiteral { lit: lit_s.to_string() }
                        }
                        crate::mutation::visitor::AssignVarTypes::Identifier(ident) => {
                            MutationDtoKind::AssignmentIdentifier { ident: ident.clone() }
                        }
                    },
                    crate::mutation::mutant::MutationType::BinaryOp(kind) => {
                        MutationDtoKind::BinaryOp { op: kind.to_str().to_string() }
                    }
                    crate::mutation::mutant::MutationType::DeleteExpression => {
                        MutationDtoKind::DeleteExpression
                    }
                    crate::mutation::mutant::MutationType::ElimDelegate => {
                        MutationDtoKind::ElimDelegate
                    }
                    crate::mutation::mutant::MutationType::FunctionCall => {
                        MutationDtoKind::FunctionCall
                    }
                    crate::mutation::mutant::MutationType::Require => MutationDtoKind::Require,
                    crate::mutation::mutant::MutationType::SwapArgumentsFunction => {
                        MutationDtoKind::SwapArgumentsFunction
                    }
                    crate::mutation::mutant::MutationType::SwapArgumentsOperator => {
                        MutationDtoKind::SwapArgumentsOperator
                    }
                    crate::mutation::mutant::MutationType::UnaryOperator(u) => {
                        MutationDtoKind::UnaryOperator {
                            expr: u.to_string(),
                            op: format!("{:?}", u.resulting_op_kind),
                        }
                    }
                };

                MutantDto {
                    path: m.path.to_string_lossy().into_owned(),
                    lo: m.span.lo().0 as u64,
                    hi: m.span.hi().0 as u64,
                    mutation,
                }
            })
            .collect();

        let contract_name = self
            .contract_to_mutate
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("contract")
            .to_string();
        let mutants_file = mutation_cache_dir.join(format!("{contract_name}.mutants"));
        let json = serde_json::to_string_pretty(&dtos).map_err(std::io::Error::other)?;
        std::fs::write(mutants_file, json)?;

        Ok(())
    }

    /// Persists results for mutants for given build hash at `cache/mutation/<hash>.results`.
    pub fn persist_cached_results(
        &self,
        // todo: why unused?
        hash: &str,
        results: &[(Mutant, crate::mutation::mutant::MutationResult)],
    ) -> std::io::Result<()> {
        #[derive(Serialize)]
        struct ResultDto {
            path: String,
            lo: u64,
            hi: u64,
            status: String,
        }

        let mutation_cache_dir = self.config.root.join(&self.config.mutation_dir);
        std::fs::create_dir_all(&mutation_cache_dir)?;

        let serialized: Vec<ResultDto> = results
            .iter()
            .map(|(m, r)| ResultDto {
                path: m.path.to_string_lossy().into_owned(),
                lo: m.span.lo().0 as u64,
                hi: m.span.hi().0 as u64,
                status: match r {
                    crate::mutation::mutant::MutationResult::Dead => "dead".to_string(),
                    crate::mutation::mutant::MutationResult::Alive => "alive".to_string(),
                    crate::mutation::mutant::MutationResult::Invalid => "invalid".to_string(),
                },
            })
            .collect();

        let contract_name = self
            .contract_to_mutate
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("contract")
            .to_string();
        let results_file = mutation_cache_dir.join(format!("{contract_name}.results"));
        let json = serde_json::to_string_pretty(&serialized).map_err(std::io::Error::other)?;
        std::fs::write(results_file, json)?;

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

    // get the file which hold a mapping `contract to mutate`->hash build
    // - if target contract doesn't exist in it, return None
    // - if target contract exist, get the hash build:
    // -- if hash build is the same as the one passed as argument, load the mutants from the
    //   hash.mutants file and return Some(mutants)
    // -- if hash build is different, remove it from the mapping file and return None
    pub fn retrieve_cached_mutants(&self, hash: &str) -> Option<Vec<Mutant>> {
        // mutation cache directory under the project root
        let mutation_cache_dir = self.config.root.join(&self.config.mutation_dir);
        let mapping_path = mutation_cache_dir.join("mapping.json");

        // Read mapping file `{contract_absolute_path -> build_hash}`
        let mapping: HashMap<String, String> = std::fs::read_to_string(&mapping_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())?;

        // Canonicalize the contract path to match mapping keys
        let contract_path = dunce::canonicalize(&self.contract_to_mutate)
            .unwrap_or_else(|_| self.contract_to_mutate.clone())
            .to_string_lossy()
            .into_owned();

        if let Some(stored_hash) = mapping.get(&contract_path) {
            if stored_hash == hash {
                // Try to read the cached mutants file for this build hash
                let contract_name = self
                    .contract_to_mutate
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("contract")
                    .to_string();
                let mutants_file = mutation_cache_dir.join(format!("{contract_name}.mutants"));
                if mutants_file.exists()
                    && let Ok(data) = std::fs::read_to_string(&mutants_file)
                {
                    #[derive(Deserialize)]
                    #[serde(tag = "kind")]
                    enum MutationDtoKindRead {
                        AssignmentLiteral { lit: String },
                        AssignmentIdentifier { ident: String },
                        BinaryOp { op: String },
                        DeleteExpression,
                        ElimDelegate,
                        FunctionCall,
                        Require,
                        SwapArgumentsFunction,
                        SwapArgumentsOperator,
                        UnaryOperator { expr: String, op: String },
                    }
                    #[derive(Deserialize)]
                    struct MutantDtoRead {
                        path: String,
                        lo: u64,
                        hi: u64,
                        mutation: MutationDtoKindRead,
                    }

                    if let Ok(raw_mutants) = serde_json::from_str::<Vec<MutantDtoRead>>(&data) {
                        let mut out: Vec<Mutant> = Vec::new();
                        for m in raw_mutants {
                            let span = solar_parse::ast::Span::new(
                                BytePos(m.lo as u32),
                                BytePos(m.hi as u32),
                            );
                            let mutation = match m.mutation {
                                MutationDtoKindRead::AssignmentLiteral { lit } => {
                                    let lit_val = match lit.as_str() {
                                        "true" => crate::mutation::mutant::OwnedLiteral::Bool(true),
                                        "false" => {
                                            crate::mutation::mutant::OwnedLiteral::Bool(false)
                                        }
                                        _ => crate::mutation::mutant::OwnedLiteral::Number(
                                            U256::ZERO,
                                        ),
                                    };
                                    crate::mutation::mutant::MutationType::Assignment(
                                        crate::mutation::visitor::AssignVarTypes::Literal(lit_val),
                                    )
                                }
                                MutationDtoKindRead::AssignmentIdentifier { ident } => {
                                    crate::mutation::mutant::MutationType::Assignment(
                                        crate::mutation::visitor::AssignVarTypes::Identifier(ident),
                                    )
                                }
                                MutationDtoKindRead::BinaryOp { op } => {
                                    let kind = match op.as_str() {
                                        "+" => solar_parse::ast::BinOpKind::Add,
                                        "-" => solar_parse::ast::BinOpKind::Sub,
                                        "*" => solar_parse::ast::BinOpKind::Mul,
                                        "/" => solar_parse::ast::BinOpKind::Div,
                                        "&" => solar_parse::ast::BinOpKind::BitAnd,
                                        "|" => solar_parse::ast::BinOpKind::BitOr,
                                        "^" => solar_parse::ast::BinOpKind::BitXor,
                                        "&&" => solar_parse::ast::BinOpKind::And,
                                        "||" => solar_parse::ast::BinOpKind::Or,
                                        "==" => solar_parse::ast::BinOpKind::Eq,
                                        "!=" => solar_parse::ast::BinOpKind::Ne,
                                        ">" => solar_parse::ast::BinOpKind::Gt,
                                        ">=" => solar_parse::ast::BinOpKind::Ge,
                                        "<" => solar_parse::ast::BinOpKind::Lt,
                                        "<=" => solar_parse::ast::BinOpKind::Le,
                                        other => panic!(
                                            "Unknown binary operator token in cache: {other}"
                                        ),
                                    };
                                    crate::mutation::mutant::MutationType::BinaryOp(kind)
                                }
                                MutationDtoKindRead::DeleteExpression => {
                                    crate::mutation::mutant::MutationType::DeleteExpression
                                }
                                MutationDtoKindRead::ElimDelegate => {
                                    crate::mutation::mutant::MutationType::ElimDelegate
                                }
                                MutationDtoKindRead::FunctionCall => {
                                    crate::mutation::mutant::MutationType::FunctionCall
                                }
                                MutationDtoKindRead::Require => {
                                    crate::mutation::mutant::MutationType::Require
                                }
                                MutationDtoKindRead::SwapArgumentsFunction => {
                                    crate::mutation::mutant::MutationType::SwapArgumentsFunction
                                }
                                MutationDtoKindRead::SwapArgumentsOperator => {
                                    crate::mutation::mutant::MutationType::SwapArgumentsOperator
                                }
                                MutationDtoKindRead::UnaryOperator { expr, op } => {
                                    let resulting = match op.as_str() {
                                        "PreInc" => solar_parse::ast::UnOpKind::PreInc,
                                        "PostInc" => solar_parse::ast::UnOpKind::PostInc,
                                        "PreDec" => solar_parse::ast::UnOpKind::PreDec,
                                        "PostDec" => solar_parse::ast::UnOpKind::PostDec,
                                        "Not" => solar_parse::ast::UnOpKind::Not,
                                        "BitNot" => solar_parse::ast::UnOpKind::BitNot,
                                        "Neg" => solar_parse::ast::UnOpKind::Neg,
                                        other => {
                                            panic!("Unknown unary operator token in cache: {other}")
                                        }
                                    };
                                    crate::mutation::mutant::MutationType::UnaryOperator(
                                        crate::mutation::mutant::UnaryOpMutated::new(
                                            expr, resulting,
                                        ),
                                    )
                                }
                            };
                            out.push(Mutant { path: PathBuf::from(m.path), span, mutation });
                        }
                        return Some(out);
                    }
                }
                // If the mutants file doesn't exist, treat as cache miss
            } else {
                // Stale entry: remove from mapping file
                let mut updated = mapping.clone();
                updated.remove(&contract_path);
                if let Ok(json) = serde_json::to_string_pretty(&updated) {
                    let _ = std::fs::create_dir_all(&mutation_cache_dir);
                    let _ = std::fs::write(&mapping_path, json);
                }
            }
            return None;
        }

        None
    }

    /// Retrieves cached results for given build hash.
    pub fn retrieve_cached_mutant_results(
        &self,
        hash: &str,
    ) -> Option<Vec<(Mutant, MutationResult)>> {
        let mutation_cache_dir = self.config.root.join(&self.config.mutation_dir);
        let mapping_path = mutation_cache_dir.join("mapping.json");

        let mapping: HashMap<String, String> = std::fs::read_to_string(&mapping_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())?;

        let contract_path = dunce::canonicalize(&self.contract_to_mutate)
            .unwrap_or_else(|_| self.contract_to_mutate.clone())
            .to_string_lossy()
            .into_owned();

        if let Some(stored_hash) = mapping.get(&contract_path)
            && stored_hash == hash
        {
            let contract_name = self
                .contract_to_mutate
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("contract")
                .to_string();
            let results_file = mutation_cache_dir.join(format!("{contract_name}.results"));
            if results_file.exists()
                && let Ok(data) = std::fs::read_to_string(&results_file)
            {
                #[derive(Deserialize)]
                struct ResultDto {
                    path: String,
                    lo: u64,
                    hi: u64,
                    status: String,
                }

                if let Ok(entries) = serde_json::from_str::<Vec<ResultDto>>(&data) {
                    let mut out = Vec::with_capacity(entries.len());
                    for e in entries {
                        let span =
                            solar_parse::ast::Span::new(BytePos(e.lo as u32), BytePos(e.hi as u32));
                        let status = match e.status.as_str() {
                            "dead" => crate::mutation::mutant::MutationResult::Dead,
                            "alive" => crate::mutation::mutant::MutationResult::Alive,
                            _ => crate::mutation::mutant::MutationResult::Invalid,
                        };
                        // We need the full mutation to be able to reuse; find it via
                        // mutants cache if available
                        // Fallback: create a placeholder minimal Mutant with empty mutation
                        // (should not happen since we also cache full mutants)
                        // Here we try to match from cached mutants file
                        if let Some(mutants) = self.retrieve_cached_mutants(hash)
                            && let Some(m) = mutants.into_iter().find(|m| {
                                m.path == PathBuf::from(&e.path)
                                    && m.span.lo().0 as u64 == e.lo
                                    && m.span.hi().0 as u64 == e.hi
                            })
                        {
                            out.push((m, status));
                            continue;
                        }
                        out.push((
                            Mutant {
                                path: PathBuf::from(e.path),
                                span,
                                mutation: crate::mutation::mutant::MutationType::DeleteExpression,
                            },
                            status,
                        ));
                    }
                    return Some(out);
                }
            }
        }
        None
    }
}
