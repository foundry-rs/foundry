use clap::Parser;
use ethers::{
    core::{
        k256::ecdsa::SigningKey,
        types::{Address, U256},
    },
    prelude::{coins_bip39::English, MnemonicBuilder, Wallet},
    solc::EvmVersion,
};
use evm_adapters::{
    evm_opts::EvmType,
    sputnik::{PrecompileFn, PRECOMPILES_MAP},
    FAUCET_ACCOUNT,
};
use forge_node::Node;
use once_cell::sync::OnceCell;

use std::{collections::BTreeMap, str::FromStr};

use super::Cmd;

use crate::opts::evm::EvmArgs;
#[cfg(feature = "evmodin-evm")]
use crate::utils::evmodin_cfg;
#[cfg(feature = "sputnik-evm")]
use crate::utils::sputnik_cfg;

#[cfg(feature = "sputnik-evm")]
static SPUTNIK_CONFIG: OnceCell<sputnik::Config> = OnceCell::new();
#[cfg(feature = "sputnik-evm")]
static SPUTNIK_VICINITY: OnceCell<sputnik::backend::MemoryVicinity> = OnceCell::new();
#[cfg(feature = "sputnik-evm")]
static SPUTNIK_BACKEND: OnceCell<sputnik::backend::MemoryBackend> = OnceCell::new();
#[cfg(feature = "sputnik-evm")]
static SPUTNIK_PRECOMPILES: OnceCell<BTreeMap<Address, PrecompileFn>> = OnceCell::new();

#[derive(Clone, Debug, Parser)]
pub struct NodeArgs {
    #[clap(flatten)]
    evm_opts: EvmArgs,

    #[clap(help = "choose the evm version", long, default_value = "london")]
    evm_version: EvmVersion,

    #[clap(
        long,
        help = "either a comma-separated hex-encoded list of private keys, or a mnemonic phrase",
        default_value = "20,test test test test test test test test test test test junk"
    )]
    accounts: SignerAccounts,

    #[clap(
        long,
        help = "the balance of every genesis account",
        default_value = "0xffffffffffffffffffffffff"
    )]
    balance: U256,
}

impl Cmd for NodeArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let node_config = forge_node::NodeConfig::new()
            .chain_id(self.evm_opts.env.chain_id.unwrap_or_default())
            .gas_limit(self.evm_opts.env.gas_limit.unwrap_or_default())
            .gas_price(self.evm_opts.env.gas_price.unwrap_or_default())
            .genesis_accounts(self.accounts.0)
            .genesis_balance(self.balance);

        match self.evm_opts.evm_type {
            #[cfg(feature = "sputnik-evm")]
            EvmType::Sputnik => {
                use evm_adapters::sputnik::Executor;
                use sputnik::backend::MemoryBackend;

                SPUTNIK_CONFIG
                    .set(sputnik_cfg(&self.evm_version))
                    .expect("could not set EVM_CONFIG");

                SPUTNIK_VICINITY
                    .set(self.evm_opts.env.sputnik_state())
                    .expect("could not set EVM_VICINITY");

                let mut backend = MemoryBackend::new(
                    SPUTNIK_VICINITY.get().expect("could not get EVM_VICINITY"),
                    Default::default(),
                );
                let faucet =
                    backend.state_mut().entry(*FAUCET_ACCOUNT).or_insert_with(Default::default);
                faucet.balance = U256::MAX;

                SPUTNIK_BACKEND.set(backend).expect("could not set EVM_BACKEND");

                SPUTNIK_PRECOMPILES
                    .set(PRECOMPILES_MAP.clone())
                    .unwrap_or_else(|_| panic!("could not set EVM_PRECOMPILES"));

                let evm = Executor::new_with_cheatcodes(
                    SPUTNIK_BACKEND.get().expect("could not get EVM_BACKEND"),
                    self.evm_opts.env.gas_limit.unwrap_or_default(),
                    SPUTNIK_CONFIG.get().expect("could not get EVM_CONFIG"),
                    SPUTNIK_PRECOMPILES.get().expect("could not get EVM_PRECOMPILES"),
                    false,
                    false,
                    false,
                );
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(Node::init_and_run(evm, node_config));
            }
            #[cfg(feature = "evmodin-evm")]
            EvmType::EvmOdin => {
                use evm_adapters::evmodin::EvmOdin;
                use evmodin::tracing::NoopTracer;

                let revision = evmodin_cfg(&self.evm_version);

                // TODO: Replace this with a proper host. We'll want this to also be
                // provided generically when we add the Forking host(s).
                let host = self.evm_opts.env.evmodin_state();

                let evm = EvmOdin::new(
                    host,
                    self.evm_opts.env.gas_limit.unwrap_or_default(),
                    revision,
                    NoopTracer,
                );
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(forge_node::Node::init_and_run(evm, node_config));
            }
        }
        Ok(())
    }
}

/// SignerAccounts are the signer accounts that will be initialised in the genesis block
#[derive(Clone, Debug)]
pub struct SignerAccounts(pub Vec<Wallet<SigningKey>>);

impl FromStr for SignerAccounts {
    type Err = std::io::Error;

    /// SignerAccounts can be initialised by passing in either a comma-separated list of
    /// hex-encoded private keys or a mnemonic phrase from the English wordset.
    ///
    /// Private Keys:
    /// --accounts="0000000000000000000000000000000000000000000000000000000000000001,
    /// 0000000000000000000000000000000000000000000000000000000000000002"
    ///
    /// Mnemonic Phrase:
    /// --accounts="25,fire evolve buddy tenant talent favorite ankle stem regret myth dream fresh"
    fn from_str(src: &str) -> Result<Self, Self::Err> {
        if src.contains(' ') {
            let parts: Vec<&str> = src.split(',').collect();
            let (num_accounts, mnemonic) = match parts.len() {
                1 => (20, parts[1]),
                2 => (
                    parts[0].parse().map_err(|_| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "error parsing number of accounts",
                        )
                    })?,
                    parts[1],
                ),
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "error parsing mnemonic",
                    ))
                }
            };
            let mut accounts = Vec::<Wallet<SigningKey>>::with_capacity(num_accounts);
            for i in 0..num_accounts {
                accounts.push(
                    MnemonicBuilder::<English>::default()
                        .phrase(mnemonic)
                        .index(i as u32)
                        .unwrap()
                        .build()
                        .unwrap(),
                );
            }
            Ok(SignerAccounts(accounts))
        } else {
            let pks: Vec<&str> = src.split(',').collect();
            let mut accounts = Vec::<Wallet<SigningKey>>::with_capacity(pks.len());
            for pk in pks.iter() {
                accounts.push(pk.parse().map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "error parsing private keys",
                    )
                })?);
            }
            Ok(SignerAccounts(accounts))
        }
    }
}
