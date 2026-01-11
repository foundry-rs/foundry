//! Chain-specific utility functions for determining network characteristics.

use crate::{Chain, NamedChain};

/// Returns true if the network calculates gas costs differently.
///
/// Some networks have non-standard gas calculation mechanisms that require special handling
/// during transaction simulation and broadcasting. This includes networks like Arbitrum,
/// Mantle, Moonbeam, and others with custom gas models.
pub fn has_different_gas_calc(chain_id: u64) -> bool {
    if let Some(chain) = Chain::from(chain_id).named() {
        return chain.is_arbitrum()
            || chain.is_elastic()
            || matches!(
                chain,
                NamedChain::Acala
                    | NamedChain::AcalaMandalaTestnet
                    | NamedChain::AcalaTestnet
                    | NamedChain::Etherlink
                    | NamedChain::EtherlinkTestnet
                    | NamedChain::Karura
                    | NamedChain::KaruraTestnet
                    | NamedChain::Mantle
                    | NamedChain::MantleSepolia
                    | NamedChain::Monad
                    | NamedChain::MonadTestnet
                    | NamedChain::Moonbase
                    | NamedChain::Moonbeam
                    | NamedChain::MoonbeamDev
                    | NamedChain::Moonriver
                    | NamedChain::Metis
            );
    }
    false
}

/// Returns true if the network supports broadcasting transactions in batches.
///
/// Some networks (like Arbitrum) do not support batched transaction broadcasting and require
/// sequential transaction submission. This function identifies such networks.
pub fn has_batch_support(chain_id: u64) -> bool {
    if let Some(chain) = Chain::from(chain_id).named() {
        return !chain.is_arbitrum();
    }
    true
}

