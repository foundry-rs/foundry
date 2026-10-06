use alloy_primitives::{Address, B256, Bytes, U256};
use eyre::{Result, WrapErr};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use reth_ethereum::storage::errors::provider::ProviderResult;
use revm::context::BlockEnv;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Read, Write},
    path::Path,
};

/// The state dump format of `anvil_dumpState` and `--dump-state`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SerializableState {
    /// The block environment at the time of the dump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<serde_json::Value>,
    /// The accounts and their state.
    #[serde(default)]
    pub accounts: BTreeMap<Address, SerializableAccountRecord>,
    /// The head block number at the time of the dump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_block_number: Option<u64>,
    /// The blocks of the chain, when historical states were preserved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<serde_json::Value>,
    /// The transactions of the chain, when historical states were preserved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transactions: Vec<serde_json::Value>,
    /// The historical states, when preserved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub historical_states: Option<serde_json::Value>,
}

/// One account in a state dump.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SerializableAccountRecord {
    /// The nonce.
    pub nonce: u64,
    /// The balance.
    pub balance: U256,
    /// The code.
    pub code: Bytes,
    /// The storage.
    pub storage: BTreeMap<B256, B256>,
}

impl SerializableState {
    /// Returns the block environment at the time of the dump, if the dump has one.
    pub fn block_env(&self) -> Option<BlockEnv> {
        self.block.clone().and_then(|block| serde_json::from_value(block).ok())
    }

    /// Returns the head block number at the time of the dump.
    pub fn head_number(&self) -> Option<u64> {
        self.best_block_number
            .or_else(|| self.block_env().map(|block| block.number.saturating_to::<u64>()))
    }

    /// Encodes the dump as gzipped JSON, the wire format of `anvil_dumpState`.
    pub fn encode(&self) -> Result<Bytes> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&serde_json::to_vec(self)?)?;
        Ok(encoder.finish()?.into())
    }

    /// Loads a state file. A directory resolves to `state.json` inside it.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let mut path = path.as_ref().to_path_buf();
        if path.is_dir() {
            path = path.join("state.json");
        }
        let file =
            File::open(&path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
        serde_json::from_reader(BufReader::new(file))
            .wrap_err_with(|| format!("failed to parse {}", path.display()))
    }

    /// Clap value parser for state files.
    pub fn parse(path: &str) -> Result<Self, String> {
        Self::load(path).map_err(|err| err.to_string())
    }

    /// Decodes a dump returned by `anvil_dumpState`: gzipped or plain JSON.
    pub fn decode(buf: &[u8]) -> Result<Self> {
        let mut decoder = GzDecoder::new(buf);
        if decoder.header().is_some() {
            let mut decoded = Vec::new();
            decoder.read_to_end(&mut decoded)?;
            return Ok(serde_json::from_slice(&decoded)?);
        }
        Ok(serde_json::from_slice(buf)?)
    }
}

/// A `--state` file: the path to dump to, and the state loaded from it if it exists.
#[derive(Clone, Debug)]
pub struct StateFile {
    /// The path to dump to.
    pub path: std::path::PathBuf,
    /// The state loaded from the path, if the file existed.
    pub state: Option<SerializableState>,
}

impl StateFile {
    /// Clap value parser for `--state`.
    pub fn parse(path: &str) -> Result<Self, String> {
        Self::parse_path(path)
    }

    /// Resolves the path and loads the state when the file exists.
    pub fn parse_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let mut path = path.as_ref().to_path_buf();
        if path.is_dir() {
            path = path.join("state.json");
        }
        let mut state = Self { path, state: None };
        if !state.path.exists() {
            return Ok(state);
        }
        state.state = Some(SerializableState::load(&state.path).map_err(|err| err.to_string())?);
        Ok(state)
    }
}

/// Reads every account of the local state, for `anvil_dumpState`.
pub trait AccountDump: Send + Sync {
    /// Returns the accounts of the latest state, without the anvil state write overlay.
    fn dump_accounts(&self) -> ProviderResult<BTreeMap<Address, SerializableAccountRecord>>;
}
