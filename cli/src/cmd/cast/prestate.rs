use crate::{cmd::Cmd, init_progress, update_progress, utils::try_consume_config_rpc_url};
use cast::{revm::db::AccountState, HashMap};
use clap::Parser;
use ethers::{
    abi::Address,
    prelude::{Middleware, U256, U64},
    solc::utils::RuntimeOrHandle,
    types::{Transaction, H256},
};
use eyre::WrapErr;
use forge::executor::{
    inspector::cheatcodes::util::configure_tx_env, opts::EvmOpts, Backend, ExecutorBuilder,
};
use foundry_common::try_get_http_provider;
use foundry_config::{find_project_root_path, Config};
use serde::{Deserialize, Serialize, Serializer};
use std::{fs::File, str::FromStr};
use tracing::trace;

/// CLI arguments for `cast prestate`.
#[derive(Debug, Clone, Parser)]
pub struct PrestateArgs {
    #[clap(help = "The transaction hash.", value_name = "TXHASH")]
    tx_hash: String,
    #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
    rpc_url: Option<String>,
    #[clap(
        long,
        help = "Whether to include parent block related fields in the dumped environment. Requires an extra request."
    )]
    include_parent: bool,
}

impl Cmd for PrestateArgs {
    type Output = ();
    /// Gathers the state necessary to execute the given transaction.
    ///
    /// Executes all transactions up until and including the given transaction and dumps the state
    /// of the accounts used to execute the transaction onto disk.
    ///
    /// The dump will contain all accounts and their state *prior* to executing the target
    /// transaction.
    ///
    /// Cheatcodes are disabled.
    fn run(self) -> eyre::Result<Self::Output> {
        RuntimeOrHandle::new().block_on(self.run_tx())
    }
}

impl PrestateArgs {
    async fn run_tx(self) -> eyre::Result<()> {
        let figment = Config::figment_with_root(find_project_root_path().unwrap());
        let mut evm_opts = figment.extract::<EvmOpts>()?;
        let config = Config::from_provider(figment).sanitized();

        let rpc_url = try_consume_config_rpc_url(self.rpc_url)?;
        let provider = try_get_http_provider(&rpc_url)?;

        let tx_hash = H256::from_str(&self.tx_hash).wrap_err("invalid tx hash")?;
        let tx = provider
            .get_transaction(tx_hash)
            .await?
            .ok_or_else(|| eyre::eyre!("tx not found: {:?}", tx_hash))?;

        let tx_block_number = tx
            .block_number
            .ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?
            .as_u64();
        evm_opts.fork_url = Some(rpc_url);
        // we need to set the fork block to the previous block, because that's the state at
        // which we access the data in order to execute the transaction(s)
        evm_opts.fork_block_number = Some(tx_block_number - 1);

        // Set up the execution environment
        let env = evm_opts.evm_env().await;
        let db = Backend::spawn(evm_opts.get_fork(&config, env.clone()));

        // Configures a bare version of the evm executor (no cheatcodes, no tracing).
        let mut executor = ExecutorBuilder::default()
            .with_config(env)
            .with_spec(crate::utils::evm_spec(&config.evm_version))
            .build(db);

        // Fetch the block
        let block = provider
            .get_block_with_txs(tx_block_number)
            .await?
            .ok_or_else(|| eyre::eyre!("block not found"))?;
        let mut parent_block = None;
        if self.include_parent {
            parent_block = Some(
                provider
                    .get_block(tx_block_number - 1)
                    .await?
                    .ok_or_else(|| eyre::eyre!("parent block not found"))?,
            );
        }

        let prestate_env = PrestateEnv {
            current_coinbase: block.author.ok_or_else(|| eyre::eyre!("block has no author"))?,
            current_difficulty: Some(block.difficulty),
            current_random: None,
            current_base_fee: block.base_fee_per_gas,
            parent_difficulty: parent_block.as_ref().map(|parent| parent.difficulty),
            parent_gas_used: parent_block.as_ref().map(|parent| parent.gas_used),
            parent_gas_limit: parent_block.as_ref().map(|parent| parent.gas_limit),
            parent_timestamp: parent_block.as_ref().map(|parent| parent.timestamp),
            current_number: U64::from(tx_block_number),
            current_timestamp: block.timestamp,
            current_gas_limit: block.gas_limit,
            blockhashes: vec![],
        };

        // Configure the executor environment
        let mut env = executor.env().clone();
        env.block.number = tx_block_number.into();
        env.block.timestamp = block.timestamp;
        env.block.coinbase = block.author.unwrap_or_default();
        env.block.difficulty = block.difficulty;
        env.block.prevrandao = block.mix_hash;
        env.block.basefee = block.base_fee_per_gas.unwrap_or_default();
        env.block.gas_limit = block.gas_limit;

        // Execute prior transactions
        println!("Executing previous transactions from the block.");
        let pb = init_progress!(block.transactions, "tx");
        for (index, tx) in block.transactions.into_iter().enumerate() {
            if tx.hash().eq(&tx_hash) {
                break
            }

            configure_tx_env(&mut env, &tx);

            if let Some(to) = tx.to {
                trace!(tx=?tx.hash,?to, "executing previous call transaction");
                executor
                    .commit_tx_with_env(env.clone())
                    .wrap_err_with(|| format!("Failed to execute transaction: {:?}", tx.hash()))?;
            } else {
                trace!(tx=?tx.hash, "executing previous create transaction");
                executor.deploy_with_env(env.clone(), None).wrap_err_with(|| {
                    format!("Failed to execute create transaction: {:?}", tx.hash())
                })?;
            }

            update_progress!(pb, index);
        }

        // Execute the target transaction
        println!("Executing target transaction.");
        configure_tx_env(&mut env, &tx);
        let _ = executor.call_raw_with_env(env).unwrap().state_changeset.unwrap();

        // Find prestate accounts
        let all_accounts = &executor.backend().active_fork_db().unwrap().accounts;
        println!("Found {} accounts in database, filtering...", all_accounts.len());

        let accounts: HashMap<Address, PrestateAccount> = all_accounts
            .iter()
            .filter(|(_, account)| !matches!(account.account_state, AccountState::NotExisting))
            .map(|(address, account)| {
                let code = account
                    .info
                    .code
                    .as_ref()
                    .filter(|code| !code.is_empty())
                    .map(|code| ethers::core::types::Bytes(code.bytes().clone()));
                (
                    *address,
                    PrestateAccount {
                        balance: account.info.balance,
                        nonce: account.info.nonce.into(),
                        storage: account.storage.clone(),
                        code,
                    },
                )
            })
            .collect();

        println!("Saving {} accounts as prestate.", accounts.len());
        let prestate = File::create("alloc.json").expect("Unable to create file");
        serde_json::to_writer_pretty(prestate, &accounts).expect("could not write to file");
        println!("Wrote account states to alloc.json.");

        let env = File::create("env.json").expect("Unable to create file");
        serde_json::to_writer_pretty(env, &prestate_env).expect("Could not write to file");
        println!("Wrote environment info to env.json.");

        let tx_file = File::create("txs.json").expect("Unable to create file");
        serde_json::to_writer_pretty(tx_file, &Tx([tx])).expect("Could not write to file");
        println!("Wrote transaction description to txs.json.");

        Ok(())
    }
}

