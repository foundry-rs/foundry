use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};

use eyre::{Context, Result};
use foundry_common::compact_to_contract;
use foundry_compilers::{
    artifacts::{sourcemap::SourceMap, Bytecode, ContractBytecodeSome, Libraries, Source},
    multi::MultiCompilerLanguage,
    Artifact, Compiler, ProjectCompileOutput,
};
use foundry_evm_core::utils::PcIcMap;
use foundry_linking::Linker;
use rustc_hash::FxHashMap;

#[derive(Clone, Debug)]
pub struct SourceData {
    pub source: Arc<String>,
    pub language: MultiCompilerLanguage,
    pub name: String,
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
        let parse = |b: &Bytecode| {
            // Only parse source map if it's not empty.
            let source_map = if b.source_map.as_ref().map_or(true, |s| s.is_empty()) {
                Ok(None)
            } else {
                b.source_map().transpose()
            };

            // Only parse bytecode if it's not empty.
            let pc_ic_map = if let Some(bytes) = b.bytes() {
                (!bytes.is_empty()).then(|| PcIcMap::new(bytes))
            } else {
                None
            };

            source_map.map(|source_map| (source_map, pc_ic_map))
        };
        let (source_map, pc_ic_map) = parse(&bytecode.bytecode)?;
        let (source_map_runtime, pc_ic_map_runtime) = bytecode
            .deployed_bytecode
            .bytecode
            .map(|b| parse(&b))
            .unwrap_or_else(|| Ok((None, None)))?;

        Ok(Self { source_map, source_map_runtime, pc_ic_map, pc_ic_map_runtime, build_id, file_id })
    }
}

/// Container with artifacts data useful for identifying individual execution steps.
#[derive(Clone, Debug, Default)]
pub struct ContractSources {
    /// Map over build_id -> file_id -> (source code, language)
    pub sources_by_id: HashMap<String, FxHashMap<u32, SourceData>>,
    /// Map over contract name -> Vec<(bytecode, build_id, file_id)>
    pub artifacts_by_name: HashMap<String, Vec<ArtifactData>>,
}

impl ContractSources {
    /// Collects the contract sources and artifacts from the project compile output.
    pub fn from_project_output(
        output: &ProjectCompileOutput,
        root: impl AsRef<Path>,
        libraries: Option<&Libraries>,
    ) -> Result<Self> {
        let mut sources = Self::default();

        sources.insert(output, root, libraries)?;

        Ok(sources)
    }

    pub fn insert<C: Compiler>(
        &mut self,
        output: &ProjectCompileOutput<C>,
        root: impl AsRef<Path>,
        libraries: Option<&Libraries>,
    ) -> Result<()>
    where
        C::Language: Into<MultiCompilerLanguage>,
    {
        let root = root.as_ref();
        let link_data = libraries.map(|libraries| {
            let linker = Linker::new(root, output.artifact_ids().collect());
            (linker, libraries)
        });

        for (id, artifact) in output.artifact_ids() {
            if let Some(file_id) = artifact.id {
                let artifact = if let Some((linker, libraries)) = link_data.as_ref() {
                    linker.link(&id, libraries)?.into_contract_bytecode()
                } else {
                    artifact.clone().into_contract_bytecode()
                };
                let bytecode = compact_to_contract(artifact.clone().into_contract_bytecode())?;

                self.artifacts_by_name.entry(id.name.clone()).or_default().push(ArtifactData::new(
                    bytecode,
                    id.build_id.clone(),
                    file_id,
                )?);
            } else {
                warn!(id = id.identifier(), "source not found");
            }
        }

        // Not all source files produce artifacts, so we are populating sources by using build
        // infos.
        let mut files: BTreeMap<PathBuf, Arc<String>> = BTreeMap::new();
        for (build_id, build) in output.builds() {
            for (source_id, path) in &build.source_id_to_path {
                let source_code = if let Some(source) = files.get(path) {
                    source.clone()
                } else {
                    let source = Source::read(path).wrap_err_with(|| {
                        format!("failed to read artifact source file for `{}`", path.display())
                    })?;
                    files.insert(path.clone(), source.content.clone());
                    source.content
                };

                self.sources_by_id.entry(build_id.clone()).or_default().insert(
                    *source_id,
                    SourceData {
                        source: source_code,
                        language: build.language.into(),
                        name: path.strip_prefix(root).unwrap_or(path).to_string_lossy().to_string(),
                    },
                );
            }
        }

        Ok(())
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
                Some((artifact, source))
            })
        })
    }

    /// Returns all (name, bytecode, source) sets.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &ArtifactData, &SourceData)> {
        self.artifacts_by_name.iter().flat_map(|(name, artifacts)| {
            artifacts.iter().filter_map(|artifact| {
                let source =
                    self.sources_by_id.get(artifact.build_id.as_str())?.get(&artifact.file_id)?;
                Some((name.as_str(), artifact, source))
            })
        })
    }
}
