use eyre::{Context, Result};
use foundry_common::compact_to_contract;
use foundry_compilers::{
    artifacts::{
        sourcemap::{SourceElement, SourceMap},
        Bytecode, Contract, ContractBytecodeSome, Libraries, Source,
    },
    multi::MultiCompilerLanguage,
    Artifact, Compiler, ProjectCompileOutput,
};
use foundry_evm_core::utils::PcIcMap;
use foundry_linking::Linker;
use rayon::prelude::*;
use solar_parse::{interface::Session, Parser};
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
    contract_definitions: Vec<(String, Range<usize>)>,
}

impl SourceData {
    pub fn new(source: Arc<String>, language: MultiCompilerLanguage, path: PathBuf) -> Self {
        let mut contract_definitions = Vec::new();

        match language {
            MultiCompilerLanguage::Vyper(_) => {
                // Vyper contracts have the same name as the file name.
                if let Some(name) = path.file_stem().map(|s| s.to_string_lossy().to_string()) {
                    contract_definitions.push((name, 0..source.len()));
                }
            }
            MultiCompilerLanguage::Solc(_) => {
                let sess = Session::builder().with_silent_emitter(None).build();
                let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
                    let arena = solar_parse::ast::Arena::new();
                    let filename = path.clone().into();
                    let mut parser =
                        Parser::from_source_code(&sess, &arena, filename, source.to_string())?;
                    let ast = parser.parse_file().map_err(|e| e.emit())?;
                    for item in ast.items {
                        if let solar_parse::ast::ItemKind::Contract(contract) = &item.kind {
                            let range = item.span.lo().to_usize()..item.span.hi().to_usize();
                            contract_definitions.push((contract.name.to_string(), range));
                        }
                    }
                    Ok(())
                });
            }
        }

        Self { source, language, path, contract_definitions }
    }

    /// Finds name of contract that contains given loc.
    pub fn find_contract_name(&self, start: usize, end: usize) -> Option<&str> {
        self.contract_definitions
            .iter()
            .find(|(_, r)| start >= r.start && end <= r.end)
            .map(|(name, _)| name.as_str())
    }
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

            // Only parse bytecode if it's not empty.
            let pc_ic_map = if let Some(bytes) = b.bytes() {
                (!bytes.is_empty()).then(|| PcIcMap::new(bytes))
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

    pub fn insert<C: Compiler<CompilerContract = Contract>>(
        &mut self,
        output: &ProjectCompileOutput<C>,
        root: &Path,
        libraries: Option<&Libraries>,
    ) -> Result<()>
    where
        C::Language: Into<MultiCompilerLanguage>,
    {
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
                            source.content.clone(),
                            build.language.into(),
                            stripped,
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
            let res = source_element
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
                });

            res
        })
    }
}
