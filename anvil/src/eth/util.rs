use ethers::abi::Address;
use forge::revm::SpecId;
use std::fmt;

macro_rules! precompiles_for {
    ($spec:ident) => {{
        let precompiles =
            revm_precompiles::Precompiles::new::<{ SpecId::to_precompile_id(SpecId::$spec) }>();
        precompiles.as_slice().iter().map(|(a, _)| a).copied().collect()
    }};
}

pub fn get_precompiles_for(spec_id: SpecId) -> Vec<Address> {
    match spec_id {
        SpecId::FRONTIER => precompiles_for!(FRONTIER),
        SpecId::HOMESTEAD => precompiles_for!(HOMESTEAD),
        SpecId::TANGERINE => precompiles_for!(TANGERINE),
        SpecId::SPURIOUS_DRAGON => precompiles_for!(SPURIOUS_DRAGON),
        SpecId::BYZANTIUM => precompiles_for!(BYZANTIUM),
        SpecId::CONSTANTINOPLE => precompiles_for!(CONSTANTINOPLE),
        SpecId::PETERSBURG => precompiles_for!(PETERSBURG),
        SpecId::ISTANBUL => precompiles_for!(ISTANBUL),
        SpecId::MUIRGLACIER => precompiles_for!(MUIRGLACIER),
        SpecId::BERLIN => precompiles_for!(BERLIN),
        SpecId::LONDON => precompiles_for!(LONDON),
        SpecId::MERGE => precompiles_for!(MERGE),
        SpecId::LATEST => precompiles_for!(LATEST),
    }
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
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.0.len() < 1027 {
            for byte in self.0 {
                f.write_fmt(format_args!("{:02x}", byte))?;
            }
        } else {
            for byte in &self.0[0..512] {
                f.write_fmt(format_args!("{:02x}", byte))?;
            }
            f.write_str("...")?;
            for byte in &self.0[self.0.len() - 512..] {
                f.write_fmt(format_args!("{:02x}", byte))?;
            }
        }
        Ok(())
    }
}

impl<'a> fmt::Debug for HexDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for byte in self.0 {
            f.write_fmt(format_args!("{:02x}", byte))?;
        }
        Ok(())
    }
}
