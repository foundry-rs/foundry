use serde::{Deserialize, Serialize};

use crate::binary_mappings::{BinaryMappings, BinaryName};

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkFamily {
    #[default]
    Ethereum,
    Zksync,
}

impl NetworkFamily {
    pub fn binary_mappings(self) -> BinaryMappings {
        match self {
            Self::Ethereum => BinaryMappings::default(),
            Self::Zksync => Self::zksync_mappings(),
        }
    }

    fn zksync_mappings() -> BinaryMappings {
        BinaryMappings::from([
            (BinaryName::Forge, "forge-zksync".into()),
            (BinaryName::Anvil, "anvil-zksync".into()),
            (BinaryName::Cast, "cast-zksync".into()),
            // Chisel is not currently supported on ZKsync.
        ])
    }
}
