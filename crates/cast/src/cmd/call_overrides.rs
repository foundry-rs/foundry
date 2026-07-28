//! Shared `eth_call` state and block override options.

use alloy_contract::{CallDecoder, EthCall};
use alloy_network::Network;
use alloy_primitives::{Address, B256, Bytes, U256, map::HashMap};
use alloy_rpc_types::{
    BlockOverrides,
    state::{StateOverride, StateOverridesBuilder},
};
use clap::Args;
use eyre::Result;
use regex::Regex;
use std::{str::FromStr, sync::LazyLock};

// Matches override pattern <address>:<slot>:<value>.
// e.g. 0x123:0x1:0x1234.
static OVERRIDE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^:]+):([^:]+):([^:]+)$").unwrap());

/// State and block overrides for an `eth_call`.
#[derive(Args, Clone, Debug, Default)]
pub struct CallOverrideOpts {
    /// Override account balances.
    /// Format: "address:balance,address:balance".
    #[arg(long = "override-balance", value_name = "ADDRESS:BALANCE", value_delimiter = ',')]
    pub balance_overrides: Option<Vec<String>>,

    /// Override account nonces.
    /// Format: "address:nonce,address:nonce".
    #[arg(long = "override-nonce", value_name = "ADDRESS:NONCE", value_delimiter = ',')]
    pub nonce_overrides: Option<Vec<String>>,

    /// Override account code.
    /// Format: "address:code,address:code".
    #[arg(long = "override-code", value_name = "ADDRESS:CODE", value_delimiter = ',')]
    pub code_overrides: Option<Vec<String>>,

    /// Override account state and replace the current state entirely with the new one.
    /// Format: "address:slot:value,address:slot:value".
    #[arg(long = "override-state", value_name = "ADDRESS:SLOT:VALUE", value_delimiter = ',')]
    pub state_overrides: Option<Vec<String>>,

    /// Override specific account storage slots and preserve the rest of the state.
    /// Format: "address:slot:value,address:slot:value".
    #[arg(long = "override-state-diff", value_name = "ADDRESS:SLOT:VALUE", value_delimiter = ',')]
    pub state_diff_overrides: Option<Vec<String>>,

    /// Override the block timestamp.
    #[arg(long = "block.time", value_name = "TIME")]
    pub block_time: Option<u64>,

    /// Override the block number.
    #[arg(long = "block.number", value_name = "NUMBER")]
    pub block_number: Option<u64>,
}

impl CallOverrideOpts {
    /// Returns true when no state or block override was provided.
    pub const fn is_empty(&self) -> bool {
        self.balance_overrides.is_none()
            && self.nonce_overrides.is_none()
            && self.code_overrides.is_none()
            && self.state_overrides.is_none()
            && self.state_diff_overrides.is_none()
            && self.block_time.is_none()
            && self.block_number.is_none()
    }

    /// Applies the configured overrides to an `eth_call`.
    pub fn apply<'a, D, N>(&self, mut call: EthCall<'a, D, N>) -> Result<EthCall<'a, D, N>>
    where
        D: CallDecoder,
        N: Network,
    {
        if let Some(state_overrides) = self.get_state_overrides()? {
            call = call.overrides(state_overrides);
        }
        if let Some(block_overrides) = self.get_block_overrides()? {
            call = call.with_block_overrides(block_overrides);
        }
        Ok(call)
    }

    /// Parses state overrides from command line arguments.
    pub fn get_state_overrides(&self) -> Result<Option<StateOverride>> {
        // Early return if no override set - <https://github.com/foundry-rs/foundry/issues/10705>.
        if [
            self.balance_overrides.as_ref(),
            self.nonce_overrides.as_ref(),
            self.code_overrides.as_ref(),
            self.state_overrides.as_ref(),
            self.state_diff_overrides.as_ref(),
        ]
        .iter()
        .all(Option::is_none)
        {
            return Ok(None);
        }

        let mut state_overrides_builder = StateOverridesBuilder::default();

        for override_str in self.balance_overrides.iter().flatten() {
            let (addr, balance) = address_value_override(override_str)?;
            state_overrides_builder =
                state_overrides_builder.with_balance(addr.parse()?, balance.parse()?);
        }

        for override_str in self.nonce_overrides.iter().flatten() {
            let (addr, nonce) = address_value_override(override_str)?;
            state_overrides_builder =
                state_overrides_builder.with_nonce(addr.parse()?, nonce.parse()?);
        }

        for override_str in self.code_overrides.iter().flatten() {
            let (addr, code_str) = address_value_override(override_str)?;
            state_overrides_builder =
                state_overrides_builder.with_code(addr.parse()?, Bytes::from_str(code_str)?);
        }

        type StateOverrides = HashMap<Address, HashMap<B256, B256>>;
        let parse_state_overrides = |overrides: &Option<Vec<String>>| -> Result<StateOverrides> {
            let mut state_overrides = StateOverrides::default();

            overrides.iter().flatten().try_for_each(|s| -> Result<()> {
                let (addr, slot, value) = address_slot_value_override(s)?;
                state_overrides.entry(addr).or_default().insert(slot.into(), value.into());
                Ok(())
            })?;

            Ok(state_overrides)
        };

        for (addr, entries) in parse_state_overrides(&self.state_overrides)? {
            state_overrides_builder = state_overrides_builder.with_state(addr, entries);
        }

        for (addr, entries) in parse_state_overrides(&self.state_diff_overrides)? {
            state_overrides_builder = state_overrides_builder.with_state_diff(addr, entries)
        }

        Ok(Some(state_overrides_builder.build()))
    }

    /// Parses block overrides from command line arguments.
    pub fn get_block_overrides(&self) -> Result<Option<BlockOverrides>> {
        let mut overrides = BlockOverrides::default();
        if let Some(number) = self.block_number {
            overrides = overrides.with_number(U256::from(number));
        }
        if let Some(time) = self.block_time {
            overrides = overrides.with_time(time);
        }
        if overrides.is_empty() { Ok(None) } else { Ok(Some(overrides)) }
    }
}

