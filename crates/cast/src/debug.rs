use alloy_chains::Chain;
use alloy_primitives::{Bytes, map::AddressHashMap};
use foundry_cli::{
    opts::TracingArgs,
    utils::{TraceResult, print_traces},
};
use foundry_common::{ContractsByArtifact, compile::ProjectCompiler, shell};
use foundry_config::Config;
use foundry_debugger::Debugger;
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
    tracing: &TracingArgs,
    with_local_artifacts: bool,
    debug: bool,
    tempo_hardfork: Option<TempoHardfork>,
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

    let labels = tracing.parsed_labels();
    let config_labels = config.labels.clone().into_iter();

    let mut builder = CallTraceDecoderBuilder::new()
        .with_labels(labels.into_iter().chain(config_labels))
        .with_signature_identifier(SignaturesIdentifier::from_config(config)?)
        .with_label_disabled(tracing.disable_labels(&config.tracing))
        .with_chain_id(Some(chain.id()))
        .with_tempo_hardfork(
            tempo_hardfork
                .or_else(|| chain.is_tempo().then(|| config.evm_spec_id::<TempoHardfork>())),
        );
    let mut identifier = TraceIdentifiers::new().with_external(config, Some(chain))?;
    if let Some(contracts) = &known_contracts {
        builder = builder.with_known_contracts(contracts);
        identifier = identifier.with_local_and_bytecodes(contracts, contracts_bytecode);
    }

    let mut decoder = builder.build();

    for (_, trace) in result.traces.as_deref_mut().unwrap_or_default() {
        decoder.identify(trace, &mut identifier);
    }

    if tracing.decode_internal(&config.tracing) || debug {
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

    let verbosity = shell::verbosity().max(config.tracing.verbosity);
    print_traces(
        &mut result,
        &decoder,
        verbosity > 0,
        verbosity > 4,
        tracing.trace_depth(&config.tracing),
    )
    .await?;

    Ok(())
}
