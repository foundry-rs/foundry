use alloy_primitives::{hex, Address};
use itertools::Itertools;
use revm::{
    precompile::{PrecompileSpecId, Precompiles},
    primitives::hardfork::SpecId,
};

pub fn get_precompiles_for(spec_id: SpecId) -> Vec<Address> {
    Precompiles::new(PrecompileSpecId::from_spec_id(spec_id)).addresses().copied().collect()
}

/// Formats values as hex strings, separated by commas.
pub fn hex_fmt_many<I, T>(i: I) -> String
where
    I: IntoIterator<Item = T>,
    T: AsRef<[u8]>,
{
    let items = i.into_iter().map(|item| hex::encode(item.as_ref())).format(", ");
    format!("{items}")
}
