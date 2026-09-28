//! Linked test artifacts shared by Forge execution runners.

use crate::test_matcher::{SymbolicArtifactReplayConfig, TestFunctionMatcher};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, B256, Bytes};
use eyre::Result;
use foundry_cli::opts::configure_pcx_from_compile_output;
use foundry_common::{
    ContractsByArtifact, ContractsByArtifactBuilder, EmptyTestFilter, LIBRARY_DEPLOYER,
};
use foundry_compilers::{Artifact, ArtifactId, ProjectCompileOutput, artifacts::Libraries};
use foundry_config::{Config, InlineConfig};
use foundry_evm::{decode::RevertDecoder, opts::EvmOpts};
use foundry_linking::{DetailedLinkOutput, LinkOutput, Linker, LinkerError, Resolver};
use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// A deployable test contract and the libraries linked into its bytecode.
#[derive(Debug, Clone)]
pub struct TestContract {
    pub abi: JsonAbi,
    pub bytecode: Bytes,
    pub library_addresses: BTreeSet<Address>,
}

/// Test contracts indexed by compiler artifact ID.
pub type DeployableContracts = BTreeMap<ArtifactId, TestContract>;

/// Forge-local library deployment strategy.
#[derive(Clone, Copy, Debug)]
pub enum LibraryDeployment {
    Nonce,
    Create2 { deployer: Address, salt: B256 },
}

/// Compiler output prepared without constructing an EVM executor.
pub(crate) struct PreparedTestArtifacts {
    pub contracts: DeployableContracts,
    pub known_contracts: ContractsByArtifact,
    pub revert_decoder: RevertDecoder,
    pub libs_to_deploy: Vec<Bytes>,
    pub library_addresses: Vec<Address>,
    pub library_deployment: LibraryDeployment,
    pub libraries: Libraries,
}

impl PreparedTestArtifacts {
    pub fn new(
        config: &Config,
        inline_config: &InlineConfig,
        symbolic_artifact_replay: Option<&SymbolicArtifactReplayConfig>,
        output: &ProjectCompileOutput,
        evm_opts: &EvmOpts,
        line_coverage: bool,
        create2_deployer_available: bool,
    ) -> Result<Self> {
        let root = &config.root;
        let coverage_artifacts = line_coverage
            .then(|| config.coverage_cache_path())
            .flatten()
            .map(|path| path.join("artifacts"));
        let artifact_id = |mut id: ArtifactId| {
            // Artifact-path cheatcodes retain the logical output paths even when coverage
            // compilation stores its unoptimized artifacts in a separate cache.
            if let Some(coverage_artifacts) = &coverage_artifacts
                && let Ok(path) = id.path.strip_prefix(coverage_artifacts)
            {
                id.path = config.out.join(path);
            }
            id.with_stripped_file_prefixes(root).with_slashed_paths()
        };
        let contracts = output.artifact_ids().map(|(id, v)| (artifact_id(id), v)).collect();
        let linker = Linker::new(root, contracts);

        let abis = linker
            .contracts
            .values()
            .filter_map(|contract| contract.abi.as_ref().map(|abi| abi.borrow()));
        let revert_decoder = RevertDecoder::new().with_abis(abis);

        let configured_libraries = config.libraries_with_remappings()?;
        let create2 = if create2_deployer_available {
            match linker.link_with_create2_detailed(
                configured_libraries.clone(),
                evm_opts.create2_deployer,
                config.create2_library_salt,
                linker.contracts.keys(),
            ) {
                Ok(output) => Some(output),
                Err(LinkerError::CyclicDependency) => None,
                Err(err) => return Err(err.into()),
            }
        } else {
            None
        };
        let (
            DetailedLinkOutput {
                output: LinkOutput { libraries, library_addresses, libs_to_deploy },
                artifact_libraries,
                ..
            },
            library_deployment,
        ) = match create2 {
            Some(output) => {
                let deployment = if output.output.libs_to_deploy.is_empty() {
                    LibraryDeployment::Nonce
                } else {
                    LibraryDeployment::Create2 {
                        deployer: evm_opts.create2_deployer,
                        salt: config.create2_library_salt,
                    }
                };
                (output, deployment)
            }
            None => (
                linker.link_with_nonce_or_address_detailed(
                    configured_libraries,
                    LIBRARY_DEPLOYER,
                    0,
                    linker.contracts.keys(),
                )?,
                LibraryDeployment::Nonce,
            ),
        };

        let linked_contracts = linker
            .get_linked_artifacts_cow_with_artifact_libraries(&libraries, &artifact_libraries)?;
        let matcher = TestFunctionMatcher::new(config, inline_config, symbolic_artifact_replay);
        let filter = EmptyTestFilter::default();
        let resolver = Resolver::new(&linker);
        let mut deployable_contracts = DeployableContracts::default();
        for (id, contract) in linked_contracts.iter() {
            let Some(abi) = contract.abi.as_ref() else { continue };
            if abi.constructor.as_ref().is_some_and(|c| !c.inputs.is_empty())
                || !matcher.matches_contract(&filter, id, abi)
            {
                continue;
            }
            linker.ensure_linked(contract, id)?;
            let Some(bytecode) =
                contract.get_bytecode_bytes().map(|b| b.into_owned()).filter(|b| !b.is_empty())
            else {
                continue;
            };
            let artifact_libraries = artifact_libraries.get(id).unwrap_or(&libraries);
            let library_addresses = resolver.linked_library_addresses(id, artifact_libraries)?;
            deployable_contracts.insert(
                id.clone(),
                TestContract { abi: abi.clone().into_owned(), bytecode, library_addresses },
            );
        }

        let known_contracts = ContractsByArtifactBuilder::new(linked_contracts)
            .with_storage_layouts(output.artifact_ids().filter_map(|(id, artifact)| {
                artifact.storage_layout.as_ref().map(|layout| (artifact_id(id), layout.clone()))
            }))
            .build();
        Ok(Self {
            contracts: deployable_contracts,
            known_contracts,
            revert_decoder,
            libs_to_deploy,
            library_addresses,
            library_deployment,
            libraries,
        })
    }
}

/// Parses compiled sources for Forge test analysis without an EVM executor.
pub(crate) fn analyze_compiled_sources(
    config: &Config,
    output: &ProjectCompileOutput,
    line_coverage: bool,
) -> Result<Arc<solar::sema::Compiler>> {
    let mut analysis = solar::sema::Compiler::new(
        solar::interface::Session::builder().with_stderr_emitter().build(),
    );
    let dcx = analysis.dcx_mut();
    dcx.set_emitter(Box::new(
        solar::interface::diagnostics::HumanEmitter::stderr(Default::default())
            .source_map(Some(dcx.source_map().unwrap())),
    ));
    dcx.set_flags_mut(|f| f.track_diagnostics = false);

    let files: Vec<_> = output.output().sources.as_ref().keys().cloned().collect();
    analysis.enter_mut(|compiler| -> Result<()> {
        let mut pcx = compiler.parse();
        configure_pcx_from_compile_output(
            &mut pcx,
            config,
            output,
            (!line_coverage && !files.is_empty()).then_some(&files),
        )?;
        pcx.parse();
        let _ = compiler.lower_asts();
        Ok(())
    })?;
    Ok(Arc::new(analysis))
}
