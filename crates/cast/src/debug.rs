use std::str::FromStr;

use alloy_chains::Chain;
use alloy_primitives::{Address, Bytes, map::HashMap};
use foundry_cli::utils::{TraceResult, print_traces};
use foundry_common::{ContractsByArtifact, compile::ProjectCompiler, shell};
use foundry_config::Config;
use foundry_debugger::Debugger;
use foundry_evm::traces::{
    CallTraceDecoderBuilder, DebugTraceIdentifier,
    debug::ContractSources,
    identifier::{SignaturesIdentifier, TraceIdentifiers},
};

/// labels the traces, conditionally prints them or opens the debugger
#[expect(clippy::too_many_arguments)]
pub(crate) async fn handle_traces(
    mut result: TraceResult,
    config: &Config,
    chain: Chain,
    contracts_bytecode: &HashMap<Address, Bytes>,
    labels: Vec<String>,
    with_local_artifacts: bool,
    debug: bool,
    decode_internal: bool,
    disable_label: bool,
    trace_depth: Option<usize>,
) -> eyre::Result<()> {
    let (known_contracts, mut sources) = if with_local_artifacts {
        let _ = sh_println!("Compiling project to generate artifacts");
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

    let labels = labels.iter().filter_map(|label_str| {
        let mut iter = label_str.split(':');

        if let Some(addr) = iter.next()
            && let (Ok(address), Some(label)) = (Address::from_str(addr), iter.next())
        {
            return Some((address, label.to_string()));
        }
        None
    });
    let config_labels = config.labels.clone().into_iter();

    let mut builder = CallTraceDecoderBuilder::new()
        .with_labels(labels.chain(config_labels))
        .with_signature_identifier(SignaturesIdentifier::from_config(config)?)
        .with_label_disabled(disable_label);
    let mut identifier = TraceIdentifiers::new().with_external(config, Some(chain))?;
    if let Some(contracts) = &known_contracts {
        builder = builder.with_known_contracts(contracts);
        identifier = identifier.with_local_and_bytecodes(contracts, contracts_bytecode);
    }

    let mut decoder = builder.build();

    for (_, trace) in result.traces.as_deref_mut().unwrap_or_default() {
        decoder.identify(trace, &mut identifier);
    }

    if decode_internal || debug {
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
        shell::verbosity() > 0,
        shell::verbosity() > 4,
        trace_depth,
    )
    .await?;

    Ok(())
}
