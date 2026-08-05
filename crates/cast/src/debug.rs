use alloy_chains::Chain;
#[cfg(test)]
use alloy_primitives::B256;
use alloy_primitives::{Bytes, map::AddressHashMap};
use foundry_cli::utils::{TraceResult, print_traces};
use foundry_common::{ContractsByArtifact, compile::ProjectCompiler};
use foundry_config::{Config, FoundryHardfork, TracingConfig};
use foundry_debugger::Debugger;
#[cfg(all(test, feature = "monad"))]
use foundry_evm::hardforks::EthereumHardfork;
#[cfg(feature = "monad")]
use foundry_evm::hardforks::MonadHardfork;
#[cfg(feature = "base")]
use foundry_evm::{
    core::evm::{BaseEvmNetwork, SpecFor},
    hardforks::{BaseUpgrade, ExecutionSpec},
};
use foundry_evm::{
    hardforks::TempoHardfork,
    opts::ForkEndpointIdentity,
    traces::{
        CallTraceDecoderBuilder, DebugTraceIdentifier,
        debug::ContractSources,
        identifier::{SignaturesIdentifier, TraceIdentifiers},
    },
};
use foundry_evm_networks::{NetworkConfigs, NetworkVariant};

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
#[expect(clippy::too_many_arguments)]
pub(crate) async fn handle_traces(
    mut result: TraceResult,
    config: &Config,
    chain: Chain,
    contracts_bytecode: &AddressHashMap<Bytes>,
    tracing: &TracingConfig,
    with_local_artifacts: bool,
    debug: bool,
    hardfork: Option<FoundryHardfork>,
    networks: NetworkConfigs,
) -> eyre::Result<()> {
    let (known_contracts, mut sources) = if with_local_artifacts {
        // Status prose goes to stderr so `--json` output on stdout stays machine-readable.
        let _ = sh_status!("Compiling project to generate artifacts");
        let project = config.project()?;
        let compiler = ProjectCompiler::new();
        let output = compiler.compile(&project)?;
        (
            Some(ContractsByArtifact::new(
                output.artifact_ids().map(|(id, artifact)| (id, artifact.clone().into())),
            )),
            ContractSources::from_project_output(&output, project.root(), None)?,
        )
    } else {
        (None, ContractSources::default())
    };

    let resolved_hardfork = hardfork.or(config.hardfork);
    let execution_network = networks.execution_network();
    let tempo_hardfork = resolved_hardfork.and_then(|hardfork| match hardfork {
        FoundryHardfork::Tempo(hardfork) => Some(hardfork),
        _ => None,
    });
    let is_tempo = execution_network.is_tempo();
    #[cfg(feature = "monad")]
    let is_monad = execution_network.is_monad();
    #[cfg(feature = "monad")]
    let monad_hardfork = resolved_hardfork.and_then(|hardfork| match hardfork {
        FoundryHardfork::Monad(hardfork) => Some(hardfork),
        _ => None,
    });
    let mut builder = CallTraceDecoderBuilder::new()
        .with_tracing_config(tracing)
        .with_signature_identifier(SignaturesIdentifier::from_config(config)?)
        .with_networks(networks)
        .with_chain_id(Some(chain.id()))
        .with_tempo_hardfork(
            tempo_hardfork.or_else(|| is_tempo.then(|| config.evm_spec_id::<TempoHardfork>())),
        );
    #[cfg(feature = "monad")]
    {
        builder = builder.with_monad_hardfork(
            monad_hardfork.or_else(|| is_monad.then(|| config.evm_spec_id::<MonadHardfork>())),
        );
    }
    #[cfg(feature = "base")]
    if execution_network.is_base() {
        let upgrade = resolved_hardfork
            .and_then(|hardfork| match hardfork {
                FoundryHardfork::Base(upgrade) => Some(upgrade),
                _ => None,
            })
            .or_else(|| {
                config
                    .evm_spec_id::<SpecFor<BaseEvmNetwork>>()
                    .evm_version_name()
                    .parse::<BaseUpgrade>()
                    .ok()
            });
        builder = builder.with_base_upgrade(upgrade);
    }
    let mut identifier = TraceIdentifiers::new().with_external(config, Some(chain))?;
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
            let mut debugger = Debugger::builder()
                .traces(result.traces.expect("missing traces"))
                .decoder(&decoder)
                .sources(sources)
                .build();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_trace_context_rejects_same_url_reset() {
        let before = ForkEndpointIdentity {
            endpoint: "http://localhost:8545".to_string(),
            execution_chain_id: 1,
            source_chain_id: 1,
            network: NetworkVariant::Ethereum,
            network_profile: foundry_evm_networks::NetworkConfigs::default(),
            reported_hardfork: None,
            hardfork: None,
            instance_id: Some(B256::with_last_byte(1)),
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };
        assert!(ensure_remote_trace_context_unchanged(&before, &before).is_ok());

        let mut after = before.clone();
        after.instance_id = Some(B256::with_last_byte(2));
        assert!(ensure_remote_trace_context_unchanged(&before, &after).is_err());

        let mut before_unknown = before;
        before_unknown.instance_id = None;
        before_unknown.reported_hardfork = Some("FutureA".to_string());
        let mut after_unknown = before_unknown.clone();
        after_unknown.reported_hardfork = Some("FutureB".to_string());
        assert!(ensure_remote_trace_context_unchanged(&before_unknown, &after_unknown).is_err());
    }

    #[test]
    #[cfg(feature = "monad")]
    fn remote_trace_hardfork_ignores_cross_network_override() {
        let ethereum = FoundryHardfork::Ethereum(EthereumHardfork::Cancun);
        let monad_eight = FoundryHardfork::Monad(MonadHardfork::MonadEight);
        let monad_nine = FoundryHardfork::Monad(MonadHardfork::MonadNine);

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
