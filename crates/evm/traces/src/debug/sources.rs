use super::{
    CodeFunctions, FunctionEntry,
    functions::{SourceFunction, UserType, UserTypes, collect, name_of_key},
};
use evm2::interpreter::opcode::OpCode;
use eyre::{Context, Result};
use foundry_common::{
    compact_to_contract, external_compiler::is_external_artifact, is_deploy_helper_path,
    strip_bytecode_placeholders,
};
use foundry_compilers::{
    Artifact, ProjectCompileOutput,
    artifacts::{
        Bytecode, ContractBytecodeSome, FunctionDebugData, Libraries, Source, Sources,
        sourcemap::{SourceElement, SourceMap},
    },
    buildinfo::SourceEdit,
    multi::MultiCompilerLanguage,
};
use foundry_config::read_build_info_input;
use foundry_evm_core::{bytecode::InstIter, ic::PcIcMap};
use foundry_linking::Linker;
use rayon::prelude::*;
use serde::Deserialize;
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
    /// Maps the byte offset of a function, modifier, or public state variable definition to it.
    pub functions: HashMap<usize, Arc<SourceFunction>>,
    /// The user-defined types that the source declares.
    pub user_types: Vec<(String, UserType)>,
    /// The edits that a preprocessor made to the source before compilation, sorted by start
    /// offset. The compiler's source locations refer to the edited source.
    pub source_edits: Vec<SourceEdit>,
}

impl SourceData {
    pub fn new(
        output: &ProjectCompileOutput,
        source: Arc<String>,
        language: MultiCompilerLanguage,
        path: PathBuf,
        root: &Path,
        source_edits: Vec<SourceEdit>,
    ) -> Self {
        let mut contract_definitions = Vec::new();
        let mut functions = HashMap::new();
        let mut user_types = Vec::new();

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
                    let ast = source.ast.as_ref()?;
                    for item in ast.items.iter() {
                        if let solar::ast::ItemKind::Contract(contract) = &item.kind {
                            let Some(contract_range) = span_to_range(source_map, item.span) else {
                                continue;
                            };
                            contract_definitions.push((contract.name.to_string(), contract_range));
                        }
                    }
                    collect(ast, source_map, &mut functions, &mut user_types);
                    Some(())
                });
                if r.is_none() {
                    warn!("failed to parse contract definitions for {}", path.display());
                }
            }
        }

        Self { source, language, path, contract_definitions, functions, user_types, source_edits }
    }

    /// Finds name of contract that contains given loc.
    pub fn find_contract_name(&self, start: usize, end: usize) -> Option<&str> {
        self.contract_definitions
            .iter()
            .find(|(_, r)| start >= r.start && end <= r.end)
            .map(|(name, _)| name.as_str())
    }
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
    /// Whether the contract was compiled through the via-IR pipeline, if the metadata says.
    pub via_ir: Option<bool>,
    /// The functions of the creation code.
    pub functions: CodeFunctions,
    /// The functions of the runtime code.
    pub functions_runtime: CodeFunctions,
}

impl ArtifactData {
    fn new(
        bytecode: ContractBytecodeSome,
        build_id: String,
        file_id: u32,
        via_ir: Option<bool>,
        [function_debug_data, function_debug_data_runtime]: [Option<&BTreeMap<String, FunctionDebugData>>;
            2],
    ) -> Result<Self> {
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
            let code = strip_bytecode_placeholders(&b.object).filter(|bytes| !bytes.is_empty());
            let pc_ic_map = code.as_ref().map(|bytes| PcIcMap::new(bytes.as_ref()));
            let jumpdests =
                code.as_ref().map(|bytes| jumpdests(bytes.as_ref())).unwrap_or_default();

            source_map.map(|source_map| (source_map, pc_ic_map, jumpdests))
        };
        let (source_map, pc_ic_map, jumpdests) = parse(&bytecode.bytecode, "creation")?;
        let (source_map_runtime, pc_ic_map_runtime, jumpdests_runtime) = bytecode
            .deployed_bytecode
            .bytecode
            .map(|b| parse(&b, "runtime"))
            .unwrap_or_else(|| Ok((None, None, HashSet::new())))?;

        Ok(Self {
            source_map,
            source_map_runtime,
            pc_ic_map,
            pc_ic_map_runtime,
            build_id,
            file_id,
            via_ir,
            functions: CodeFunctions { entries: entries(function_debug_data), jumpdests },
            functions_runtime: CodeFunctions {
                entries: entries(function_debug_data_runtime),
                jumpdests: jumpdests_runtime,
            },
        })
    }

    /// The functions of the creation code if `init_code`, otherwise of the runtime code.
    pub const fn functions(&self, init_code: bool) -> &CodeFunctions {
        if init_code { &self.functions } else { &self.functions_runtime }
    }
}

