use alloy_consensus::{
    EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH, Header, constants::EMPTY_WITHDRAWALS,
};
use alloy_eips::{eip4895::Withdrawals, eip7685::EMPTY_REQUESTS_HASH};
use alloy_primitives::{Address, B256, Bytes, U256};
use eyre::{Result, WrapErr};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use foundry_evm_hardforks::EthereumHardfork;
use foundry_primitives::FoundryHeader;
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
    #[serde(default, deserialize_with = "deserialize_block_env_compat")]
    pub block: Option<BlockEnv>,
    /// The accounts and their state.
    #[serde(default)]
    pub accounts: BTreeMap<Address, SerializableAccountRecord>,
    /// The head block number at the time of the dump.
    #[serde(
        default,
        deserialize_with = "deserialize_quantity_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub best_block_number: Option<u64>,
    /// The blocks of the chain.
    #[serde(default)]
    pub blocks: Vec<SerializableBlock>,
    /// The mined transactions of the chain, with their receipts.
    #[serde(default)]
    pub transactions: Vec<SerializableTransaction>,
    /// The state at every block, when preserved.
    #[serde(default)]
    pub historical_states: Option<SerializableHistoricalStates>,
}

/// The hardforks a checkpoint block's header reflects.
#[derive(Clone, Copy, Debug, Default)]
pub struct CheckpointForks {
    /// London: the header has a base fee.
    pub london: bool,
    /// Shanghai: the header has a withdrawals root.
    pub shanghai: bool,
    /// Cancun: the header has blob gas fields and a parent beacon block root.
    pub cancun: bool,
    /// Prague: the header has a requests hash.
    pub prague: bool,
}

/// One block of a state dump.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableBlock {
    /// Source hardfork used for a transaction-hash replay, independent of later blocks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_hardfork: Option<EthereumHardfork>,
    /// Arbitrum's L1 execution number, distinct from the consensus header's L2 number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub l1_block_number: Option<u64>,
    /// Chain id used when this block was executed, retained across later chain-id changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_chain_id: Option<u64>,
    /// The header: a Tempo header keeps its Tempo fields.
    #[serde(deserialize_with = "deserialize_header_compat")]
    pub header: FoundryHeader,
    /// The transactions, in block order.
    #[serde(default)]
    pub transactions: Vec<SerializableTransactionType>,
    /// The ommers.
    #[serde(default, deserialize_with = "deserialize_headers_compat")]
    pub ommers: Vec<Header>,
    /// The withdrawals, from Shanghai on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawals: Option<Withdrawals>,
}

/// A transaction of a dumped block: with the sender it was impersonated from, or the signed
/// transaction alone, as older dumps store it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SerializableTransactionType {
    /// The transaction and its impersonated sender, if any.
    MaybeImpersonatedTransaction(ImpersonatedTransaction),
    /// The signed transaction alone.
    TypedTransaction(serde_json::Value),
}

impl SerializableTransactionType {
    /// Returns the signed transaction as JSON.
    pub const fn transaction(&self) -> &serde_json::Value {
        match self {
            Self::MaybeImpersonatedTransaction(transaction) => &transaction.transaction,
            Self::TypedTransaction(transaction) => transaction,
        }
    }

    /// Returns the sender the transaction was impersonated from, if any.
    pub const fn impersonated_sender(&self) -> Option<Address> {
        match self {
            Self::MaybeImpersonatedTransaction(transaction) => transaction.impersonated_sender,
            Self::TypedTransaction(_) => None,
        }
    }
}

/// A signed transaction and the sender it was impersonated from, if any.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImpersonatedTransaction {
    /// The signed transaction as JSON.
    pub transaction: serde_json::Value,
    /// The impersonated sender, when the signature does not recover the sender.
    #[serde(default)]
    pub impersonated_sender: Option<Address>,
}

/// A mined transaction of a state dump: its outcome and receipt.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableTransaction {
    /// The outcome of the transaction.
    pub info: TransactionInfo,
    /// The receipt as JSON.
    pub receipt: serde_json::Value,
    /// The hash of the block that mined the transaction.
    pub block_hash: B256,
    /// The number of the block that mined the transaction.
    pub block_number: u64,
}

/// The outcome of a mined transaction, in anvil's format.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionInfo {
    /// The transaction hash.
    pub transaction_hash: B256,
    /// The index in the block.
    pub transaction_index: u64,
    /// The sender.
    pub from: Address,
    /// The recipient, for a call.
    pub to: Option<Address>,
    /// The created contract, for a create.
    pub contract_address: Option<Address>,
    /// The call traces, as anvil records them.
    #[serde(default)]
    pub traces: Vec<serde_json::Value>,
    /// The exit reason, as anvil names it.
    pub exit: String,
    /// The output.
    #[serde(default)]
    pub out: Option<Bytes>,
    /// The nonce.
    pub nonce: u64,
    /// The gas the transaction used.
    pub gas_used: u64,
}

/// The state at every block of a dump, by block hash.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SerializableHistoricalStates(pub Vec<(B256, StateSnapshot)>);

