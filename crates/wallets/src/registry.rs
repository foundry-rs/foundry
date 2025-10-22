use alloy_primitives::Address;
use eyre::{Context, Result};
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WalletKind {
    Ledger,
    Trezor,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletRegistryEntry {
    pub name: String,
    pub kind: WalletKind,
    #[serde(default)]
    pub hd_path: Option<String>,
    #[serde(default)]
    pub mnemonic_index: Option<u32>,
    #[serde(default)]
    pub cached_public_key: Option<String>,
    #[serde(default)]
    pub cached_address: Option<Address>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WalletRegistry {
    #[serde(default)]
    pub wallets: BTreeMap<String, WalletRegistryEntry>,
}

impl WalletRegistry {
    fn file_path() -> Result<PathBuf> {
        Config::foundry_wallets_file().ok_or_else(|| eyre::eyre!("Could not find foundry dir"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::file_path()?;
        if !path.exists() {
            return Ok(Default::default());
        }
        let data = fs::read_to_string(&path)
            .wrap_err_with(|| format!("Failed to read wallets registry at {}", path.display()))?;
        let reg: Self = serde_json::from_str(&data)
            .wrap_err_with(|| format!("Failed to parse wallets registry at {}", path.display()))?;
        Ok(reg)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::file_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&path, json)
            .wrap_err_with(|| format!("Failed to write wallets registry at {}", path.display()))
    }

    pub fn get(&self, name: &str) -> Option<&WalletRegistryEntry> {
        self.wallets.get(name)
    }

    pub fn set(&mut self, entry: WalletRegistryEntry) {
        self.wallets.insert(entry.name.clone(), entry);
    }

    pub fn remove(&mut self, name: &str) {
        self.wallets.remove(name);
    }

    pub fn list(&self) -> impl Iterator<Item = (&String, &WalletRegistryEntry)> {
        self.wallets.iter()
    }
}
