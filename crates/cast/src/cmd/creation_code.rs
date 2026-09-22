use super::interface::load_abi_from_file;
use alloy_consensus::Transaction;
use alloy_dyn_abi::{DynSolType, Specifier};
use alloy_json_abi::{Constructor, JsonAbi};
use alloy_primitives::{Address, Bytes};
use alloy_provider::{Provider, ext::TraceApi};
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
        let Self { contract, disassemble, without_args, only_args, abi_path, .. } = self;

        let bytecode = fetch_creation_code(&mut config, contract).await?;
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
            sh_println!("{}", super::disassemble(&bytecode)?)?;
        } else {
            sh_println!("{bytecode}")?;
        }
        Ok(())
    }
}

/// Parses the creation bytecode and returns one of the following:
/// - The complete bytecode
/// - The bytecode without constructor arguments
/// - Only the constructor arguments
pub(crate) async fn parse_code_output(
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

    let abi = load_abi(contract, config, abi_path).await?;
    let constructor = match constructor_with_args(&abi) {
        Ok(constructor) => constructor,
        Err(e) if only_args => return Err(e),
        Err(_) => return Ok(bytecode),
    };
    let split = constructor_args_offset(constructor, &bytecode)?;
    Ok(if without_args { bytecode.slice(..split) } else { bytecode.slice(split..) })
}

/// Loads the ABI of `contract` from `abi_path`, or from Etherscan when no path is given.
pub(crate) async fn load_abi(
    contract: Address,
    config: &Config,
    abi_path: Option<&str>,
) -> Result<JsonAbi> {
    if let Some(path) = abi_path {
        return load_abi_from_file(path);
    }
    let abis = fetch_abi_from_etherscan(contract, config).await?;
    abis.into_iter().next().map(|(abi, _)| abi).ok_or_eyre("No ABI found.")
}

/// Returns the constructor of `abi`, failing if there is none or it takes no arguments.
pub(crate) fn constructor_with_args(abi: &JsonAbi) -> Result<&Constructor> {
    let constructor = abi.constructor().ok_or_else(|| eyre!("No constructor found."))?;
    if constructor.inputs.is_empty() {
        eyre::bail!("No constructor arguments found.");
    }
    Ok(constructor)
}

/// Returns the offset in `bytecode` at which the ABI-encoded constructor arguments start.
///
/// The arguments are appended to the init code, so their encoding is a word-aligned suffix of the
/// creation bytecode. Static arguments occupy a fixed number of words, but dynamic ones carry
/// their own lengths, so the suffix is searched from the shortest candidate upwards for the first
/// one that decodes as the constructor inputs and encodes back to the same bytes.
pub(crate) fn constructor_args_offset(constructor: &Constructor, bytecode: &[u8]) -> Result<usize> {
    let types =
        constructor.inputs.iter().map(|input| input.resolve()).collect::<Result<Vec<_>, _>>()?;
    let min_words = types.iter().map(DynSolType::minimum_words).sum::<usize>();
    let max_offset = bytecode.len().checked_sub(min_words * 32).ok_or_else(|| {
        eyre!(
            "Invalid creation bytecode length: have {} bytes, need at least {} for {} constructor inputs",
            bytecode.len(),
            min_words * 32,
            constructor.inputs.len()
        )
    })?;
    if !types.iter().any(DynSolType::is_dynamic) {
        return Ok(max_offset);
    }

    let tuple = DynSolType::Tuple(types);
    (min_words..=bytecode.len() / 32)
        .map(|words| bytecode.len() - words * 32)
        .find(|&offset| {
            let args = &bytecode[offset..];
            tuple.abi_decode_params(args).is_ok_and(|value| value.abi_encode_params() == args)
        })
        .ok_or_else(|| {
            eyre!("Could not find constructor arguments matching the ABI in the creation bytecode")
        })
}

