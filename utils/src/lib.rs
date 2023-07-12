#![doc = include_str!("../README.md")]

use ethers_addressbook::contract;
use ethers_core::types::*;
use ethers_providers::{Middleware, Provider};
use eyre::{Result, WrapErr};
use futures::future::BoxFuture;
use std::{env::VarError, fmt::Write, time::Duration};

pub mod abi;
pub mod error;
pub mod glob;
pub mod linker;
pub mod rpc;

/// Given a k/v serde object, it pretty prints its keys and values as a table.
pub fn to_table(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Object(map) => {
            let mut s = String::new();
            for (k, v) in map.iter() {
                writeln!(&mut s, "{k: <20} {v}\n").expect("could not write k/v to table");
            }
            s
        }
        _ => "".to_owned(),
    }
}

/// Resolves an input to [`NameOrAddress`]. The input could also be a contract/token name supported
/// by
/// [`ethers-addressbook`](https://github.com/gakonst/ethers-rs/tree/master/ethers-addressbook).
pub fn resolve_addr<T: Into<NameOrAddress>>(to: T, chain: Option<Chain>) -> Result<NameOrAddress> {
    Ok(match to.into() {
        NameOrAddress::Address(addr) => NameOrAddress::Address(addr),
        NameOrAddress::Name(contract_or_ens) => {
            if let Some(contract) = contract(&contract_or_ens) {
                let chain = chain
                    .ok_or_else(|| eyre::eyre!("resolving contract requires a known chain"))?;
                NameOrAddress::Address(contract.address(chain).ok_or_else(|| {
                    eyre::eyre!(
                        "contract: {} not found in addressbook for network: {}",
                        contract_or_ens,
                        chain
                    )
                })?)
            } else {
                NameOrAddress::Name(contract_or_ens)
            }
        }
    })
}

/// Reads the `ETHERSCAN_API_KEY` env variable
pub fn etherscan_api_key() -> eyre::Result<String> {
    std::env::var("ETHERSCAN_API_KEY").map_err(|err| match err {
        VarError::NotPresent => {
            eyre::eyre!(
                r#"
  You need an Etherscan Api Key to verify contracts.
  Create one at https://etherscan.io/myapikey
  Then export it with \`export ETHERSCAN_API_KEY=xxxxxxxx'"#
            )
        }
        VarError::NotUnicode(err) => {
            eyre::eyre!("Invalid `ETHERSCAN_API_KEY`: {:?}", err)
        }
    })
}

/// A type that keeps track of attempts
#[derive(Debug, Clone)]
pub struct Retry {
    retries: u32,
    delay: Option<u32>,
}

/// Sample retry logic implementation
impl Retry {
    pub fn new(retries: u32, delay: Option<u32>) -> Self {
        Self { retries, delay }
    }

    fn handle_err(&mut self, err: eyre::Report) {
        self.retries -= 1;
        tracing::warn!(
            "erroneous attempt ({} tries remaining): {}",
            self.retries,
            err.root_cause()
        );
        if let Some(delay) = self.delay {
            std::thread::sleep(Duration::from_secs(delay.into()));
        }
    }

    pub fn run<T, F>(mut self, mut callback: F) -> eyre::Result<T>
    where
        F: FnMut() -> eyre::Result<T>,
    {
        loop {
            match callback() {
                Err(e) if self.retries > 0 => self.handle_err(e),
                res => return res,
            }
        }
    }

    pub async fn run_async<'a, T, F>(mut self, mut callback: F) -> eyre::Result<T>
    where
        F: FnMut() -> BoxFuture<'a, eyre::Result<T>>,
    {
        loop {
            match callback().await {
                Err(e) if self.retries > 0 => self.handle_err(e),
                res => return res,
            };
        }
    }
}

pub async fn next_nonce(
    caller: Address,
    provider_url: &str,
    block: Option<BlockId>,
) -> Result<U256> {
    let provider = Provider::try_from(provider_url)
        .wrap_err_with(|| format!("Bad fork_url provider: {provider_url}"))?;
    Ok(provider.get_transaction_count(caller, block).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::types::Address;

    #[test]
    fn test_resolve_addr() {
        use std::str::FromStr;

        // DAI:mainnet exists in ethers-addressbook (0x6b175474e89094c44da98b954eedeac495271d0f)
        assert_eq!(
            resolve_addr(NameOrAddress::Name("dai".to_string()), Some(Chain::Mainnet)).ok(),
            Some(NameOrAddress::Address(
                Address::from_str("0x6b175474e89094c44da98b954eedeac495271d0f").unwrap()
            ))
        );

        // DAI:goerli exists in ethers-adddressbook (0x11fE4B6AE13d2a6055C8D9cF65c55bac32B5d844)
        assert_eq!(
            resolve_addr(NameOrAddress::Name("dai".to_string()), Some(Chain::Goerli)).ok(),
            Some(NameOrAddress::Address(
                Address::from_str("0x11fE4B6AE13d2a6055C8D9cF65c55bac32B5d844").unwrap()
            ))
        );

        // DAI:moonbean does not exist in addressbook
        assert!(
            resolve_addr(NameOrAddress::Name("dai".to_string()), Some(Chain::MoonbeamDev)).is_err()
        );

        // If not present in addressbook, gets resolved to an ENS name.
        assert_eq!(
            resolve_addr(
                NameOrAddress::Name("contractnotpresent".to_string()),
                Some(Chain::Mainnet)
            )
            .ok(),
            Some(NameOrAddress::Name("contractnotpresent".to_string())),
        );

        // Nothing to resolve for an address.
        assert_eq!(
            resolve_addr(NameOrAddress::Address(Address::zero()), Some(Chain::Mainnet)).ok(),
            Some(NameOrAddress::Address(Address::zero())),
        );
    }
}