/// Parses an override string in the format address:value.
fn address_value_override(address_override: &str) -> Result<(&str, &str)> {
    address_override.split_once(':').ok_or_else(|| {
        eyre::eyre!("Invalid override {address_override}. Expected <address>:<value>")
    })
}

/// Parses an override string in the format address:slot:value.
fn address_slot_value_override(address_override: &str) -> Result<(Address, U256, U256)> {
    let captures = OVERRIDE_PATTERN.captures(address_override).ok_or_else(|| {
        eyre::eyre!("Invalid override {address_override}. Expected <address>:<slot>:<value>")
    })?;

    Ok((captures[1].parse()?, captures[2].parse()?, captures[3].parse()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, b256, fixed_bytes};
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct TestArgs {
        #[command(flatten)]
        overrides: CallOverrideOpts,
    }

    #[test]
    fn test_get_state_overrides() {
        let args = TestArgs::parse_from([
            "foundry-cli",
            "--override-balance",
            "0x0000000000000000000000000000000000000001:2",
            "--override-nonce",
            "0x0000000000000000000000000000000000000001:3",
            "--override-code",
            "0x0000000000000000000000000000000000000001:0x04",
            "--override-state",
            "0x0000000000000000000000000000000000000001:5:6",
            "--override-state-diff",
            "0x0000000000000000000000000000000000000001:7:8",
        ]);
        let overrides = args.overrides.get_state_overrides().unwrap().unwrap();
        let address = address!("0x0000000000000000000000000000000000000001");
        let account = overrides.get(&address).unwrap();

        assert_eq!(account.balance, Some(U256::from(2)));
        assert_eq!(account.nonce, Some(3));
        assert_eq!(account.code, Some(Bytes::from([0x04])));
        assert_eq!(
            account
                .state
                .as_ref()
                .unwrap()
                .get(&b256!("0x0000000000000000000000000000000000000000000000000000000000000005")),
            Some(&b256!("0x0000000000000000000000000000000000000000000000000000000000000006"))
        );
        assert_eq!(
            account
                .state_diff
                .as_ref()
                .unwrap()
                .get(&b256!("0x0000000000000000000000000000000000000000000000000000000000000007")),
            Some(&b256!("0x0000000000000000000000000000000000000000000000000000000000000008"))
        );
    }

    #[test]
    fn test_get_state_overrides_empty() {
        let args = TestArgs::parse_from([""]);
        assert_eq!(args.overrides.get_state_overrides().unwrap(), None);
    }

    #[test]
    fn test_get_block_overrides() {
        let args =
            TestArgs::parse_from(["foundry-cli", "--block.number", "1", "--block.time", "2"]);
        let overrides = args.overrides.get_block_overrides().unwrap().unwrap();
        assert_eq!(overrides.number, Some(U256::from(1)));
        assert_eq!(overrides.time, Some(2));
    }

    #[test]
    fn test_get_block_overrides_empty() {
        let args = TestArgs::parse_from([""]);
        assert_eq!(args.overrides.get_block_overrides().unwrap(), None);
    }

    #[test]
    fn test_address_value_override_success() {
        let text = "0x0000000000000000000000000000000000000001:2";
        let (address, value) = address_value_override(text).unwrap();
        assert_eq!(address, "0x0000000000000000000000000000000000000001");
        assert_eq!(value, "2");
    }

    #[test]
    fn test_address_value_override_error() {
        let text = "invalid_value";
        let error = address_value_override(text).unwrap_err();
        assert_eq!(error.to_string(), "Invalid override invalid_value. Expected <address>:<value>");
    }

    #[test]
    fn test_address_slot_value_override_success() {
        let text = "0x0000000000000000000000000000000000000001:2:3";
        let (address, slot, value) = address_slot_value_override(text).unwrap();
        assert_eq!(*address, fixed_bytes!("0x0000000000000000000000000000000000000001"));
        assert_eq!(slot, U256::from(2));
        assert_eq!(value, U256::from(3));
    }

    #[test]
    fn test_address_slot_value_override_error() {
        let text = "invalid_value";
        let error = address_slot_value_override(text).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Invalid override invalid_value. Expected <address>:<slot>:<value>"
        );
    }
}