impl IntoIterator for SerializableHistoricalStates {
    type Item = (B256, StateSnapshot);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// The state at one block.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// The accounts.
    #[serde(default)]
    pub accounts: BTreeMap<Address, SnapshotAccount>,
    /// The storage of the accounts.
    #[serde(default)]
    pub storage: BTreeMap<Address, BTreeMap<U256, U256>>,
    /// The hashes of the blocks before this one.
    #[serde(default)]
    pub block_hashes: BTreeMap<U256, B256>,
}

/// One account of a state snapshot, in revm's account info format.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SnapshotAccount {
    /// The balance.
    pub balance: U256,
    /// The nonce.
    pub nonce: u64,
    /// The code hash.
    pub code_hash: B256,
    /// The code, as revm serializes it.
    #[serde(default)]
    pub code: Option<serde_json::Value>,
}

/// One account in a state dump.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SerializableAccountRecord {
    /// The nonce.
    #[serde(deserialize_with = "deserialize_quantity")]
    pub nonce: u64,
    /// The balance.
    pub balance: U256,
    /// The code.
    pub code: Bytes,
    /// The storage. Older dumps write the slots and values as short quantities.
    #[serde(deserialize_with = "deserialize_storage")]
    pub storage: BTreeMap<B256, B256>,
}

/// Reads a number that older dumps write as a quantity string.
fn deserialize_quantity<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u64, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    match &value {
        serde_json::Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom(format!("{number} is not a u64"))),
        serde_json::Value::String(text) => text
            .strip_prefix("0x")
            .and_then(|hex| u64::from_str_radix(hex, 16).ok())
            .or_else(|| text.parse().ok())
            .ok_or_else(|| serde::de::Error::custom(format!("{text:?} is not a quantity"))),
        other => Err(serde::de::Error::custom(format!("{other} is not a quantity"))),
    }
}

/// Reads an optional number that older dumps write as a quantity string.
fn deserialize_quantity_opt<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    let Some(value) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    deserialize_quantity(value).map(Some).map_err(serde::de::Error::custom)
}

/// Reads storage slots and values as 256-bit words from quantities of any length.
fn deserialize_storage<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<B256, B256>, D::Error> {
    Ok(BTreeMap::<U256, U256>::deserialize(deserializer)?
        .into_iter()
        .map(|(slot, value)| (B256::from(slot), B256::from(value)))
        .collect())
}

/// The optional header fields a dump from an older anvil may lack.
const OPTIONAL_HEADER_FIELDS: [&str; 8] = [
    "baseFeePerGas",
    "withdrawalsRoot",
    "blobGasUsed",
    "excessBlobGas",
    "parentBeaconBlockRoot",
    "requestsHash",
    "blockAccessListHash",
    "slot_num",
];

/// Fills in the optional fields a header from an older dump lacks.
fn header_from_value<H: serde::de::DeserializeOwned>(
    mut value: serde_json::Value,
) -> Result<H, serde_json::Error> {
    if let Some(header) = value.as_object_mut() {
        for field in OPTIONAL_HEADER_FIELDS {
            header.entry(field).or_insert(serde_json::Value::Null);
        }
    }
    serde_json::from_value(value)
}

/// Reads a header of a dump from an older anvil, which may lack the newer optional fields. A
/// header with the Tempo fields reads as a Tempo header.
fn deserialize_header_compat<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<FoundryHeader, D::Error> {
    header_from_value(serde_json::Value::deserialize(deserializer)?)
        .map_err(serde::de::Error::custom)
}

/// Reads the ommers of a dump from an older anvil.
fn deserialize_headers_compat<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Header>, D::Error> {
    Vec::<serde_json::Value>::deserialize(deserializer)?
        .into_iter()
        .map(|value| header_from_value(value).map_err(serde::de::Error::custom))
        .collect()
}

/// Converts a value through its JSON form, between types with the same serialization.
pub(crate) fn json_convert<S: Serialize, D: serde::de::DeserializeOwned>(
    value: &S,
) -> ProviderResult<D> {
    serde_json::to_value(value).and_then(serde_json::from_value).map_err(|error| {
        reth_ethereum::provider::ProviderError::other(std::io::Error::other(error))
    })
}

/// Reads the block environment of a dump. Older dumps name the beneficiary `coinbase` and write
/// the gas limit, the base fee, and the blob gas values as quantity strings.
fn deserialize_block_env_compat<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<BlockEnv>, D::Error> {
    let Some(mut value) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    if let Some(block) = value.as_object_mut() {
        if let Some(coinbase) = block.remove("coinbase") {
            block.entry("beneficiary").or_insert(coinbase);
        }
        for key in ["gas_limit", "basefee"] {
            quantity_to_number(block.get_mut(key));
        }
        // Dumps from before Amsterdam have no slot number, and older ones no blob gas.
        block.entry("slot_num").or_insert(serde_json::Value::from(0u64));
        block
            .entry("blob_excess_gas_and_price")
            .or_insert(serde_json::json!({ "excess_blob_gas": 0, "blob_gasprice": 1 }));
        if let Some(blob) =
            block.get_mut("blob_excess_gas_and_price").and_then(|blob| blob.as_object_mut())
        {
            for key in ["excess_blob_gas", "blob_gasprice"] {
                quantity_to_number(blob.get_mut(key));
            }
        }
    }
    serde_json::from_value(value).map(Some).map_err(serde::de::Error::custom)
}

