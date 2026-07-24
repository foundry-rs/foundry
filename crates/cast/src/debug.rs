use alloy_chains::Chain;
use alloy_primitives::{Bytes, map::AddressHashMap};
use foundry_cli::utils::{TraceResult, print_traces};
use foundry_common::{ContractsByArtifact, compile::ProjectCompiler};
#[cfg(feature = "monad")]
use foundry_config::NamedChain;
use foundry_config::{Config, FoundryHardfork, TracingConfig};
use foundry_debugger::Debugger;
#[cfg(feature = "monad")]
use foundry_evm::hardforks::MonadHardfork;
use foundry_evm::{
    hardforks::TempoHardfork,
    traces::{
        CallTraceDecoderBuilder, DebugTraceIdentifier,
        debug::ContractSources,
        identifier::{SignaturesIdentifier, TraceIdentifiers},
    },
};

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
    let tempo_hardfork = resolved_hardfork.and_then(|hardfork| match hardfork {
        FoundryHardfork::Tempo(hardfork) => Some(hardfork),
        _ => None,
    });
    let is_tempo = tempo_hardfork.is_some() || chain.is_tempo();
    #[cfg(feature = "monad")]
    let is_monad = config.networks.is_monad()
        || matches!(chain.named(), Some(NamedChain::Monad | NamedChain::MonadTestnet));
    #[cfg(feature = "monad")]
    let monad_hardfork = resolved_hardfork.and_then(|hardfork| match hardfork {
        FoundryHardfork::Monad(hardfork) => Some(hardfork),
        _ => None,
    });
    let mut builder = CallTraceDecoderBuilder::new()
        .with_tracing_config(tracing)
        .with_signature_identifier(SignaturesIdentifier::from_config(config)?)
        .with_chain_id((!is_tempo).then(|| chain.id()))
        .with_tempo_hardfork(
            tempo_hardfork
                .or_else(|| chain.is_tempo().then(|| config.evm_spec_id::<TempoHardfork>())),
        );
    #[cfg(feature = "monad")]
    {
        builder = builder.with_monad_hardfork(
            monad_hardfork.or_else(|| is_monad.then(|| config.evm_spec_id::<MonadHardfork>())),
        );
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
