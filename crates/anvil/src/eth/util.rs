use ethers::abi::Address;
use foundry_evm::revm::{self, precompile::Precompiles, primitives::SpecId};
use foundry_utils::types::ToEthers;
use std::fmt;

pub fn get_precompiles_for(spec_id: SpecId) -> Vec<Address> {
    Precompiles::new(to_precompile_id(spec_id))
        .addresses()
        .into_iter()
        .copied()
        .map(|item| item.to_ethers())
        .collect()
}

/// wrapper type that displays byte as hex
pub struct HexDisplay<'a>(&'a [u8]);

pub fn hex_fmt_many<I, T>(i: I) -> String
where
    I: IntoIterator<Item = T>,
    T: AsRef<[u8]>,
{
    i.into_iter()
        .map(|item| HexDisplay::from(item.as_ref()).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

impl<'a> HexDisplay<'a> {
    pub fn from(b: &'a [u8]) -> Self {
        HexDisplay(b)
    }
}

impl<'a> fmt::Display for HexDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.len() < 1027 {
            for byte in self.0 {
                f.write_fmt(format_args!("{byte:02x}"))?;
            }
        } else {
            for byte in &self.0[0..512] {
                f.write_fmt(format_args!("{byte:02x}"))?;
            }
            f.write_str("...")?;
            for byte in &self.0[self.0.len() - 512..] {
                f.write_fmt(format_args!("{byte:02x}"))?;
            }
        }
        Ok(())
    }
}

impl<'a> fmt::Debug for HexDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            f.write_fmt(format_args!("{byte:02x}"))?;
        }
        Ok(())
    }
}

pub fn to_precompile_id(spec_id: SpecId) -> revm::precompile::SpecId {
    match spec_id {
        SpecId::FRONTIER |
        SpecId::FRONTIER_THAWING |
        SpecId::HOMESTEAD |
        SpecId::DAO_FORK |
        SpecId::TANGERINE |
        SpecId::SPURIOUS_DRAGON => revm::precompile::SpecId::HOMESTEAD,
        SpecId::BYZANTIUM | SpecId::CONSTANTINOPLE | SpecId::PETERSBURG => {
            revm::precompile::SpecId::BYZANTIUM
        }
        SpecId::ISTANBUL | SpecId::MUIR_GLACIER => revm::precompile::SpecId::ISTANBUL,
        SpecId::BERLIN |
        SpecId::LONDON |
        SpecId::ARROW_GLACIER |
        SpecId::GRAY_GLACIER |
        SpecId::MERGE |
        SpecId::SHANGHAI |
        SpecId::CANCUN |
        SpecId::LATEST => revm::precompile::SpecId::BERLIN,
    }
}
