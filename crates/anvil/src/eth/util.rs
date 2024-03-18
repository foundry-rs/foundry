use alloy_primitives::Address;
use foundry_evm::revm::{
    precompile::{PrecompileSpecId, Precompiles},
    primitives::SpecId,
};
use std::fmt;

pub fn get_precompiles_for(spec_id: SpecId) -> Vec<Address> {
    Precompiles::new(PrecompileSpecId::from_spec_id(spec_id)).addresses().copied().collect()
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
