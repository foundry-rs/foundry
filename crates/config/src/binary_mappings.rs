use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// Binaries that can be remapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BinaryName {
    Forge,
    Anvil,
    Cast,
    Chisel,
}

impl TryFrom<&str> for BinaryName {
    type Error = eyre::Error;

    fn try_from(value: &str) -> eyre::Result<Self> {
        match value {
            "forge" => Ok(Self::Forge),
            "anvil" => Ok(Self::Anvil),
            "cast" => Ok(Self::Cast),
            "chisel" => Ok(Self::Chisel),
            _ => eyre::bail!("Invalid binary name: {value}"),
        }
    }
}

/// Contains the config for binary remappings,
/// e.g. ability to redirect any of the foundry binaries to some other binary.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryMappings {
    /// The mappings from binary name to the path of the binary.
    #[serde(flatten)]
    pub mappings: HashMap<BinaryName, PathBuf>,
}

impl BinaryMappings {
    /// Tells if the binary name is remapped to some other binary.
    /// This function will return `None` if the binary name cannot be parsed or if
    /// the binary name is not remapped.
    pub fn redirect_for(&self, binary_name: &str) -> Option<&PathBuf> {
        // Sanitize the path so that it
        let binary_name = Path::new(binary_name).file_stem()?.to_str()?;
        let binary_name = BinaryName::try_from(binary_name).ok()?;
        self.mappings.get(&binary_name)
    }
}

impl<T> From<T> for BinaryMappings
where
    T: Into<HashMap<BinaryName, PathBuf>>,
{
    fn from(mappings: T) -> Self {
        Self { mappings: mappings.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_names() {
        assert_eq!(BinaryName::try_from("forge").unwrap(), BinaryName::Forge);
        assert_eq!(BinaryName::try_from("anvil").unwrap(), BinaryName::Anvil);
        assert_eq!(BinaryName::try_from("cast").unwrap(), BinaryName::Cast);
        assert_eq!(BinaryName::try_from("chisel").unwrap(), BinaryName::Chisel);
    }

    #[test]
    fn binary_names_serde() {
        let test_vector = [
            (BinaryName::Forge, r#""forge""#),
            (BinaryName::Anvil, r#""anvil""#),
            (BinaryName::Cast, r#""cast""#),
            (BinaryName::Chisel, r#""chisel""#),
        ];

        for (binary_name, expected) in test_vector.iter() {
            let serialized = serde_json::to_string(binary_name).unwrap();
            assert_eq!(serialized, *expected);

            let deserialized: BinaryName = serde_json::from_str(expected).unwrap();
            assert_eq!(deserialized, *binary_name);
        }
    }

    #[test]
    fn redirect_to() {
        let mappings = BinaryMappings::from([
            (BinaryName::Forge, PathBuf::from("forge-zksync")),
            (BinaryName::Anvil, PathBuf::from("anvil-zksync")),
            (BinaryName::Cast, PathBuf::from("cast-zksync")),
            (BinaryName::Chisel, PathBuf::from("chisel-zksync")),
        ]);

        assert_eq!(mappings.redirect_for("forge"), Some(&PathBuf::from("forge-zksync")));
        assert_eq!(mappings.redirect_for("anvil"), Some(&PathBuf::from("anvil-zksync")));
        assert_eq!(mappings.redirect_for("cast"), Some(&PathBuf::from("cast-zksync")));
        assert_eq!(mappings.redirect_for("chisel"), Some(&PathBuf::from("chisel-zksync")));
        assert_eq!(mappings.redirect_for("invalid"), None);
        assert_eq!(mappings.redirect_for("/usr/bin/forge"), Some(&PathBuf::from("forge-zksync")));
        assert_eq!(mappings.redirect_for("anvil.exe"), Some(&PathBuf::from("anvil-zksync")));
    }
}
