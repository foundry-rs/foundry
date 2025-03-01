use crate::utils::ConversionError;
use std::{fmt, str::FromStr};

/// Common Ethereum unit types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Units {
    /// Wei is equivalent to 1 wei.
    Wei,
    /// Kwei is equivalent to 1e3 wei.
    Kwei,
    /// Mwei is equivalent to 1e6 wei.
    Mwei,
    /// Gwei is equivalent to 1e9 wei.
    Gwei,
    /// Twei is equivalent to 1e12 wei.
    Twei,
    /// Pwei is equivalent to 1e15 wei.
    Pwei,
    /// Ether is equivalent to 1e18 wei.
    Ether,
    /// Other less frequent unit sizes, equivalent to 1e{0} wei.
    Other(u32),
}

impl fmt::Display for Units {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_num().to_string().as_str())
    }
}

impl TryFrom<u32> for Units {
    type Error = ConversionError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(Units::Other(value))
    }
}

impl TryFrom<i32> for Units {
    type Error = ConversionError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Ok(Units::Other(value as u32))
    }
}

impl TryFrom<usize> for Units {
    type Error = ConversionError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Ok(Units::Other(value as u32))
    }
}

impl TryFrom<String> for Units {
    type Error = ConversionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_str(&value)
    }
}

impl<'a> TryFrom<&'a String> for Units {
    type Error = ConversionError;

    fn try_from(value: &'a String) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl TryFrom<&str> for Units {
    type Error = ConversionError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl FromStr for Units {
    type Err = ConversionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "eth" | "ether" => Units::Ether,
            "pwei" | "milli" | "milliether" | "finney" => Units::Pwei,
            "twei" | "micro" | "microether" | "szabo" => Units::Twei,
            "gwei" | "nano" | "nanoether" | "shannon" => Units::Gwei,
            "mwei" | "pico" | "picoether" | "lovelace" => Units::Mwei,
            "kwei" | "femto" | "femtoether" | "babbage" => Units::Kwei,
            "wei" => Units::Wei,
            _ => return Err(ConversionError::UnrecognizedUnits(s.to_string())),
        })
    }
}

impl From<Units> for u32 {
    fn from(units: Units) -> Self {
        units.as_num()
    }
}

impl From<Units> for i32 {
    fn from(units: Units) -> Self {
        units.as_num() as i32
    }
}

impl From<Units> for usize {
    fn from(units: Units) -> Self {
        units.as_num() as usize
    }
}

impl Units {
    pub fn as_num(&self) -> u32 {
        match self {
            Units::Wei => 0,
            Units::Kwei => 3,
            Units::Mwei => 6,
            Units::Gwei => 9,
            Units::Twei => 12,
            Units::Pwei => 15,
            Units::Ether => 18,
            Units::Other(inner) => *inner,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Units::*;

    #[test]
    fn test_units() {
        assert_eq!(Wei.as_num(), 0);
        assert_eq!(Kwei.as_num(), 3);
        assert_eq!(Mwei.as_num(), 6);
        assert_eq!(Gwei.as_num(), 9);
        assert_eq!(Twei.as_num(), 12);
        assert_eq!(Pwei.as_num(), 15);
        assert_eq!(Ether.as_num(), 18);
        assert_eq!(Other(10).as_num(), 10);
        assert_eq!(Other(20).as_num(), 20);
    }

    #[test]
    fn test_into() {
        assert_eq!(Units::try_from("wei").unwrap(), Wei);
        assert_eq!(Units::try_from("kwei").unwrap(), Kwei);
        assert_eq!(Units::try_from("mwei").unwrap(), Mwei);
        assert_eq!(Units::try_from("gwei").unwrap(), Gwei);
        assert_eq!(Units::try_from("twei").unwrap(), Twei);
        assert_eq!(Units::try_from("pwei").unwrap(), Pwei);
        assert_eq!(Units::try_from("ether").unwrap(), Ether);

        assert_eq!(Units::try_from("wei".to_string()).unwrap(), Wei);
        assert_eq!(Units::try_from("kwei".to_string()).unwrap(), Kwei);
        assert_eq!(Units::try_from("mwei".to_string()).unwrap(), Mwei);
        assert_eq!(Units::try_from("gwei".to_string()).unwrap(), Gwei);
        assert_eq!(Units::try_from("twei".to_string()).unwrap(), Twei);
        assert_eq!(Units::try_from("pwei".to_string()).unwrap(), Pwei);
        assert_eq!(Units::try_from("ether".to_string()).unwrap(), Ether);

        assert_eq!(Units::try_from(&"wei".to_string()).unwrap(), Wei);
        assert_eq!(Units::try_from(&"kwei".to_string()).unwrap(), Kwei);
        assert_eq!(Units::try_from(&"mwei".to_string()).unwrap(), Mwei);
        assert_eq!(Units::try_from(&"gwei".to_string()).unwrap(), Gwei);
        assert_eq!(Units::try_from(&"twei".to_string()).unwrap(), Twei);
        assert_eq!(Units::try_from(&"pwei".to_string()).unwrap(), Pwei);
        assert_eq!(Units::try_from(&"ether".to_string()).unwrap(), Ether);
    }
}
