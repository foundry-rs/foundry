use super::{build::BuildArgs, script::ScriptArgs};
use crate::cmd::retry::RetryArgs;
use clap::{Parser, ValueHint};
use ethers::types::U256;
use foundry_cli::{opts::MultiWallet, utils::parse_ether_value};
use foundry_common::evm::EvmArgs;

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(DebugArgs, opts, evm_opts);

/// CLI arguments for `forge debug`.
#[derive(Debug, Clone, Parser)]
pub struct DebugArgs {
    /// The contract you want to run. Either the file path or contract name.
    ///
    /// If multiple contracts exist in the same file you must specify the target contract with
    /// --target-contract.
    #[clap(value_hint = ValueHint::FilePath)]
    pub path: String,

    /// Arguments to pass to the script function.
    pub args: Vec<String>,

    /// The name of the contract you want to run.
    #[clap(long, visible_alias = "tc", value_name = "CONTRACT_NAME")]
    pub target_contract: Option<String>,

    /// The signature of the function you want to call in the contract, or raw calldata.
    #[clap(
        long,
        short,
        default_value = "run()",
        value_parser = foundry_common::clap_helpers::strip_0x_prefix
    )]
    pub sig: String,

    /// Max priority fee per gas for EIP1559 transactions.
    #[clap(
        long,
        env = "ETH_PRIORITY_GAS_PRICE",
        value_parser = parse_ether_value,
        value_name = "PRICE"
    )]
    pub priority_gas_price: Option<U256>,

    /// Use legacy transactions instead of EIP1559 ones.
    ///
    /// This is auto-enabled for common networks without EIP1559.
    #[clap(long)]
    pub legacy: bool,

    /// Broadcasts the transactions.
    #[clap(long)]
    pub broadcast: bool,

    /// Skips on-chain simulation.
    #[clap(long)]
    pub skip_simulation: bool,

    /// Relative percentage to multiply gas estimates by.
    #[clap(long, short, default_value = "130")]
    pub gas_estimate_multiplier: u64,

    /// Send via `eth_sendTransaction` using the `--from` argument or `$ETH_FROM` as sender
    #[clap(
        long,
        requires = "sender",
        conflicts_with_all = &["private_key", "private_keys", "froms", "ledger", "trezor", "aws"],
    )]
    pub unlocked: bool,

    /// Resumes submitting transactions that failed or timed-out previously.
    ///
    /// It DOES NOT simulate the script again and it expects nonces to have remained the same.
    ///
    /// Example: If transaction N has a nonce of 22, then the account should have a nonce of 22,
    /// otherwise it fails.
    #[clap(long)]
    pub resume: bool,

    /// If present, --resume or --verify will be assumed to be a multi chain deployment.
    #[clap(long)]
    pub multi: bool,

    /// Makes sure a transaction is sent,
    /// only after its previous one has been confirmed and succeeded.
    #[clap(long)]
    pub slow: bool,

    /// Disables interactive prompts that might appear when deploying big contracts.
    ///
    /// For more info on the contract size limit, see EIP-170: <https://eips.ethereum.org/EIPS/eip-170>
    #[clap(long)]
    pub non_interactive: bool,

    /// The Etherscan (or equivalent) API key
    #[clap(long, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
    pub etherscan_api_key: Option<String>,

    /// Verifies all the contracts found in the receipts of a script, if any.
    #[clap(long)]
    pub verify: bool,

    /// Output results in JSON format.
    #[clap(long)]
    pub json: bool,

    /// Gas price for legacy transactions, or max fee per gas for EIP1559 transactions.
    #[clap(
        long,
        env = "ETH_GAS_PRICE",
        value_parser = parse_ether_value,
        value_name = "PRICE",
    )]
    pub with_gas_price: Option<U256>,

    #[clap(flatten)]
    pub opts: BuildArgs,

    #[clap(flatten)]
    pub wallets: MultiWallet,

    #[clap(flatten)]
    pub evm_opts: EvmArgs,

    #[clap(flatten)]
    pub verifier: super::verify::VerifierArgs,

    #[clap(flatten)]
    pub retry: RetryArgs,
}

impl DebugArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let script = ScriptArgs {
            path: self.path,
            args: self.args,
            target_contract: self.target_contract,
            sig: self.sig,
            priority_gas_price: self.priority_gas_price,
            legacy: self.legacy,
            broadcast: self.broadcast,
            skip_simulation: self.skip_simulation,
            gas_estimate_multiplier: self.gas_estimate_multiplier,
            unlocked: self.unlocked,
            resume: self.resume,
            multi: self.multi,
            debug: true,
            slow: self.slow,
            non_interactive: self.non_interactive,
            etherscan_api_key: self.etherscan_api_key,
            verify: self.verify,
            json: self.json,
            with_gas_price: self.with_gas_price,
            opts: self.opts,
            wallets: self.wallets,
            evm_opts: self.evm_opts,
            verifier: self.verifier,
            retry: self.retry,
        };
        script.run_script().await
    }
}
