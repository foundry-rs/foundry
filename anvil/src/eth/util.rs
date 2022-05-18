use ethers::types::H160;
use std::fmt;

macro_rules! precompile_addr {
    ($idx:expr) => {{
        H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, $idx])
    }};
}

/// All ethereum precompiles ref <https://ethereum.github.io/yellowpaper/paper.pdf>
pub static PRECOMPILES: [H160; 9] = [
    // ecrecover
    precompile_addr!(1),
    // keccak
    precompile_addr!(2),
    // ripemd
    precompile_addr!(3),
    // identity
    precompile_addr!(4),
    // modexp
    precompile_addr!(5),
    // ecadd
    precompile_addr!(6),
    // ecmul
    precompile_addr!(7),
    // ecpairing
    precompile_addr!(8),
    // blake2f
    precompile_addr!(9),
];

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