/// The state of an account prior to execution of the target transaction.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrestateAccount {
    /// The balance of the account
    pub balance: U256,
    /// The nonce of the account
    pub nonce: U64,
    /// The storage slots of the account.
    ///
    /// Note: This only includes the storage slots that were read or written to during execution of
    /// the transactions.
    #[serde(serialize_with = "geth_alloc_compat")]
    pub storage: HashMap<U256, U256>,
    /// The bytecode of the account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<ethers::core::types::Bytes>,
    // TODO: SecretKey?
}

/// The environment *prior* to executing the target transaction.
///
/// This is intended to be compatible with the type definitions in [geth], however this is lacking
/// ommers and withdrawals.
///
/// [geth]: https://github.com/ethereum/go-ethereum/tree/master/cmd/evm
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PrestateEnv {
    /// The author of the block (required).
    current_coinbase: Address,
    /// The gas limit of the block (required).
    current_gas_limit: U256,
    /// The block number (required).
    current_number: U64,
    /// The block timestamp (required).
    current_timestamp: U256,
    // TODO: withdrawals: []withdrawal
    /// The block difficulty.
    #[serde(skip_serializing_if = "Option::is_none")]
    current_difficulty: Option<U256>,
    /// The current RANDAO.
    #[serde(skip_serializing_if = "Option::is_none")]
    current_random: Option<U256>,
    /// The current base fee.
    #[serde(skip_serializing_if = "Option::is_none")]
    current_base_fee: Option<U256>,
    /// The difficulty of the parent block.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_difficulty: Option<U256>,
    /// The gas used in the parent block.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_gas_used: Option<U256>,
    /// The gas limit of the parent block.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_gas_limit: Option<U256>,
    /// The timestamp of the parent block.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_timestamp: Option<U256>,
    /// Blockhashes for the `BLOCKHASH` opcode.
    blockhashes: Vec<H256>,
}

/// A description of the target transaction.
///
/// This is intended to be compatible with the type definitions in [geth], however this is lacking
/// `SecretKey`.
///
/// [geth]: https://github.com/ethereum/go-ethereum/tree/master/cmd/evm
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tx([Transaction; 1]);

fn geth_alloc_compat<S>(value: &HashMap<U256, U256>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_map(
        value.iter().map(|(k, v)| (format!("0x{:0>64x}", k), format!("0x{:0>64x}", v))),
    )
}
