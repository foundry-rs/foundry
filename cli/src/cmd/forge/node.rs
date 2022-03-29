use clap::Parser;
use ethers::{
    core::{
        k256::ecdsa::SigningKey,
        types::{Address, U256},
    },
    prelude::{coins_bip39::English, MnemonicBuilder, Wallet},
    solc::EvmVersion,
};
use once_cell::sync::OnceCell;

use crate::opts::evm::EvmArgs;
use std::{collections::BTreeMap, str::FromStr};

use super::Cmd;

#[derive(Clone, Debug, Parser)]
pub struct NodeArgs {
    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    evm_opts: EvmArgs,

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