/// Turns a quantity string into a JSON number, when it fits one.
fn quantity_to_number(value: Option<&mut serde_json::Value>) {
    if let Some(value) = value
        && let Some(text) = value.as_str()
        && let Some(hex) = text.strip_prefix("0x")
        && let Ok(number) = u128::from_str_radix(hex, 16)
    {
        *value = serde_json::Value::from(number);
    }
}

impl SerializableState {
    /// Returns the block environment at the time of the dump, if the dump has one.
    pub fn block_env(&self) -> Option<BlockEnv> {
        self.block.clone()
    }

    /// Returns the head block number at the time of the dump.
    pub fn head_number(&self) -> Option<u64> {
        self.best_block_number
            .or_else(|| self.block_env().map(|block| block.number.saturating_to::<u64>()))
    }

    /// Returns the head block of the dump: the last block with the head number, as anvil selects
    /// it when the dump holds several blocks at that height.
    pub fn head_block(&self) -> Option<&SerializableBlock> {
        let number = self.head_number()?;
        self.blocks.iter().rev().find(|block| block.header.number == number)
    }

    /// Gives a dump without a block at its head a checkpoint block to continue from, as anvil
    /// does: a synthetic header built from the block environment on top of `parent_hash`.
    pub fn synthesize_head(
        &mut self,
        parent_hash: B256,
        forks: CheckpointForks,
        convert: fn(Header) -> FoundryHeader,
    ) {
        let Some(block) = self.block_env() else { return };
        let number = self.head_number().unwrap_or_else(|| block.number.saturating_to());
        let header = Header {
            parent_hash,
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            beneficiary: block.beneficiary,
            state_root: B256::ZERO,
            transactions_root: EMPTY_ROOT_HASH,
            receipts_root: EMPTY_ROOT_HASH,
            difficulty: block.difficulty,
            number,
            gas_limit: block.gas_limit,
            timestamp: block.timestamp.saturating_to(),
            mix_hash: block.prevrandao.unwrap_or_default(),
            base_fee_per_gas: forks.london.then_some(block.basefee),
            withdrawals_root: forks.shanghai.then_some(EMPTY_WITHDRAWALS),
            blob_gas_used: forks.cancun.then_some(0),
            excess_blob_gas: forks
                .cancun
                .then(|| block.blob_excess_gas_and_price.map(|blob| blob.excess_blob_gas))
                .flatten(),
            parent_beacon_block_root: forks.cancun.then_some(B256::ZERO),
            requests_hash: forks.prague.then_some(EMPTY_REQUESTS_HASH),
            ..Default::default()
        };
        tracing::warn!(
            target: "node",
            block_number = number,
            "state dump has no block history; created a synthetic checkpoint block"
        );
        self.blocks.push(SerializableBlock {
            l1_block_number: None,
            execution_chain_id: None,
            replay_hardfork: None,
            header: convert(header),
            transactions: Vec::new(),
            ommers: Vec::new(),
            withdrawals: forks.shanghai.then(Default::default),
        });
    }

    /// Returns the hash of the last block with the given number.
    pub fn block_hash(&self, number: u64) -> Option<B256> {
        self.blocks
            .iter()
            .rev()
            .find(|block| block.header.number == number)
            .map(|block| block.header.hash_slow())
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

/// Reads the local chain for `anvil_dumpState`.
pub trait StateDump: Send + Sync {
    /// Returns the accounts of the latest state, without the anvil state write overlay.
    fn dump_accounts(&self) -> ProviderResult<BTreeMap<Address, SerializableAccountRecord>>;

    /// Returns the blocks of the chain and their mined transactions. `impersonated` names the
    /// sender of a transaction whose signature does not recover one.
    fn dump_blocks(
        &self,
        impersonated: &dyn Fn(B256) -> Option<Address>,
    ) -> ProviderResult<(Vec<SerializableBlock>, Vec<SerializableTransaction>)>;

    /// Returns the state at every block of the chain, oldest first.
    fn dump_snapshots(&self) -> ProviderResult<Vec<(B256, StateSnapshot)>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_the_optional_header_fields() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/state-dump.json");
        let dump: serde_json::Value = serde_json::from_reader(File::open(path).unwrap()).unwrap();
        let header = dump["blocks"][0]["header"].clone();
        let filled = header_from_value::<Header>(header.clone());
        assert!(filled.is_ok(), "{:?}: {header}", filled.err());
    }

    #[test]
    fn loads_the_dump_fixtures() {
        for file in ["state-dump.json", "state-dump-legacy.json", "state-dump-legacy-stress.json"] {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/").to_string() + file;
            let state =
                SerializableState::load(&path).unwrap_or_else(|error| panic!("{file}: {error:#}"));
            assert!(state.head_number().is_some(), "{file}");
        }
    }
}
