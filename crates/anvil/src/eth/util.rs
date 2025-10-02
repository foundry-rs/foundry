use alloy_primitives::hex;
use itertools::Itertools;

/// Formats values as hex strings, separated by commas.
pub fn hex_fmt_many<I, T>(i: I) -> String
where
    I: IntoIterator<Item = T>,
    T: AsRef<[u8]>,
{
    let items = i.into_iter().map(|item| hex::encode(item.as_ref())).format(", ");
    format!("{items}")
}
