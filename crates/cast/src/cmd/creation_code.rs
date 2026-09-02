use super::interface::load_abi_from_file;
use crate::SimpleCast;
use alloy_consensus::Transaction;
use alloy_network::AnyNetwork;
use alloy_primitives::{Address, Bytes};
use alloy_provider::{Provider, RootProvider, ext::TraceApi};
use alloy_rpc_types::trace::parity::{Action, CreateAction, CreateOutput, TraceOutput};
use clap::Parser;
use eyre::{OptionExt, Result, eyre};
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::{self, LoadConfig, fetch_abi_from_etherscan},
};
use foundry_config::Config;

foundry_config::impl_figment_convert!(CreationCodeArgs, etherscan, rpc);

/// CLI arguments for `cast creation-code`.
#[derive(Parser)]
pub struct CreationCodeArgs {
    /// An Ethereum address, for which the bytecode will be fetched.
    contract: Address,

    /// Path to file containing the contract's JSON ABI. It's necessary if the target contract is
    /// not verified on Etherscan.
    #[arg(long)]
    abi_path: Option<String>,

    /// Disassemble bytecodes into individual opcodes.
    #[arg(long)]
    disassemble: bool,

    /// Return creation bytecode without constructor arguments appended.
    #[arg(long, conflicts_with = "only_args")]
    without_args: bool,

    /// Return only constructor arguments.
    #[arg(long)]
    only_args: bool,

    #[command(flatten)]
    etherscan: EtherscanOpts,

    #[command(flatten)]
    rpc: RpcOpts,
}

impl CreationCodeArgs {
    pub async fn run(self) -> Result<()> {
        let mut config = self.load_config()?;

        let Self { contract, disassemble, without_args, only_args, abi_path, etherscan: _, rpc: _ } =
            self;

        let provider = utils::get_provider(&config)?;
        let chain = provider.get_chain_id().await?;
        config.chain = Some(chain.into());

        let bytecode = fetch_creation_code_from_etherscan(contract, &config, provider).await?;

        let bytecode = parse_code_output(
            bytecode,
            contract,
            &config,
            abi_path.as_deref(),
            without_args,
            only_args,
        )
        .await?;

        if disassemble {
            let _ = sh_println!("{}", SimpleCast::disassemble(&bytecode)?);
        } else {
            let _ = sh_println!("{bytecode}");
        }

        Ok(())
    }
}