/// The offsets of the `JUMPDEST`s in `code`.
fn jumpdests(code: &[u8]) -> HashSet<u32> {
    InstIter::new(code)
        .with_pc()
        .filter(|(_, inst)| inst.opcode == OpCode::JUMPDEST)
        .map(|(pc, _)| pc as u32)
        .collect()
}

/// The functions that `functionDebugData` places in the code, by entry offset.
///
/// An entry the compiler inlined has no offset.
fn entries(data: Option<&BTreeMap<String, FunctionDebugData>>) -> HashMap<u32, Vec<FunctionEntry>> {
    let mut entries = HashMap::<u32, Vec<FunctionEntry>>::new();
    for (key, function) in data.into_iter().flatten() {
        let (Some(entry_point), Some(parameter_slots), Some(return_slots)) =
            (function.entry_point, function.parameter_slots, function.return_slots)
        else {
            continue;
        };
        entries.entry(entry_point).or_default().push(FunctionEntry {
            name: function.id.and_then(|id| name_of_key(key, id)),
            parameter_slots: parameter_slots as usize,
            return_slots: return_slots as usize,
        });
    }
    entries
}

/// Container with artifacts data useful for identifying individual execution steps.
#[derive(Clone, Debug, Default)]
pub struct ContractSources {
    /// Map over build_id -> file_id -> (source code, language)
    pub sources_by_id: HashMap<String, HashMap<u32, Arc<SourceData>>>,
    /// Map over contract name -> Vec<(bytecode, build_id, file_id)>
    pub artifacts_by_name: HashMap<String, Vec<ArtifactData>>,
    /// The user-defined types of all sources by name, which function parameters refer to.
    pub user_types: UserTypes,
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

