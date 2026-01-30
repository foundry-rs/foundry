use alloy_eips::Encodable2718;
use alloy_network::AnyRpcTransaction;
use alloy_primitives::hex;
use alloy_provider::ext::TraceApi;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::RpcOpts,
    utils::{self, LoadConfig},
};
use foundry_common::stdin;

/// CLI arguments for `cast trace`.
#[derive(Debug, Parser)]
pub struct TraceArgs {
    /// Transaction hash (for trace_transaction) or raw tx hex/JSON (for trace_rawTransaction
    /// with --raw)
    tx: Option<String>,

    /// Use trace_rawTransaction instead of trace_transaction.
    /// Required when passing raw transaction hex or JSON instead of a tx hash.
    #[arg(long)]
    raw: bool,

    /// Include the basic trace of the transaction.
    #[arg(long, requires = "raw")]
    trace: bool,

    /// Include the full trace of the virtual machine's state during transaction execution
    #[arg(long, requires = "raw")]
    vm_trace: bool,

    /// Include state changes caused by the transaction (requires --raw).
    #[arg(long, requires = "raw")]
    state_diff: bool,

    #[command(flatten)]
    rpc: RpcOpts,
}

impl TraceArgs {
    pub async fn run(self) -> Result<()> {
        let config = self.rpc.load_config()?;
        let provider = utils::get_provider(&config)?;
        let input = stdin::unwrap_line(self.tx)?;

        let trimmed = input.trim();
        let is_json = trimmed.starts_with('{');
        let is_raw_hex = trimmed.starts_with("0x") && trimmed.len() > 66;

        let result = if self.raw {
            // trace_rawTransaction: accepts raw hex OR JSON tx
            let raw_bytes = if is_raw_hex {
                hex::decode(trimmed.strip_prefix("0x").unwrap_or(trimmed))?
            } else if is_json {
                let tx: AnyRpcTransaction = serde_json::from_str(trimmed)?;
                tx.inner.inner.encoded_2718().to_vec()
            } else {
                hex::decode(trimmed)?
            };

            let mut trace_builder = provider.trace_raw_transaction(&raw_bytes);

            if self.trace {
                trace_builder = trace_builder.trace();
            }
            if self.vm_trace {
                trace_builder = trace_builder.vm_trace();
            }
            if self.state_diff {
                trace_builder = trace_builder.state_diff();
            }

            if trace_builder.get_trace_types().map(|t| t.is_empty()).unwrap_or(true) {
                eyre::bail!("No trace type specified. Use --trace, --vm-trace, or --state-diff");
            }

            serde_json::to_string_pretty(&trace_builder.await?)?
        } else {
            // trace_transaction: use tx hash directly
            let hash = input.parse()?;
            let traces = provider.trace_transaction(hash).await?;
            serde_json::to_string_pretty(&traces)?
        };

        sh_println!("{}", result)?;
        Ok(())
    }
}
