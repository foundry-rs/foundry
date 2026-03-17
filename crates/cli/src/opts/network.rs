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