        let artifact_ids = output
            .artifact_ids()
            .filter(|(id, _)| !is_external_artifact(&id.build_id))
            .collect::<Vec<_>>();
        let artifacts: Vec<_> = artifact_ids
            .par_iter()
            .map(|(id, artifact)| {
                let mut new_artifact = None;
                if let Some(file_id) = artifact.id {
                    // Solc records `viaIR` in the metadata settings only when it is enabled.
                    let via_ir = artifact
                        .metadata
                        .as_ref()
                        .map(|metadata| metadata.settings.via_ir.unwrap_or_default());
                    let function_debug_data = [
                        artifact.function_debug_data.as_ref(),
                        artifact.deployed_function_debug_data.as_ref(),
                    ];
                    let artifact = if let Some((linker, libraries)) = link_data.as_ref() {
                        linker.link(id, libraries)?
                    } else {
                        artifact.get_contract_bytecode()
                    };
                    let bytecode = compact_to_contract(artifact.into_contract_bytecode())?;

                    new_artifact = Some((
                        id.name.clone(),
                        ArtifactData::new(
                            bytecode,
                            id.build_id.clone(),
                            file_id,
                            via_ir,
                            function_debug_data,
                        )?,
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
                    // Preprocessor deploy helpers are compiled from memory and never exist on disk.
                    if !is_deploy_helper_path(path) {
                        removed_files.insert(path);
                    }
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
                            source.content,
                            build.language,
                            stripped,
                            root,
                            build.source_edits.get(source_id).cloned().unwrap_or_default(),
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

        for source_data in files.values() {
            for (name, ty) in &source_data.user_types {
                self.add_user_type(name.clone(), Some(ty.clone()));
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

    /// Replaces each source that the compiler saw with different text than the file on disk, such
    /// as a test rewritten by dynamic test linking, with that text. Source maps point into it.
    ///
    /// The text comes from the full build infos (`build_info = true`) in `build_info_dir`. A build
    /// without one keeps the files on disk.
    pub fn insert_compiled_sources(&mut self, build_info_dir: &Path) {
        for (build_id, sources) in &mut self.sources_by_id {
            let path = build_info_dir.join(build_id).with_extension("json");
            if !path.exists() {
                continue;
            }
            let mut input = match read_build_info_input::<BuildInfoInput>(&path) {
                Ok(Some(input)) => input,
                Ok(None) => continue,
                Err(error) => {
                    let _ = sh_warn!("skipping build info {}: {error}", path.display());
                    continue;
                }
            };
            for source in sources.values_mut() {
                if let Some(compiled) = input.sources.remove(&source.path)
                    && compiled.content != source.source
                {
                    *source =
                        Arc::new(SourceData { source: compiled.content, ..(**source).clone() });
                }
            }
        }
    }

    /// Merges given contract sources.
    pub fn merge(&mut self, sources: Self) {
        self.sources_by_id.extend(sources.sources_by_id);
        for (name, ty) in sources.user_types {
            self.add_user_type(name, ty);
        }
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

    /// Returns the artifact of a contract that has run. Artifacts can share a name, so among them
    /// it is the first for which `ran` holds, with entry points for the code if one has them.
    pub fn find_artifact(
        &self,
        name: &str,
        init_code: bool,
        ran: impl Fn(&ArtifactData) -> bool,
    ) -> Option<&ArtifactData> {
        let artifacts = self.artifacts_by_name.get(name)?;
        if let [artifact] = artifacts.as_slice() {
            return Some(artifact);
        }
        let mut candidates = artifacts.iter().filter(|artifact| ran(artifact)).peekable();
        let first = candidates.peek().copied();
        candidates.find(|artifact| !artifact.functions(init_code).entries.is_empty()).or(first)
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
        self.get_sources(contract_name)?
            .find_map(|(artifact, source)| self.source_mapping(artifact, source, pc, init_code))
    }

    /// Returns the source map element at `pc` of the code of `artifact`, with the source that it
    /// points into.
    pub fn source_mapping_in(
        &self,
        artifact: &ArtifactData,
        pc: u32,
        init_code: bool,
    ) -> Option<(SourceElement, &SourceData)> {
        let source = self.sources_by_id.get(&artifact.build_id)?.get(&artifact.file_id)?;
        self.source_mapping(artifact, source, pc, init_code)
    }

    fn source_mapping<'a>(
        &'a self,
        artifact: &ArtifactData,
        source: &'a SourceData,
        pc: u32,
        init_code: bool,
    ) -> Option<(SourceElement, &'a SourceData)> {
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
            .and_then(|index| (index == artifact.file_id).then(|| (source_element.clone(), source)))
            .or_else(|| {
                // otherwise find the source code for the element's index
                self.sources_by_id
                    .get(&artifact.build_id)?
                    .get(&source_element.index()?)
                    .map(|source| (source_element.clone(), source.as_ref()))
            })
    }

    /// Records a user-defined type. A name that two different types have resolves to none.
    fn add_user_type(&mut self, name: String, ty: Option<UserType>) {
        match self.user_types.entry(name) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(ty);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry.get() != &ty {
                    entry.insert(None);
                }
            }
        }
    }
}

/// The compiler input of a build info, which only full build infos have.
#[derive(Deserialize)]
struct BuildInfoInput {
    sources: Sources,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_sources_replace_rewritten_files() {
        let source = |path: &str, text: &str| {
            Arc::new(SourceData {
                source: Arc::new(text.to_string()),
                language: Default::default(),
                path: PathBuf::from(path),
                contract_definitions: Vec::new(),
                functions: HashMap::new(),
                user_types: Vec::new(),
                source_edits: Vec::new(),
            })
        };
        let mut sources = ContractSources::default();
        sources.sources_by_id.insert(
            "build".to_string(),
            HashMap::from([
                (0, source("src/A.sol", "contract A {}")),
                (1, source("test/A.t.sol", "contract T { A a = new A(); }")),
            ]),
        );
        // Only `build` has a full build info, and it records a rewritten test file.
        sources.sources_by_id.insert(
            "other".to_string(),
            HashMap::from([(0, source("src/A.sol", "contract A {}"))]),
        );
        let dir = tempfile::tempdir().unwrap();
        let build_info = serde_json::json!({
            "id": "build",
            "input": { "sources": {
                "src/A.sol": { "content": "contract A {}" },
                "test/A.t.sol": { "content": "contract T { A a = A(deploy()); }" },
            }},
        });
        // The read stops before the output, so a truncated output does not matter.
        let build_info = build_info.to_string();
        let build_info =
            format!(r#"{},"output":{{"contracts":"#, &build_info[..build_info.len() - 1]);
        std::fs::write(dir.path().join("build.json"), build_info).unwrap();
        // A malformed build info is skipped.
        std::fs::write(dir.path().join("other.json"), "{").unwrap();

        sources.insert_compiled_sources(dir.path());

        let text =
            |build: &str, id: u32| sources.sources_by_id[build][&id].source.as_str().to_owned();
        assert_eq!(text("build", 0), "contract A {}");
        assert_eq!(text("build", 1), "contract T { A a = A(deploy()); }");
        assert_eq!(sources.sources_by_id["build"][&1].path, PathBuf::from("test/A.t.sol"));
        assert_eq!(text("other", 0), "contract A {}");
    }
}
