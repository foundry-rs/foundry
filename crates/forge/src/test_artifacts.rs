//! Linked test artifacts and compiler analysis shared by execution engines.

use crate::test_config::TestFunctionMatcher;
use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, Bytes};
use eyre::Result;
use foundry_cli::opts::configure_pcx_from_compile_output;
use foundry_common::{
    ContractsByArtifact, ContractsByArtifactBuilder, EmptyTestFilter, LIBRARY_DEPLOYER,
};
use foundry_compilers::{Artifact, ArtifactId, ProjectCompileOutput};
use foundry_config::Config;
use foundry_evm::core::{decode::RevertDecoder, opts::EvmOpts};
use foundry_evm_fuzz::strategies::{EnumBounds, LiteralsDictionary};
use foundry_linking::{DetailedLinkOutput, LinkOutput, Linker, LinkerError, Resolver};
use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[cfg(feature = "revm")]
use foundry_compilers::artifacts::Libraries;

#[derive(Debug, Clone)]
pub struct TestContract {
    pub abi: JsonAbi,
    pub bytecode: Bytes,
    pub library_addresses: BTreeSet<Address>,
}

pub type DeployableContracts = BTreeMap<ArtifactId, TestContract>;

/// Forge-local library deployment strategy.
#[derive(Clone, Copy, Debug)]
pub enum LibraryDeployment {
    Nonce,
    Create2 { deployer: Address, salt: alloy_primitives::B256 },
}

/// Linked test artifacts and compiler analysis, prepared without an execution backend.
pub(crate) struct PreparedTestArtifacts {
    pub contracts: DeployableContracts,
    pub revert_decoder: RevertDecoder,
    pub known_contracts: ContractsByArtifact,
    pub libs_to_deploy: Vec<Bytes>,
    pub library_addresses: Vec<Address>,
    pub library_deployment: LibraryDeployment,
    #[cfg(feature = "revm")]
    pub libraries: Libraries,
    #[cfg(feature = "revm")]
    pub analysis: Arc<solar::sema::Compiler>,
    pub fuzz_literals: LiteralsDictionary,
    pub invariant_literals: LiteralsDictionary,
    pub enum_bounds: EnumBounds,
}

/// Prepares compiler output for any execution engine.
pub(crate) fn prepare_artifacts(
    config: &Config,
    line_coverage: bool,
    test_matcher: TestFunctionMatcher<'_>,
    output: &ProjectCompileOutput,
    evm_opts: &EvmOpts,
    create2_deployer_available: bool,
) -> Result<PreparedTestArtifacts> {
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

    // Build revert decoder from ABIs of all artifacts.
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

    let linked_contracts =
        linker.get_linked_artifacts_cow_with_artifact_libraries(&libraries, &artifact_libraries)?;

    // Collect every deployable test contract: a test contract with a default constructor.
    let mut deployable_contracts = DeployableContracts::default();
    let empty_filter = EmptyTestFilter::default();
    let resolver = Resolver::new(&linker);
    for (id, contract) in linked_contracts.iter() {
        let Some(abi) = contract.abi.as_ref() else { continue };
        if abi.constructor.as_ref().is_some_and(|c| !c.inputs.is_empty())
            || !test_matcher.matches_contract(&empty_filter, id, abi)
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

    // Create known contracts from linked contracts and storage layout information (if any).
    let known_contracts = ContractsByArtifactBuilder::new(linked_contracts)
        .with_storage_layouts(output.artifact_ids().filter_map(|(id, artifact)| {
            artifact.storage_layout.as_ref().map(|layout| (artifact_id(id), layout.clone()))
        }))
        .build();

    // Initialize and configure the solar compiler.
    let mut analysis = solar::sema::Compiler::new(
        solar::interface::Session::builder().with_stderr_emitter().build(),
    );
    let dcx = analysis.dcx_mut();
    dcx.set_emitter(Box::new(
        solar::interface::diagnostics::HumanEmitter::stderr(Default::default())
            .source_map(Some(dcx.source_map().unwrap())),
    ));
    dcx.set_flags_mut(|f| f.track_diagnostics = false);

    // Populate solar's global context by parsing and lowering the sources.
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
    let analysis = Arc::new(analysis);

    // Enum variant counts used to constrain fuzzed enum inputs to valid values.
    let enum_bounds = EnumBounds::collect(&analysis);
    let literals = |max_literals| {
        LiteralsDictionary::new(Some(analysis.clone()), Some(config.project_paths()), max_literals)
    };
    let fuzz_max_literals = config.fuzz.dictionary.max_fuzz_dictionary_literals;
    let invariant_max_literals = config.invariant.dictionary.max_fuzz_dictionary_literals;
    let fuzz_literals = literals(fuzz_max_literals);
    let invariant_literals = if invariant_max_literals == fuzz_max_literals {
        fuzz_literals.clone()
    } else {
        literals(invariant_max_literals)
    };

    Ok(PreparedTestArtifacts {
        contracts: deployable_contracts,
        revert_decoder,
        known_contracts,
        libs_to_deploy,
        library_addresses,
        library_deployment,
        #[cfg(feature = "revm")]
        libraries,
        #[cfg(feature = "revm")]
        analysis,
        fuzz_literals,
        invariant_literals,
        enum_bounds,
    })
}
