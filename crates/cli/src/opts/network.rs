use alloy_chains::Chain;
use alloy_primitives::ChainId;

/// Network selection, defaulting to Ethereum
#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum NetworkVariant {
    /// Ethereum (default)
    #[default]
    Ethereum,
    /// Optimism / OP-stack
    Optimism,
    /// Tempo
    Tempo,
}

impl From<ChainId> for NetworkVariant {
    fn from(chain_id: ChainId) -> Self {
        let chain = Chain::from_id(chain_id);
        if chain.is_tempo() {
            Self::Tempo
        } else if chain.is_optimism() {
            Self::Optimism
        } else {
            Default::default()
        }
    }
}