/// Connects to the configured RPC, pins `config.chain` to it, and fetches the creation code of
/// `contract` using its Etherscan creation transaction.
pub(crate) async fn fetch_creation_code(config: &mut Config, contract: Address) -> Result<Bytes> {
    let provider = utils::get_provider(config)?;
    let chain = provider.get_chain_id().await?.into();
    config.chain = Some(chain);

    let client = config
        .get_etherscan_config_with_chain(Some(chain))?
        .ok_or_else(|| eyre!("No Etherscan API key configured for chain {chain}"))?
        .into_client_with_no_proxy(config.eth_rpc_no_proxy)?;
    let creation_tx_hash = client.contract_creation_data(contract).await?.transaction_hash;
    let tx_data = provider
        .get_transaction_by_hash(creation_tx_hash)
        .await?
        .ok_or_eyre("Could not find creation tx data.")?;

    if tx_data.to().is_none() {
        // Contract was created using a standard transaction.
        return Ok(tx_data.input().clone());
    }

    // Contract was created using a factory pattern or create2: extract the init code from the
    // creation trace.
    let traces = provider
        .trace_transaction(creation_tx_hash)
        .await
        .map_err(|e| eyre!("Could not fetch traces for transaction {}: {}", creation_tx_hash, e))?;
    traces
        .into_iter()
        .filter(|trace| {
            matches!(&trace.trace.result, Some(TraceOutput::Create(CreateOutput { address, .. })) if *address == contract)
        })
        .filter_map(|trace| match trace.trace.action {
            Action::Create(CreateAction { init, .. }) => Some(init),
            _ => None,
        })
        .last()
        .ok_or_else(|| eyre!("Could not find contract creation trace."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_dyn_abi::DynSolValue;
    use alloy_primitives::U256;
    use std::io::Write;

    fn constructor(inputs: &str) -> Constructor {
        let abi: JsonAbi =
            serde_json::from_str(&format!(r#"[{{"type":"constructor","inputs":[{inputs}]}}]"#))
                .unwrap();
        abi.constructor().unwrap().clone()
    }

    #[test]
    fn splits_static_constructor_args() {
        let constructor = constructor(
            r#"{"name":"owner","type":"address"},{"name":"limits","type":"uint256[2]"}"#,
        );
        let args = DynSolValue::Tuple(vec![
            DynSolValue::Address(Address::repeat_byte(0x11)),
            DynSolValue::FixedArray(vec![
                DynSolValue::Uint(U256::from(1), 256),
                DynSolValue::Uint(U256::from(2), 256),
            ]),
        ])
        .abi_encode_params();
        let init_code = vec![0xfe; 77];
        let bytecode = [init_code.as_slice(), args.as_slice()].concat();

        assert_eq!(constructor_args_offset(&constructor, &bytecode).unwrap(), init_code.len());
    }

    #[test]
    fn splits_dynamic_constructor_args() {
        let constructor = constructor(
            r#"{"name":"name","type":"string"},{"name":"supply","type":"uint256"},{"name":"admins","type":"address[]"}"#,
        );
        let args = DynSolValue::Tuple(vec![
            DynSolValue::String("Creation code with a name longer than one word".into()),
            DynSolValue::Uint(U256::from(42), 256),
            DynSolValue::Array(vec![
                DynSolValue::Address(Address::repeat_byte(0x11)),
                DynSolValue::Address(Address::repeat_byte(0x22)),
            ]),
        ])
        .abi_encode_params();
        let init_code = vec![0xfe; 77];
        let bytecode = [init_code.as_slice(), args.as_slice()].concat();

        let offset = constructor_args_offset(&constructor, &bytecode).unwrap();
        assert_eq!(offset, init_code.len());
        assert_eq!(&bytecode[offset..], args.as_slice());
    }

    #[test]
    fn rejects_creation_code_without_matching_args() {
        let constructor = constructor(r#"{"name":"name","type":"string"}"#);
        let bytecode = vec![0xfe; 100];

        let err = constructor_args_offset(&constructor, &bytecode).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Could not find constructor arguments matching the ABI in the creation bytecode"
        );
    }

    #[tokio::test]
    async fn rejects_creation_code_shorter_than_constructor_head() {
        let mut abi = tempfile::NamedTempFile::new().unwrap();
        write!(
            abi,
            r#"{{"abi":[{{"type":"constructor","inputs":[{{"name":"value","type":"uint256"}}]}}]}}"#
        )
        .unwrap();

        for (without_args, only_args) in [(true, false), (false, true)] {
            let err = parse_code_output(
                Bytes::from(vec![0; 31]),
                Address::ZERO,
                &Config::default(),
                abi.path().to_str(),
                without_args,
                only_args,
            )
            .await
            .unwrap_err();

            assert_eq!(
                err.to_string(),
                "Invalid creation bytecode length: have 31 bytes, need at least 32 for 1 constructor inputs"
            );
        }
    }
}
