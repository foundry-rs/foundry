use alloy_chains::NamedChain;
use alloy_network::Network;
use std::time::Duration;

use crate::{Provider, ProviderLayer};

/// A layer that wraps a [`NamedChain`]. The layer will be used to set
/// the client's poll interval based on the average block time for this chain.
///
/// Does nothing to the client with a local transport.
#[derive(Debug, Clone, Copy)]
pub struct ChainLayer(NamedChain);

impl ChainLayer {
    /// Get the chain's average blocktime, if applicable.
    pub const fn average_blocktime_hint(&self) -> Option<Duration> {
        self.0.average_blocktime_hint()
    }
}

impl From<NamedChain> for ChainLayer {
    fn from(chain: NamedChain) -> Self {
        Self(chain)
    }
}

impl<P, N> ProviderLayer<P, N> for ChainLayer
where
    P: Provider<N>,
    N: Network,
{
    type Provider = P;

    fn layer(&self, inner: P) -> Self::Provider {
        if !inner.client().is_local() {
            if let Some(avg_block_time) = self.average_blocktime_hint() {
                let poll_interval = avg_block_time.mul_f32(0.6);
                inner.client().set_poll_interval(poll_interval);
            }
        }
        inner
    }
}
