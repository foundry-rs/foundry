//! Represents an entire build

use crate::compilers::{
    CompilationError, CompilerContract, CompilerInput, CompilerOutput, Language,
};
use alloy_primitives::hex;
use foundry_compilers_core::{error::Result, utils};
use md5::Digest;
use semver::Version;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

pub const ETHERS_FORMAT_VERSION: &str = "ethers-rs-sol-build-info-1";

// A hardhat compatible build info representation
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo<I, O> {
    pub id: String,
    #[serde(rename = "_format")]
    pub format: String,
    pub solc_version: Version,
    pub solc_long_version: Version,
    pub input: I,
    pub output: O,
}

impl<I: DeserializeOwned, O: DeserializeOwned> BuildInfo<I, O> {
    /// Deserializes the `BuildInfo` object from the given file
    pub fn read(path: &Path) -> Result<Self> {
        utils::read_json_file(path)
    }
}

/// Additional context we cache for each compiler run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildContext<L> {
    /// Mapping from internal compiler source id to path of the source file.
    pub source_id_to_path: BTreeMap<u32, PathBuf>,
    /// Language of the compiler.
    pub language: L,
}

impl<L: Language> BuildContext<L> {
    pub fn new<I, E, C>(input: &I, output: &CompilerOutput<E, C>) -> Result<Self>
    where
        I: CompilerInput<Language = L>,
    {
        let mut source_id_to_path = BTreeMap::new();

        let input_sources = input.sources().map(|(path, _)| path).collect::<HashSet<_>>();
        for (path, source) in output.sources.iter() {
            if input_sources.contains(path.as_path()) {
                source_id_to_path.insert(source.id, path.to_path_buf());
            }
        }

        Ok(Self { source_id_to_path, language: input.language() })
    }

    pub fn join_all(&mut self, root: &Path) {
        self.source_id_to_path.values_mut().for_each(|path| {
            *path = root.join(path.as_path());
        });
    }

    pub fn with_joined_paths(mut self, root: &Path) -> Self {
        self.join_all(root);
        self
    }
}

/// Represents `BuildInfo` object
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawBuildInfo<L> {
    /// The hash that identifies the BuildInfo
    pub id: String,
    #[serde(flatten)]
    pub build_context: BuildContext<L>,
    /// serialized `BuildInfo` json
    #[serde(flatten)]
    pub build_info: BTreeMap<String, serde_json::Value>,
}

// === impl RawBuildInfo ===

impl<L: Language> RawBuildInfo<L> {
    /// Serializes a `BuildInfo` object
    pub fn new<I: CompilerInput<Language = L>, E: CompilationError, C: CompilerContract>(
        input: &I,
        output: &CompilerOutput<E, C>,
        full_build_info: bool,
    ) -> Result<Self> {
        let version = input.version().clone();
        let build_context = BuildContext::new(input, output)?;

        let mut hasher = md5::Md5::new();

        hasher.update(ETHERS_FORMAT_VERSION);

        let solc_short = format!("{}.{}.{}", version.major, version.minor, version.patch);
        hasher.update(&solc_short);
        hasher.update(version.to_string());

        let input = serde_json::to_value(input)?;
        hasher.update(&serde_json::to_string(&input)?);

        // create the hash for `{_format,solcVersion,solcLongVersion,input}`
        // N.B. this is not exactly the same as hashing the json representation of these values but
        // the must efficient one
        let result = hasher.finalize();
        let id = hex::encode(result);

        let mut build_info = BTreeMap::new();

        if full_build_info {
            build_info.insert("_format".to_string(), serde_json::to_value(ETHERS_FORMAT_VERSION)?);
            build_info.insert("solcVersion".to_string(), serde_json::to_value(&solc_short)?);
            build_info.insert("solcLongVersion".to_string(), serde_json::to_value(&version)?);
            build_info.insert("input".to_string(), input);
            build_info.insert("output".to_string(), serde_json::to_value(output)?);
        }

        Ok(Self { id, build_info, build_context })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compilers::solc::SolcVersionedInput;
    use foundry_compilers_artifacts::{sources::Source, Contract, Error, SolcLanguage, Sources};
    use std::path::PathBuf;

    #[test]
    fn build_info_serde() {
        let v: Version = "0.8.4+commit.c7e474f2".parse().unwrap();
        let input = SolcVersionedInput::build(
            Sources::from([(PathBuf::from("input.sol"), Source::new(""))]),
            Default::default(),
            SolcLanguage::Solidity,
            v,
        );
        let output = CompilerOutput::<Error, Contract>::default();
        let raw_info = RawBuildInfo::new(&input, &output, true).unwrap();
        let _info: BuildInfo<SolcVersionedInput, CompilerOutput<Error, Contract>> =
            serde_json::from_str(&serde_json::to_string(&raw_info).unwrap()).unwrap();
    }
}
