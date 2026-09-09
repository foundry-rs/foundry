use alloy_primitives::{Bytes, map::AddressHashMap};
use foundry_cli::utils::{TraceResult, print_traces};
use foundry_common::{ContractsByArtifactBuilder, compile::ProjectCompiler};
use foundry_compilers::artifacts::output_selection::ContractOutputSelection;
use foundry_config::{Config, FoundryHardfork, TracingConfig};
use foundry_debugger::Debugger;
use foundry_evm::{
    opts::ForkEndpointIdentity,
    traces::{
        CallTraceDecoderBuilder, DebugTraceIdentifier, TraceContext,
        debug::ContractSources,
        identifier::{SignaturesIdentifier, TraceIdentifiers},
    },
};
use foundry_evm_networks::NetworkVariant;

pub(crate) fn select_remote_trace_hardfork(
    configured: Option<FoundryHardfork>,
    endpoint: Option<FoundryHardfork>,
    network: NetworkVariant,
) -> Option<FoundryHardfork> {
    let namespace = network.hardfork_namespace();
    configured
        .filter(|hardfork| hardfork.namespace() == namespace)
        .or_else(|| endpoint.filter(|hardfork| hardfork.namespace() == namespace))
}

/// Resolves the hardfork used to decode a trace executed by the remote node. A configured
/// hardfork is an explicit override; otherwise an Anvil endpoint's exact execution hardfork is
/// honored before consulting the source chain's schedule at `block_timestamp`.
pub(crate) fn resolve_remote_trace_hardfork(
    configured: Option<FoundryHardfork>,
    endpoint: &ForkEndpointIdentity,
    block_timestamp: Option<u64>,
) -> Option<FoundryHardfork> {
    select_remote_trace_hardfork(configured, endpoint.hardfork, endpoint.network).or_else(|| {
        block_timestamp.and_then(|timestamp| {
            FoundryHardfork::from_chain_and_timestamp(endpoint.source_chain_id, timestamp)
        })
    })
}

pub(crate) fn ensure_remote_trace_context_unchanged(
    before: &ForkEndpointIdentity,
    after: &ForkEndpointIdentity,
) -> eyre::Result<()> {
    if before != after {
        eyre::bail!(
            "the RPC endpoint changed execution context while the remote trace was being \
             collected; retry the command"
        );
    }
    Ok(())
}

/// labels the traces, conditionally prints them or opens the debugger
pub(crate) async fn handle_traces(
    mut result: TraceResult,
    config: &Config,
    context: TraceContext,
    contracts_bytecode: &AddressHashMap<Bytes>,
    tracing: &TracingConfig,
    with_local_artifacts: bool,
    debug: bool,
) -> eyre::Result<()> {
    let (known_contracts, mut sources) = if with_local_artifacts {
        // Status prose goes to stderr so `--json` output on stdout stays machine-readable.
        let _ = sh_status!("Compiling project to generate artifacts");
        let mut config = config.clone();
        if debug && !config.extra_output.contains(&ContractOutputSelection::StorageLayout) {
            config.extra_output.push(ContractOutputSelection::StorageLayout);
        }
        let project = config.project()?;
        let compiler = ProjectCompiler::new();
        let output = compiler.compile(&project)?;
        (
            Some(
                ContractsByArtifactBuilder::new(
                    output.artifact_ids().map(|(id, artifact)| (id, artifact.into())),
                )
                .with_storage_layouts(output.artifact_ids().filter_map(|(id, artifact)| {
                    artifact.storage_layout.as_ref().map(|layout| (id, layout.clone()))
                }))
                .build(),
            ),
            ContractSources::from_project_output(&output, project.root(), None)?,
        )
    } else {
        (None, ContractSources::default())
    };

    let mut builder = CallTraceDecoderBuilder::new()
        .with_tracing_config(tracing)
        .with_signature_identifier(SignaturesIdentifier::from_config(config)?)
        .with_networks(context.networks())
        .with_chain_id(Some(context.chain().id()))
        .with_hardfork(context.decoding_hardfork(config));
    let mut identifier = TraceIdentifiers::new().with_external(config, Some(context.chain()))?;
    if let Some(contracts) = &known_contracts {
        builder = builder.with_known_contracts(contracts);
        identifier = identifier.with_local_and_bytecodes(contracts, contracts_bytecode);
    }

    let mut decoder = builder.build();

    for (_, trace) in result.traces.as_deref_mut().unwrap_or_default() {
        decoder.identify(trace, &mut identifier);
    }

    if tracing.decode_internal || debug {
        if let Some(ref etherscan_identifier) = identifier.external {
            sources.merge(etherscan_identifier.get_compiled_contracts().await?);
        }

        if debug {
            let mut builder = Debugger::builder()
                .traces(result.traces.expect("missing traces"))
                .decoder(&decoder)
                .sources(sources);
            if let Some(known_contracts) = &known_contracts {
                builder = builder.known_contracts(known_contracts);
            }
            let mut debugger = builder.build();
            debugger.try_run_tui()?;
            return Ok(());
        }

        decoder.debug_identifier = Some(DebugTraceIdentifier::new(sources));
    }

    print_traces(
        &mut result,
        &decoder,
        tracing.verbosity > 0,
        tracing.verbosity > 4,
        tracing.trace_depth,
    )
    .await?;

    Ok(())
}

#[cfg(all(test, feature = "monad"))]
mod tests {
    use super::*;

    #[test]
    fn remote_trace_hardfork_ignores_cross_network_override() {
        let ethereum = FoundryHardfork::Ethereum(foundry_evm::hardforks::EthereumHardfork::Cancun);
        let monad_eight = FoundryHardfork::Monad(foundry_evm::hardforks::MonadHardfork::MonadEight);
        let monad_nine = FoundryHardfork::Monad(foundry_evm::hardforks::MonadHardfork::MonadNine);

        assert_eq!(
            select_remote_trace_hardfork(Some(ethereum), Some(monad_nine), NetworkVariant::Monad),
            Some(monad_nine)
        );
        assert_eq!(
            select_remote_trace_hardfork(
                Some(monad_eight),
                Some(monad_nine),
                NetworkVariant::Monad
            ),
            Some(monad_eight)
        );
    }
}
