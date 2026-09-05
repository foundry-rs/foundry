use crate::cmd::rpc_provider;
use alloy_consensus::Typed2718;
use alloy_network::AnyRpcTransaction;
use alloy_primitives::hex;
use alloy_provider::ext::TraceApi;
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::opts::RpcOpts;
use foundry_common::stdin;
use foundry_primitives::FoundryTxEnvelope;

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
        let provider = rpc_provider(&self.rpc)?;
        let input = stdin::unwrap_line(self.tx)?;

        let result = if self.raw {
            // trace_rawTransaction: accepts raw hex OR JSON tx
            let trimmed = input.trim();
            let raw_bytes = if trimmed.starts_with('{') {
                let tx: AnyRpcTransaction = serde_json::from_str(trimmed)?;
                FoundryTxEnvelope::encode_rpc_2718(&tx)
                    .wrap_err_with(|| {
                        format!("Cannot EIP-2718 encode transaction type 0x{:x}", tx.ty())
                    })?
                    .to_vec()
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
            if trace_builder.get_trace_types().is_none_or(|t| t.is_empty()) {
                eyre::bail!("No trace type specified. Use --trace, --vm-trace, or --state-diff");
            }

            serde_json::to_string_pretty(&trace_builder.await?)?
        } else {
            // trace_transaction: use tx hash directly
            serde_json::to_string_pretty(&provider.trace_transaction(input.parse()?).await?)?
        };

        sh_println!("{}", result)?;
        Ok(())
    }
}
