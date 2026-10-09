//! Typed Tempo configuration for the public anvil node config.

use alloy_primitives::Address;
use eyre::Result;
use foundry_evm_hardforks::{FoundryHardfork, TempoHardfork, latest_active_tempo_hardfork};
use foundry_evm_networks::NetworkConfigs;
use reth_anvil::NodeConfig;

/// Settings owned by the Tempo extension.
#[derive(Clone, Debug, Default)]
pub struct TempoConfig {
    /// Fee payer for sponsorship requests; the last dev account when omitted.
    pub fee_payer: Option<Address>,
}

/// Tempo builders and hardfork resolution on the shared node config.
pub trait TempoConfigExt: Sized {
    /// Returns a silent Tempo test node on a free port.
    fn test_tempo() -> Self;
    /// Selects Tempo as the node's network.
    fn with_tempo(self) -> Self;
    /// Sets the unlocked account that sponsors fee-payer requests.
    fn with_tempo_fee_payer(self, fee_payer: Option<Address>) -> Self;
    /// Resolves the configured fee payer, or the last dev account.
    fn tempo_fee_payer_address(&self) -> Option<Address>;
    /// Resolves the Tempo hardfork active at genesis.
    fn get_tempo_hardfork(&self) -> Result<TempoHardfork>;
    /// Resolves the Tempo hardfork active at the supplied timestamp.
    fn tempo_hardfork_at(&self, timestamp: u64) -> Result<TempoHardfork>;
}

impl TempoConfigExt for NodeConfig {
    fn test_tempo() -> Self {
        Self::test().with_tempo()
    }

    fn with_tempo(mut self) -> Self {
        self.networks = NetworkConfigs::with_tempo();
        self
    }

    fn with_tempo_fee_payer(mut self, fee_payer: Option<Address>) -> Self {
        self.extensions.insert(TempoConfig { fee_payer });
        self
    }

    fn tempo_fee_payer_address(&self) -> Option<Address> {
        if !self.networks.is_tempo() {
            return None;
        }
        self.extensions
            .get::<TempoConfig>()
            .and_then(|config| config.fee_payer)
            .or_else(|| self.genesis_accounts.last().map(|wallet| wallet.address()))
    }

    fn get_tempo_hardfork(&self) -> Result<TempoHardfork> {
        self.tempo_hardfork_at(self.get_genesis_timestamp())
    }

    fn tempo_hardfork_at(&self, timestamp: u64) -> Result<TempoHardfork> {
        match self.hardfork {
            None => Ok(TempoHardfork::from_chain_and_timestamp(self.get_chain_id(), timestamp)
                .unwrap_or_else(latest_active_tempo_hardfork)),
            Some(FoundryHardfork::Tempo(hardfork)) => Ok(hardfork),
            Some(hardfork) => eyre::bail!("hardfork {hardfork:?} is not a Tempo hardfork"),
        }
    }
}
