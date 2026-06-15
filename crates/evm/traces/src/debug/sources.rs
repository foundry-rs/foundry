use eyre::{Context, Result};
use foundry_common::{compact_to_contract, strip_bytecode_placeholders};
use foundry_compilers::{
    Artifact, ProjectCompileOutput,
    artifacts::{
        Bytecode, ContractBytecodeSome, Libraries, Source,
        sourcemap::{SourceElement, SourceMap},
    },
    multi::MultiCompilerLanguage,
};
use foundry_evm_core::ic::PcIcMap;
use foundry_linking::Linker;
use rayon::prelude::*;
use solar::{ast, interface::SpannedOption};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub struct SourceData {
    pub source: Arc<String>,
    pub language: MultiCompilerLanguage,
    pub path: PathBuf,
    /// Maps contract name to (start, end) of the contract definition in the source code.
    /// This is useful for determining which contract contains given function definition.
    pub contract_definitions: Vec<(String, Range<usize>)>,
    /// Solidity function scopes and variable declarations available to the debugger.
    pub debug_scopes: Vec<DebugSourceScope>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugSourceScope {
    pub contract_name: String,
    pub function_name: String,
    pub range: Range<usize>,
    pub body_range: Range<usize>,
    pub parameters_src: String,
    pub returns_src: Option<String>,
    pub parameters: Vec<DebugVariable>,
    pub returns: Vec<DebugVariable>,
    pub locals: Vec<DebugVariable>,
}

impl DebugSourceScope {
    pub fn visible_locals(&self, offset: usize) -> impl Iterator<Item = &DebugVariable> {
        self.locals.iter().filter(move |local| {
            local.declaration.end <= offset
                && offset >= local.scope.start
                && offset <= local.scope.end
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugVariable {
    pub name: Option<String>,
    pub declaration: Range<usize>,
    pub scope: Range<usize>,
}

impl SourceData {
    pub fn new(
        output: &ProjectCompileOutput,
        source: Arc<String>,
        language: MultiCompilerLanguage,
        path: PathBuf,
        root: &Path,
    ) -> Self {
        let mut contract_definitions = Vec::new();
        let mut debug_scopes = Vec::new();

        match language {
            MultiCompilerLanguage::Vyper(_) => {
                // Vyper contracts have the same name as the file name.
                if let Some(name) = path.file_stem().map(|s| s.to_string_lossy().to_string()) {
                    contract_definitions.push((name, 0..source.len()));
                }
            }
            MultiCompilerLanguage::Solc(_) => {
                let r = output.parser().solc().compiler().enter(|compiler| -> Option<()> {
                    let (_, source) = compiler.gcx().get_ast_source(root.join(&path))?;
                    let source_map = compiler.sess().source_map();
                    for item in source.ast.as_ref()?.items.iter() {
                        if let solar::ast::ItemKind::Contract(contract) = &item.kind {
                            let Some(contract_range) = source_map.span_to_range(item.span).ok()
                            else {
                                continue;
                            };
                            contract_definitions
                                .push((contract.name.to_string(), contract_range.clone()));
                            collect_contract_debug_scopes(
                                source_map,
                                contract,
                                contract_range,
                                &mut debug_scopes,
                            );
                        }
                    }
                    Some(())
                });
                if r.is_none() {
                    warn!("failed to parse contract definitions for {}", path.display());
                }
            }
        }

        Self { source, language, path, contract_definitions, debug_scopes }
    }

    /// Finds name of contract that contains given loc.
    pub fn find_contract_name(&self, start: usize, end: usize) -> Option<&str> {
        self.contract_definitions
            .iter()
            .find(|(_, r)| start >= r.start && end <= r.end)
            .map(|(name, _)| name.as_str())
    }

    /// Finds the innermost Solidity function scope containing the given byte range.
    pub fn find_debug_scope(&self, start: usize, end: usize) -> Option<&DebugSourceScope> {
        self.debug_scopes
            .iter()
            .filter(|scope| start >= scope.range.start && end <= scope.range.end)
            .min_by_key(|scope| scope.range.end.saturating_sub(scope.range.start))
    }
}

fn collect_contract_debug_scopes(
    source_map: &solar::interface::source_map::SourceMap,
    contract: &ast::ItemContract<'_>,
    contract_range: Range<usize>,
    out: &mut Vec<DebugSourceScope>,
) {
    let mut scopes = Vec::new();
    for item in contract.body.iter() {
        let ast::ItemKind::Function(func) = &item.kind else { continue };
        if !func.is_implemented() {
            continue;
        }

        let Some(function_range) = span_to_range(source_map, item.span) else { continue };
        let body_range =
            span_to_range(source_map, func.body_span).unwrap_or_else(|| function_range.clone());
        let function_name = function_name(func);
        let parameters_src =
            source_map.span_to_snippet(func.header.parameters.span).unwrap_or_default();
        let returns_src = func
            .header
            .returns
            .as_ref()
            .and_then(|returns| source_map.span_to_snippet(returns.span).ok());

        let mut locals = Vec::new();
        if let Some(body) = &func.body {
            collect_block_locals(source_map, body, body_range.clone(), &mut locals);
        }

        scopes.push(DebugSourceScope {
            contract_name: contract.name.to_string(),
            function_name,
            range: function_range.clone(),
            body_range: body_range.clone(),
            parameters_src,
            returns_src,
            parameters: variables_from_list(
                source_map,
                &func.header.parameters,
                function_range.clone(),
            ),
            returns: func
                .header
                .returns
                .as_ref()
                .map(|returns| variables_from_list(source_map, returns, function_range.clone()))
                .unwrap_or_default(),
            locals,
        });
    }

    // Keep inherited/inline source-map lookups deterministic when multiple ranges contain a PC.
    scopes.sort_by_key(|scope| {
        (
            scope.range.start,
            scope.range.end.saturating_sub(scope.range.start),
            scope.contract_name.clone(),
            scope.function_name.clone(),
        )
    });

    // Do not let a malformed nested item extend past the contract it was collected from.
    scopes.retain(|scope| {
        scope.range.start >= contract_range.start && scope.range.end <= contract_range.end
    });
    out.extend(scopes);
}

fn function_name(func: &ast::ItemFunction<'_>) -> String {
    match func.kind {
        ast::FunctionKind::Constructor => "constructor".to_string(),
        ast::FunctionKind::Fallback => "fallback".to_string(),
        ast::FunctionKind::Receive => "receive".to_string(),
        ast::FunctionKind::Modifier => {
            func.header.name.as_ref().map(|n| n.as_str()).unwrap_or("modifier").to_string()
        }
        ast::FunctionKind::Function => {
            func.header.name.as_ref().map(|n| n.as_str()).unwrap_or("function").to_string()
        }
    }
}

fn variables_from_list(
    source_map: &solar::interface::source_map::SourceMap,
    vars: &[ast::VariableDefinition<'_>],
    scope: Range<usize>,
) -> Vec<DebugVariable> {
    vars.iter().filter_map(|var| variable_from_definition(source_map, var, scope.clone())).collect()
}

fn collect_block_locals(
    source_map: &solar::interface::source_map::SourceMap,
    block: &ast::Block<'_>,
    fallback_scope: Range<usize>,
    out: &mut Vec<DebugVariable>,
) {
    let scope = span_to_range(source_map, block.span).unwrap_or(fallback_scope);
    for stmt in block.stmts.iter() {
        collect_stmt_locals(source_map, stmt, scope.clone(), out);
    }
}

fn collect_stmt_locals(
    source_map: &solar::interface::source_map::SourceMap,
    stmt: &ast::Stmt<'_>,
    scope: Range<usize>,
    out: &mut Vec<DebugVariable>,
) {
    match &stmt.kind {
        ast::StmtKind::DeclSingle(var) => {
            if let Some(var) = variable_from_definition(source_map, var, scope) {
                out.push(var);
            }
        }
        ast::StmtKind::DeclMulti(vars, _) => {
            for var in vars.iter() {
                let SpannedOption::Some(var) = var else { continue };
                if let Some(var) = variable_from_definition(source_map, var, scope.clone()) {
                    out.push(var);
                }
            }
        }
        ast::StmtKind::Block(block) | ast::StmtKind::UncheckedBlock(block) => {
            collect_block_locals(source_map, block, scope, out);
        }
        ast::StmtKind::If(_, then_stmt, else_stmt) => {
            let then_scope =
                span_to_range(source_map, then_stmt.span).unwrap_or_else(|| scope.clone());
            collect_stmt_locals(source_map, then_stmt, then_scope, out);
            if let Some(else_stmt) = else_stmt {
                let else_scope =
                    span_to_range(source_map, else_stmt.span).unwrap_or_else(|| scope.clone());
                collect_stmt_locals(source_map, else_stmt, else_scope, out);
            }
        }
        ast::StmtKind::For { init, body, .. } => {
            let for_scope = span_to_range(source_map, stmt.span).unwrap_or_else(|| scope.clone());
            if let Some(init) = init {
                collect_stmt_locals(source_map, init, for_scope.clone(), out);
            }
            collect_stmt_locals(source_map, body, for_scope, out);
        }
        ast::StmtKind::While(_, body) | ast::StmtKind::DoWhile(body, _) => {
            let stmt_scope = span_to_range(source_map, stmt.span).unwrap_or(scope);
            collect_stmt_locals(source_map, body, stmt_scope, out);
        }
        ast::StmtKind::Try(try_stmt) => {
            for clause in try_stmt.clauses.iter() {
                let clause_scope =
                    span_to_range(source_map, clause.span).unwrap_or_else(|| scope.clone());
                for arg in clause.args.iter() {
                    if let Some(arg) =
                        variable_from_definition(source_map, arg, clause_scope.clone())
                    {
                        out.push(arg);
                    }
                }
                collect_block_locals(source_map, &clause.block, clause_scope, out);
            }
        }
        ast::StmtKind::Assembly(_)
        | ast::StmtKind::Break
        | ast::StmtKind::Continue
        | ast::StmtKind::Emit(..)
        | ast::StmtKind::Expr(_)
        | ast::StmtKind::Return(_)
        | ast::StmtKind::Revert(..)
        | ast::StmtKind::Placeholder => {}
    }
}

fn variable_from_definition(
    source_map: &solar::interface::source_map::SourceMap,
    var: &ast::VariableDefinition<'_>,
    scope: Range<usize>,
) -> Option<DebugVariable> {
    Some(DebugVariable {
        name: var.name.map(|name| name.to_string()),
        declaration: span_to_range(source_map, var.span)?,
        scope,
    })
}

fn span_to_range(
    source_map: &solar::interface::source_map::SourceMap,
    span: solar::interface::Span,
) -> Option<Range<usize>> {
    source_map.span_to_range(span).ok()
}

#[derive(Clone, Debug)]
pub struct ArtifactData {
    pub source_map: Option<SourceMap>,
    pub source_map_runtime: Option<SourceMap>,
    pub pc_ic_map: Option<PcIcMap>,
    pub pc_ic_map_runtime: Option<PcIcMap>,
    pub build_id: String,
    pub file_id: u32,
}

impl ArtifactData {
    fn new(bytecode: ContractBytecodeSome, build_id: String, file_id: u32) -> Result<Self> {
        let parse = |b: &Bytecode, name: &str| {
            // Only parse source map if it's not empty.
            let source_map = if b.source_map.as_ref().is_none_or(|s| s.is_empty()) {
                Ok(None)
            } else {
                b.source_map().transpose().wrap_err_with(|| {
                    format!("failed to parse {name} source map of file {file_id} in {build_id}")
                })
            };

            // Only parse bytecode if it's not empty, stripping placeholders if necessary.
            let pc_ic_map = if let Some(bytes) = strip_bytecode_placeholders(&b.object) {
                (!bytes.is_empty()).then(|| PcIcMap::new(bytes.as_ref()))
            } else {
                None
            };

            source_map.map(|source_map| (source_map, pc_ic_map))
        };
        let (source_map, pc_ic_map) = parse(&bytecode.bytecode, "creation")?;
        let (source_map_runtime, pc_ic_map_runtime) = bytecode
            .deployed_bytecode
            .bytecode
            .map(|b| parse(&b, "runtime"))
            .unwrap_or_else(|| Ok((None, None)))?;

        Ok(Self { source_map, source_map_runtime, pc_ic_map, pc_ic_map_runtime, build_id, file_id })
    }
}

/// Container with artifacts data useful for identifying individual execution steps.
#[derive(Clone, Debug, Default)]
pub struct ContractSources {
    /// Map over build_id -> file_id -> (source code, language)
    pub sources_by_id: HashMap<String, HashMap<u32, Arc<SourceData>>>,
    /// Map over contract name -> Vec<(bytecode, build_id, file_id)>
    pub artifacts_by_name: HashMap<String, Vec<ArtifactData>>,
}

impl ContractSources {
    /// Collects the contract sources and artifacts from the project compile output.
    pub fn from_project_output(
        output: &ProjectCompileOutput,
        root: &Path,
        libraries: Option<&Libraries>,
    ) -> Result<Self> {
        let mut sources = Self::default();
        sources.insert(output, root, libraries)?;
        Ok(sources)
    }

    pub fn insert(
        &mut self,
        output: &ProjectCompileOutput,
        root: &Path,
        libraries: Option<&Libraries>,
    ) -> Result<()> {
        let link_data = libraries.map(|libraries| {
            let linker = Linker::new(root, output.artifact_ids().collect());
            (linker, libraries)
        });

        let artifacts: Vec<_> = output
            .artifact_ids()
            .collect::<Vec<_>>()
            .par_iter()
            .map(|(id, artifact)| {
                let mut new_artifact = None;
                if let Some(file_id) = artifact.id {
                    let artifact = if let Some((linker, libraries)) = link_data.as_ref() {
                        linker.link(id, libraries)?
                    } else {
                        artifact.get_contract_bytecode()
                    };
                    let bytecode = compact_to_contract(artifact.into_contract_bytecode())?;

                    new_artifact = Some((
                        id.name.clone(),
                        ArtifactData::new(bytecode, id.build_id.clone(), file_id)?,
                    ));
                } else {
                    warn!(id = id.identifier(), "source not found");
                };

                Ok(new_artifact)
            })
            .collect::<Result<Vec<_>>>()?;

        for (name, artifact) in artifacts.into_iter().flatten() {
            self.artifacts_by_name.entry(name).or_default().push(artifact);
        }

        // Not all source files produce artifacts, so we are populating sources by using build
        // infos.
        let mut files: BTreeMap<PathBuf, Arc<SourceData>> = BTreeMap::new();
        let mut removed_files = HashSet::new();
        for (build_id, build) in output.builds() {
            for (source_id, path) in &build.source_id_to_path {
                if !path.exists() {
                    removed_files.insert(path);
                    continue;
                }

                let source_data = match files.entry(path.clone()) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        let source = Source::read(path).wrap_err_with(|| {
                            format!("failed to read artifact source file for `{}`", path.display())
                        })?;
                        let stripped = path.strip_prefix(root).unwrap_or(path).to_path_buf();
                        let source_data = Arc::new(SourceData::new(
                            output,
                            source.content.clone(),
                            build.language,
                            stripped,
                            root,
                        ));
                        entry.insert(source_data.clone());
                        source_data
                    }
                    std::collections::btree_map::Entry::Occupied(entry) => entry.get().clone(),
                };
                self.sources_by_id
                    .entry(build_id.clone())
                    .or_default()
                    .insert(*source_id, source_data);
            }
        }

        if !removed_files.is_empty() {
            let mut warning = "Detected artifacts built from source files that no longer exist. \
                Run `forge clean` to make sure builds are in sync with project files."
                .to_string();
            for file in removed_files {
                write!(warning, "\n - {}", file.display())?;
            }
            let _ = sh_warn!("{}", warning);
        }

        Ok(())
    }

    /// Merges given contract sources.
    pub fn merge(&mut self, sources: Self) {
        self.sources_by_id.extend(sources.sources_by_id);
        for (name, artifacts) in sources.artifacts_by_name {
            self.artifacts_by_name.entry(name).or_default().extend(artifacts);
        }
    }

    /// Returns all sources for a contract by name.
    pub fn get_sources(
        &self,
        name: &str,
    ) -> Option<impl Iterator<Item = (&ArtifactData, &SourceData)>> {
        self.artifacts_by_name.get(name).map(|artifacts| {
            artifacts.iter().filter_map(|artifact| {
                let source =
                    self.sources_by_id.get(artifact.build_id.as_str())?.get(&artifact.file_id)?;
                Some((artifact, source.as_ref()))
            })
        })
    }

    /// Returns all (name, bytecode, source) sets.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &ArtifactData, &SourceData)> {
        self.artifacts_by_name.iter().flat_map(|(name, artifacts)| {
            artifacts.iter().filter_map(|artifact| {
                let source =
                    self.sources_by_id.get(artifact.build_id.as_str())?.get(&artifact.file_id)?;
                Some((name.as_str(), artifact, source.as_ref()))
            })
        })
    }

    pub fn find_source_mapping(
        &self,
        contract_name: &str,
        pc: u32,
        init_code: bool,
    ) -> Option<(SourceElement, &SourceData)> {
        self.get_sources(contract_name)?.find_map(|(artifact, source)| {
            let source_map = if init_code {
                artifact.source_map.as_ref()
            } else {
                artifact.source_map_runtime.as_ref()
            }?;

            // Solc indexes source maps by instruction counter, but Vyper indexes by program
            // counter.
            let source_element = if matches!(source.language, MultiCompilerLanguage::Solc(_)) {
                let pc_ic_map = if init_code {
                    artifact.pc_ic_map.as_ref()
                } else {
                    artifact.pc_ic_map_runtime.as_ref()
                }?;
                let ic = pc_ic_map.get(pc)?;

                source_map.get(ic as usize)
            } else {
                source_map.get(pc as usize)
            }?;
            // if the source element has an index, find the sourcemap for that index
            source_element
                .index()
                // if index matches current file_id, return current source code
                .and_then(|index| {
                    (index == artifact.file_id).then(|| (source_element.clone(), source))
                })
                .or_else(|| {
                    // otherwise find the source code for the element's index
                    self.sources_by_id
                        .get(&artifact.build_id)?
                        .get(&source_element.index()?)
                        .map(|source| (source_element.clone(), source.as_ref()))
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(name: &str, declaration: Range<usize>, scope: Range<usize>) -> DebugVariable {
        DebugVariable { name: Some(name.to_string()), declaration, scope }
    }

    fn scope(locals: Vec<DebugVariable>) -> DebugSourceScope {
        DebugSourceScope {
            contract_name: "DebugMe".to_string(),
            function_name: "foo".to_string(),
            range: 0..100,
            body_range: 10..90,
            parameters_src: String::new(),
            returns_src: None,
            parameters: Vec::new(),
            returns: Vec::new(),
            locals,
        }
    }

    #[test]
    fn visible_locals_require_declaration_and_scope() {
        let scope = scope(vec![
            variable("before", 10..15, 10..90),
            variable("after", 70..75, 10..90),
            variable("nested", 20..25, 20..40),
        ]);

        let names = |offset| {
            scope
                .visible_locals(offset)
                .map(|variable| variable.name.as_deref().unwrap())
                .collect::<Vec<_>>()
        };

        assert_eq!(names(14), Vec::<&str>::new());
        assert_eq!(names(30), ["before", "nested"]);
        assert_eq!(names(50), ["before"]);
        assert_eq!(names(80), ["before", "after"]);
    }
}