/// Parses the creation bytecode and returns one of the following:
/// - The complete bytecode
/// - The bytecode without constructor arguments
/// - Only the constructor arguments
pub async fn parse_code_output(
    bytecode: Bytes,
    contract: Address,
    config: &Config,
    abi_path: Option<&str>,
    without_args: bool,
    only_args: bool,
) -> Result<Bytes> {
    if !without_args && !only_args {
        return Ok(bytecode);
    }

    let abi = if let Some(abi_path) = abi_path {
        load_abi_from_file(abi_path, None)?
    } else {
        fetch_abi_from_etherscan(contract, config).await?
    };

    let abi = abi.into_iter().next().ok_or_eyre("No ABI found.")?;
    let (abi, _) = abi;

    if abi.constructor.is_none() {
        if only_args {
            return Err(eyre!("No constructor found."));
        }
        return Ok(bytecode);
    }

    let constructor = abi.constructor.unwrap();
    if constructor.inputs.is_empty() {
        if only_args {
            return Err(eyre!("No constructor arguments found."));
        }
        return Ok(bytecode);
    }

    let args_size = constructor.inputs.len() * 32;
    if bytecode.len() < args_size {
        return Err(eyre!(
            "Invalid creation bytecode length: have {} bytes, need at least {} for {} constructor inputs",
            bytecode.len(),
            args_size,
            constructor.inputs.len()
        ));
    }

    let bytecode = if without_args {
        Bytes::from(bytecode[..bytecode.len() - args_size].to_vec())
    } else if only_args {
        Bytes::from(bytecode[bytecode.len() - args_size..].to_vec())
    } else {
        unreachable!();
    };

    Ok(bytecode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // Constructor ABI declaring 2 uint256 inputs (64 bytes encoded), paired with a
    // deployed bytecode shorter than that - e.g. an EIP-1167 minimal proxy is only
    // ~45 bytes. `--abi-path` is user-supplied and documented as not needing to match
    // the actually-deployed contract, so this combination is directly reachable.
    const TWO_UINT_CONSTRUCTOR_ABI: &str = r#"[{
        "type": "constructor",
        "stateMutability": "nonpayable",
        "inputs": [
            {"name": "a", "type": "uint256", "internalType": "uint256"},
            {"name": "b", "type": "uint256", "internalType": "uint256"}
        ]
    }]"#;

    fn write_abi_file(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file
    }

    // 20 bytes of "code" (0xAA) followed by 64 bytes of "args" (0xBB), so a correct
    // split is verifiable by content, not just by length.
    fn code_plus_args_bytecode() -> Bytes {
        let mut b = vec![0xAAu8; 20];
        b.extend(vec![0xBBu8; 64]);
        Bytes::from(b)
    }

    #[tokio::test]
    async fn without_args_errors_instead_of_panicking_on_undersized_bytecode() {
        let abi_file = write_abi_file(TWO_UINT_CONSTRUCTOR_ABI);
        // 20-byte bytecode, but the ABI declares 64 bytes of constructor args.
        let bytecode = Bytes::from(vec![0u8; 20]);

        let result = parse_code_output(
            bytecode,
            Address::ZERO,
            &Config::default(),
            Some(abi_file.path().to_str().unwrap()),
            true,
            false,
        )
        .await;

        assert!(result.is_err(), "expected an error, not a panic, on undersized bytecode");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Invalid creation bytecode length"), "unexpected message: {msg}");
    }

    #[tokio::test]
    async fn only_args_errors_instead_of_panicking_on_undersized_bytecode() {
        let abi_file = write_abi_file(TWO_UINT_CONSTRUCTOR_ABI);
        let bytecode = Bytes::from(vec![0u8; 20]);

        let result = parse_code_output(
            bytecode,
            Address::ZERO,
            &Config::default(),
            Some(abi_file.path().to_str().unwrap()),
            false,
            true,
        )
        .await;

        assert!(result.is_err(), "expected an error, not a panic, on undersized bytecode");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Invalid creation bytecode length"), "unexpected message: {msg}");
    }

    #[tokio::test]
    async fn errors_instead_of_panicking_at_the_exact_equality_boundary() {
        let abi_file = write_abi_file(TWO_UINT_CONSTRUCTOR_ABI);
        // bytecode.len() == args_size exactly: no room for any actual code.
        let bytecode = Bytes::from(vec![0u8; 64]);

        let result = parse_code_output(
            bytecode.clone(),
            Address::ZERO,
            &Config::default(),
            Some(abi_file.path().to_str().unwrap()),
            true,
            false,
        )
        .await;
        assert_eq!(result.unwrap().len(), 0, "without_args should return empty code, not error");

        let result = parse_code_output(
            bytecode,
            Address::ZERO,
            &Config::default(),
            Some(abi_file.path().to_str().unwrap()),
            false,
            true,
        )
        .await;
        assert_eq!(result.unwrap().len(), 64, "only_args should return the whole slice");
    }

    #[tokio::test]
    async fn without_args_still_succeeds_on_correctly_sized_bytecode() {
        let abi_file = write_abi_file(TWO_UINT_CONSTRUCTOR_ABI);
        let bytecode = code_plus_args_bytecode();

        let result = parse_code_output(
            bytecode,
            Address::ZERO,
            &Config::default(),
            Some(abi_file.path().to_str().unwrap()),
            true,
            false,
        )
        .await
        .unwrap();

        assert_eq!(result, Bytes::from(vec![0xAAu8; 20]));
    }

    #[tokio::test]
    async fn only_args_still_succeeds_on_correctly_sized_bytecode() {
        let abi_file = write_abi_file(TWO_UINT_CONSTRUCTOR_ABI);
        let bytecode = code_plus_args_bytecode();

        let result = parse_code_output(
            bytecode,
            Address::ZERO,
            &Config::default(),
            Some(abi_file.path().to_str().unwrap()),
            false,
            true,
        )
        .await
        .unwrap();

        assert_eq!(result, Bytes::from(vec![0xBBu8; 64]));
    }
}

/// Fetches the creation code of a contract from Etherscan and RPC.
pub async fn fetch_creation_code_from_etherscan(
    contract: Address,
    config: &Config,
    provider: RootProvider<AnyNetwork>,
) -> Result<Bytes> {
    let chain = config.chain.unwrap_or_default();
    let client = config
        .get_etherscan_config_with_chain(Some(chain))?
        .ok_or_else(|| eyre!("No Etherscan API key configured for chain {chain}"))?
        .into_client_with_no_proxy(config.eth_rpc_no_proxy)?;
    let creation_data = client.contract_creation_data(contract).await?;
    let creation_tx_hash = creation_data.transaction_hash;
    let tx_data = provider.get_transaction_by_hash(creation_tx_hash).await?;
    let tx_data = tx_data.ok_or_eyre("Could not find creation tx data.")?;

    let bytecode = if tx_data.to().is_none() {
        // Contract was created using a standard transaction
        tx_data.input().clone()
    } else {
        // Contract was created using a factory pattern or create2
        // Extract creation code from tx traces
        let mut creation_bytecode = None;

        let traces = provider.trace_transaction(creation_tx_hash).await.map_err(|e| {
            eyre!("Could not fetch traces for transaction {}: {}", creation_tx_hash, e)
        })?;

        for trace in traces {
            if let Some(TraceOutput::Create(CreateOutput { address, .. })) = trace.trace.result
                && address == contract
            {
                creation_bytecode = match trace.trace.action {
                    Action::Create(CreateAction { init, .. }) => Some(init),
                    _ => None,
                };
            }
        }

        creation_bytecode.ok_or_else(|| eyre!("Could not find contract creation trace."))?
    };

    Ok(bytecode)
}
