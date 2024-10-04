use alloy_primitives::{Address, Bytes};
use alloy_provider::{ext::TraceApi, Provider};
use alloy_rpc_types::trace::parity::{Action, CreateAction, CreateOutput, TraceOutput};
use clap::{command, Parser};
use evm_disassembler::{disassemble_bytes, format_operations};
use eyre::Result;
use foundry_block_explorers::Client;
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils,
};
use foundry_common::provider::RetryProvider;
use foundry_config::Config;

/// CLI arguments for `cast creation-code`.
#[derive(Parser)]
pub struct CreationCodeArgs {
    /// An Ethereum address, for which the bytecode will be fetched.
    contract: Address,

    /// Disassemble bytecodes into individual opcodes.
    #[arg(long)]
    disassemble: bool,

    #[command(flatten)]
    etherscan: EtherscanOpts,
    #[command(flatten)]
    rpc: RpcOpts,
}

impl CreationCodeArgs {
    pub async fn run(self) -> Result<()> {
        let Self { contract, etherscan, rpc, disassemble } = self;
        let config = Config::from(&etherscan);
        let chain = config.chain.unwrap_or_default();
        let api_key = config.get_etherscan_api_key(Some(chain)).unwrap_or_default();
        let client = Client::new(chain, api_key)?;

        let config = Config::from(&rpc);
        let provider = utils::get_provider(&config)?;

        let bytecode = fetch_creation_code(contract, client, provider).await?;

        if disassemble {
            print!("{}", format_operations(disassemble_bytes(bytecode.into())?)?);
        } else {
            print!("{bytecode}");
        }

        Ok(())
    }
}

/// Fetches the creation code of a contract from Etherscan and RPC.
///
/// If present, constructor arguments are appended to the end of the bytecode.
async fn fetch_creation_code(
    contract: Address,
    client: Client,
    provider: RetryProvider,
) -> Result<Bytes> {
    let creation_data = client.contract_creation_data(contract).await?;
    let creation_tx_hash = creation_data.transaction_hash;
    let tx_data = provider.get_transaction_by_hash(creation_tx_hash).await?;
    let tx_data = tx_data.ok_or_else(|| eyre::eyre!("Could not find creation tx data."))?;

    let bytecode = if tx_data.inner.to.is_none() {
        // Contract was created using a standard transaction
        tx_data.inner.input
    } else {
        // Contract was created using a factory pattern or create2
        // Extract creation code from tx traces
        let mut creation_bytecode = None;

        let traces = provider.trace_transaction(creation_tx_hash).await.map_err(|e| {
            eyre::eyre!("Could not fetch traces for transaction {}: {}", creation_tx_hash, e)
        })?;

        for trace in traces {
            if let Some(TraceOutput::Create(CreateOutput { address, code: _, gas_used: _ })) =
                trace.trace.result
            {
                if address == contract {
                    creation_bytecode = match trace.trace.action {
                        Action::Create(CreateAction { init, value: _, from: _, gas: _ }) => {
                            Some(init)
                        }
                        _ => None,
                    };
                }
            }
        }

        creation_bytecode.ok_or_else(|| eyre::eyre!("Could not find contract creation trace."))?
    };

    Ok(bytecode)
}
