use crate::{
    block_number::BlockNumber,
    serde_helpers::{
        deserialize_stringified_block_number, deserialize_stringified_numeric,
        deserialize_stringified_numeric_opt, deserialize_stringified_u64,
        deserialize_stringified_u64_opt,
    },
    Client, EtherscanError, Query, Response, Result,
};
use alloy_primitives::{Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::HashMap,
    fmt::{Display, Error, Formatter},
};

/// The raw response from the balance-related API endpoints
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountBalance {
    pub account: Address,
    pub balance: String,
}

mod genesis_string {
    use super::*;
    use serde::{
        de::{DeserializeOwned, Error as _},
        ser::Error as _,
        Deserializer, Serializer,
    };

    pub(crate) fn serialize<T, S>(
        value: &GenesisOption<T>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        T: Serialize,
        S: Serializer,
    {
        let json = match value {
            GenesisOption::None => Cow::from(""),
            GenesisOption::Genesis => Cow::from("GENESIS"),
            GenesisOption::Some(value) => {
                serde_json::to_string(value).map_err(S::Error::custom)?.into()
            }
        };
        serializer.serialize_str(&json)
    }

    pub(crate) fn deserialize<'de, T, D>(
        deserializer: D,
    ) -> std::result::Result<GenesisOption<T>, D::Error>
    where
        T: DeserializeOwned,
        D: Deserializer<'de>,
    {
        let json = Cow::<'de, str>::deserialize(deserializer)?;
        if !json.is_empty() && !json.starts_with("GENESIS") {
            serde_json::from_str(&format!("\"{}\"", &json))
                .map(GenesisOption::Some)
                .map_err(D::Error::custom)
        } else if json.starts_with("GENESIS") {
            Ok(GenesisOption::Genesis)
        } else {
            Ok(GenesisOption::None)
        }
    }
}

mod json_string {
    use super::*;
    use serde::{
        de::{DeserializeOwned, Error as _},
        ser::Error as _,
        Deserializer, Serializer,
    };

    pub(crate) fn serialize<T, S>(
        value: &Option<T>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        T: Serialize,
        S: Serializer,
    {
        let json = match value {
            Option::None => Cow::from(""),
            Option::Some(value) => serde_json::to_string(value).map_err(S::Error::custom)?.into(),
        };
        serializer.serialize_str(&json)
    }

    pub(crate) fn deserialize<'de, T, D>(
        deserializer: D,
    ) -> std::result::Result<Option<T>, D::Error>
    where
        T: DeserializeOwned,
        D: Deserializer<'de>,
    {
        let json = Cow::<'de, str>::deserialize(deserializer)?;
        if json.is_empty() {
            Ok(Option::None)
        } else {
            serde_json::from_str(&format!("\"{}\"", &json))
                .map(Option::Some)
                .map_err(D::Error::custom)
        }
    }
}

/// Possible values for some field responses.
///
/// Transactions from the Genesis block may contain fields that do not conform to the expected
/// types.
#[derive(Clone, Debug)]
pub enum GenesisOption<T> {
    None,
    Genesis,
    Some(T),
}

impl<T> From<GenesisOption<T>> for Option<T> {
    fn from(value: GenesisOption<T>) -> Self {
        match value {
            GenesisOption::Some(value) => Some(value),
            _ => None,
        }
    }
}

impl<T> GenesisOption<T> {
    pub fn is_genesis(&self) -> bool {
        matches!(self, GenesisOption::Genesis)
    }

    pub fn value(&self) -> Option<&T> {
        match self {
            GenesisOption::Some(value) => Some(value),
            _ => None,
        }
    }
}

/// The raw response from the transaction list API endpoint
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalTransaction {
    pub is_error: String,
    #[serde(deserialize_with = "deserialize_stringified_block_number")]
    pub block_number: BlockNumber,
    pub time_stamp: String,
    #[serde(with = "genesis_string")]
    pub hash: GenesisOption<B256>,
    #[serde(with = "json_string")]
    pub nonce: Option<U256>,
    #[serde(with = "json_string")]
    pub block_hash: Option<U256>,
    #[serde(deserialize_with = "deserialize_stringified_u64_opt")]
    pub transaction_index: Option<u64>,
    #[serde(with = "genesis_string")]
    pub from: GenesisOption<Address>,
    #[serde(with = "json_string")]
    pub to: Option<Address>,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub value: U256,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas: U256,
    #[serde(deserialize_with = "deserialize_stringified_numeric_opt")]
    pub gas_price: Option<U256>,
    #[serde(rename = "txreceipt_status")]
    pub tx_receipt_status: String,
    pub input: Bytes,
    #[serde(with = "json_string")]
    pub contract_address: Option<Address>,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas_used: U256,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub cumulative_gas_used: U256,
    #[serde(deserialize_with = "deserialize_stringified_u64")]
    pub confirmations: u64,
    pub method_id: Option<Bytes>,
    #[serde(with = "json_string")]
    pub function_name: Option<String>,
}

/// The raw response from the internal transaction list API endpoint
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalTransaction {
    #[serde(deserialize_with = "deserialize_stringified_block_number")]
    pub block_number: BlockNumber,
    pub time_stamp: String,
    pub hash: B256,
    pub from: Address,
    #[serde(with = "genesis_string")]
    pub to: GenesisOption<Address>,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub value: U256,
    #[serde(with = "genesis_string")]
    pub contract_address: GenesisOption<Address>,
    #[serde(with = "genesis_string")]
    pub input: GenesisOption<Bytes>,
    #[serde(rename = "type")]
    pub result_type: String,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas: U256,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas_used: U256,
    pub trace_id: String,
    pub is_error: String,
    pub err_code: String,
}

/// The raw response from the ERC20 transfer list API endpoint
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ERC20TokenTransferEvent {
    #[serde(deserialize_with = "deserialize_stringified_block_number")]
    pub block_number: BlockNumber,
    pub time_stamp: String,
    pub hash: B256,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub nonce: U256,
    pub block_hash: B256,
    pub from: Address,
    pub contract_address: Address,
    pub to: Option<Address>,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub value: U256,
    pub token_name: String,
    pub token_symbol: String,
    pub token_decimal: String,
    #[serde(deserialize_with = "deserialize_stringified_u64")]
    pub transaction_index: u64,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas: U256,
    #[serde(deserialize_with = "deserialize_stringified_numeric_opt")]
    pub gas_price: Option<U256>,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas_used: U256,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub cumulative_gas_used: U256,
    /// deprecated
    pub input: String,
    #[serde(deserialize_with = "deserialize_stringified_u64")]
    pub confirmations: u64,
}

/// The raw response from the ERC721 transfer list API endpoint
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ERC721TokenTransferEvent {
    #[serde(deserialize_with = "deserialize_stringified_block_number")]
    pub block_number: BlockNumber,
    pub time_stamp: String,
    pub hash: B256,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub nonce: U256,
    pub block_hash: B256,
    pub from: Address,
    pub contract_address: Address,
    pub to: Option<Address>,
    #[serde(rename = "tokenID")]
    pub token_id: String,
    pub token_name: String,
    pub token_symbol: String,
    pub token_decimal: String,
    #[serde(deserialize_with = "deserialize_stringified_u64")]
    pub transaction_index: u64,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas: U256,
    #[serde(deserialize_with = "deserialize_stringified_numeric_opt")]
    pub gas_price: Option<U256>,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas_used: U256,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub cumulative_gas_used: U256,
    /// deprecated
    pub input: String,
    #[serde(deserialize_with = "deserialize_stringified_u64")]
    pub confirmations: u64,
}

/// The raw response from the ERC1155 transfer list API endpoint
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ERC1155TokenTransferEvent {
    #[serde(deserialize_with = "deserialize_stringified_block_number")]
    pub block_number: BlockNumber,
    pub time_stamp: String,
    pub hash: B256,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub nonce: U256,
    pub block_hash: B256,
    pub from: Address,
    pub contract_address: Address,
    pub to: Option<Address>,
    #[serde(rename = "tokenID")]
    pub token_id: String,
    pub token_value: String,
    pub token_name: String,
    pub token_symbol: String,
    #[serde(deserialize_with = "deserialize_stringified_u64")]
    pub transaction_index: u64,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas: U256,
    #[serde(deserialize_with = "deserialize_stringified_numeric_opt")]
    pub gas_price: Option<U256>,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas_used: U256,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub cumulative_gas_used: U256,
    /// deprecated
    pub input: String,
    #[serde(deserialize_with = "deserialize_stringified_u64")]
    pub confirmations: u64,
}

/// The raw response from the mined blocks API endpoint
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinedBlock {
    #[serde(deserialize_with = "deserialize_stringified_block_number")]
    pub block_number: BlockNumber,
    pub time_stamp: String,
    pub block_reward: String,
}

/// The pre-defined block parameter for balance API endpoints
#[derive(Clone, Copy, Debug, Default)]
pub enum Tag {
    Earliest,
    Pending,
    #[default]
    Latest,
}

impl Display for Tag {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::result::Result<(), Error> {
        match self {
            Tag::Earliest => write!(f, "earliest"),
            Tag::Pending => write!(f, "pending"),
            Tag::Latest => write!(f, "latest"),
        }
    }
}

/// The list sorting preference
#[derive(Clone, Copy, Debug)]
pub enum Sort {
    Asc,
    Desc,
}

impl Display for Sort {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::result::Result<(), Error> {
        match self {
            Sort::Asc => write!(f, "asc"),
            Sort::Desc => write!(f, "desc"),
        }
    }
}

/// Common optional arguments for the transaction or event list API endpoints
#[derive(Clone, Copy, Debug)]
pub struct TxListParams {
    pub start_block: u64,
    pub end_block: u64,
    pub page: u64,
    pub offset: u64,
    pub sort: Sort,
}

impl TxListParams {
    pub fn new(start_block: u64, end_block: u64, page: u64, offset: u64, sort: Sort) -> Self {
        Self { start_block, end_block, page, offset, sort }
    }
}

impl Default for TxListParams {
    fn default() -> Self {
        Self { start_block: 0, end_block: 99999999, page: 0, offset: 10000, sort: Sort::Asc }
    }
}

impl From<TxListParams> for HashMap<&'static str, String> {
    fn from(tx_params: TxListParams) -> Self {
        let mut params = HashMap::new();
        params.insert("startBlock", tx_params.start_block.to_string());
        params.insert("endBlock", tx_params.end_block.to_string());
        params.insert("page", tx_params.page.to_string());
        params.insert("offset", tx_params.offset.to_string());
        params.insert("sort", tx_params.sort.to_string());
        params
    }
}

/// Options for querying internal transactions
#[derive(Clone, Debug)]
#[allow(missing_copy_implementations)]
pub enum InternalTxQueryOption {
    ByAddress(Address),
    ByTransactionHash(B256),
    ByBlockRange,
}

/// Options for querying ERC20 or ERC721 token transfers
#[derive(Clone, Debug)]
#[allow(missing_copy_implementations)]
pub enum TokenQueryOption {
    ByAddress(Address),
    ByContract(Address),
    ByAddressAndContract(Address, Address),
}

impl TokenQueryOption {
    pub fn into_params(self, list_params: TxListParams) -> HashMap<&'static str, String> {
        let mut params: HashMap<&'static str, String> = list_params.into();
        match self {
            TokenQueryOption::ByAddress(address) => {
                params.insert("address", format!("{address:?}"));
                params
            }
            TokenQueryOption::ByContract(contract) => {
                params.insert("contractaddress", format!("{contract:?}"));
                params
            }
            TokenQueryOption::ByAddressAndContract(address, contract) => {
                params.insert("address", format!("{address:?}"));
                params.insert("contractaddress", format!("{contract:?}"));
                params
            }
        }
    }
}

/// The pre-defined block type for retrieving mined blocks
#[derive(Copy, Clone, Debug, Default)]
pub enum BlockType {
    #[default]
    CanonicalBlocks,
    Uncles,
}

impl Display for BlockType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::result::Result<(), Error> {
        match self {
            BlockType::CanonicalBlocks => write!(f, "blocks"),
            BlockType::Uncles => write!(f, "uncles"),
        }
    }
}

impl Client {
    /// Returns the Ether balance of a given address.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let address = "0x58eB28A67731c570Ef827C365c89B5751F9E6b0a".parse()?;
    /// let balance = client.get_ether_balance_single(&address, None).await?;
    /// # Ok(()) }
    /// ```
    pub async fn get_ether_balance_single(
        &self,
        address: &Address,
        tag: Option<Tag>,
    ) -> Result<AccountBalance> {
        let tag_str = tag.unwrap_or_default().to_string();
        let addr_str = format!("{address:?}");
        let query = self.create_query(
            "account",
            "balance",
            HashMap::from([("address", &addr_str), ("tag", &tag_str)]),
        );
        let response: Response<String> = self.get_json(&query).await?;

        match response.status.as_str() {
            "0" => Err(EtherscanError::BalanceFailed),
            "1" => Ok(AccountBalance { account: *address, balance: response.result }),
            err => Err(EtherscanError::BadStatusCode(err.to_string())),
        }
    }

    /// Returns the balance of the accounts from a list of addresses.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use alloy_primitives::Address;
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let addresses = [
    ///     "0x3E3c00494d0b306a0739E480DBB5DB91FFb5d4CB".parse::<Address>()?,
    ///     "0x7e9996ef050a9Fa7A01248e63271F69086aaFc9D".parse::<Address>()?,
    /// ];
    /// let balances = client.get_ether_balance_multi(&addresses, None).await?;
    /// assert_eq!(addresses.len(), balances.len());
    /// # Ok(()) }
    /// ```
    pub async fn get_ether_balance_multi(
        &self,
        addresses: &[Address],
        tag: Option<Tag>,
    ) -> Result<Vec<AccountBalance>> {
        let tag_str = tag.unwrap_or_default().to_string();
        let addrs = addresses.iter().map(|x| format!("{x:?}")).collect::<Vec<String>>().join(",");
        let query: Query<'_, HashMap<&str, &str>> = self.create_query(
            "account",
            "balancemulti",
            HashMap::from([("address", addrs.as_ref()), ("tag", tag_str.as_ref())]),
        );
        let response: Response<Vec<AccountBalance>> = self.get_json(&query).await?;

        match response.status.as_str() {
            "0" => Err(EtherscanError::BalanceFailed),
            "1" => Ok(response.result),
            err => Err(EtherscanError::BadStatusCode(err.to_string())),
        }
    }

    /// Returns the list of transactions performed by an address, with optional pagination.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let address = "0x1f162cf730564efD2Bb96eb27486A2801d76AFB6".parse()?;
    /// let transactions = client.get_transactions(&address, None).await?;
    /// # Ok(()) }
    /// ```
    pub async fn get_transactions(
        &self,
        address: &Address,
        params: Option<TxListParams>,
    ) -> Result<Vec<NormalTransaction>> {
        let mut tx_params: HashMap<&str, String> = params.unwrap_or_default().into();
        tx_params.insert("address", format!("{address:?}"));
        let query = self.create_query("account", "txlist", tx_params);
        let response: Response<Vec<NormalTransaction>> = self.get_json(&query).await?;

        Ok(response.result)
    }

    /// Returns the list of internal transactions performed by an address or within a transaction,
    /// with optional pagination.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use foundry_block_explorers::account::InternalTxQueryOption;
    ///
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let address = "0x2c1ba59d6f58433fb1eaee7d20b26ed83bda51a3".parse()?;
    /// let query = InternalTxQueryOption::ByAddress(address);
    /// let internal_transactions = client.get_internal_transactions(query, None).await?;
    /// # Ok(()) }
    /// ```
    pub async fn get_internal_transactions(
        &self,
        tx_query_option: InternalTxQueryOption,
        params: Option<TxListParams>,
    ) -> Result<Vec<InternalTransaction>> {
        let mut tx_params: HashMap<&str, String> = params.unwrap_or_default().into();
        match tx_query_option {
            InternalTxQueryOption::ByAddress(address) => {
                tx_params.insert("address", format!("{address:?}"));
            }
            InternalTxQueryOption::ByTransactionHash(tx_hash) => {
                tx_params.insert("txhash", format!("{tx_hash:?}"));
            }
            _ => {}
        }
        let query = self.create_query("account", "txlistinternal", tx_params);
        let response: Response<Vec<InternalTransaction>> = self.get_json(&query).await?;

        Ok(response.result)
    }

    /// Returns the list of ERC-20 tokens transferred by an address, with optional filtering by
    /// token contract.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use foundry_block_explorers::account::TokenQueryOption;
    ///
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let address = "0x4e83362442b8d1bec281594cea3050c8eb01311c".parse()?;
    /// let query = TokenQueryOption::ByAddress(address);
    /// let events = client.get_erc20_token_transfer_events(query, None).await?;
    /// # Ok(()) }
    /// ```
    pub async fn get_erc20_token_transfer_events(
        &self,
        event_query_option: TokenQueryOption,
        params: Option<TxListParams>,
    ) -> Result<Vec<ERC20TokenTransferEvent>> {
        let params = event_query_option.into_params(params.unwrap_or_default());
        let query = self.create_query("account", "tokentx", params);
        let response: Response<Vec<ERC20TokenTransferEvent>> = self.get_json(&query).await?;

        Ok(response.result)
    }

    /// Returns the list of ERC-721 ( NFT ) tokens transferred by an address, with optional
    /// filtering by token contract.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use foundry_block_explorers::account::TokenQueryOption;
    ///
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let contract = "0x06012c8cf97bead5deae237070f9587f8e7a266d".parse()?;
    /// let query = TokenQueryOption::ByContract(contract);
    /// let events = client.get_erc721_token_transfer_events(query, None).await?;
    /// # Ok(()) }
    /// ```
    pub async fn get_erc721_token_transfer_events(
        &self,
        event_query_option: TokenQueryOption,
        params: Option<TxListParams>,
    ) -> Result<Vec<ERC721TokenTransferEvent>> {
        let params = event_query_option.into_params(params.unwrap_or_default());
        let query = self.create_query("account", "tokennfttx", params);
        let response: Response<Vec<ERC721TokenTransferEvent>> = self.get_json(&query).await?;

        Ok(response.result)
    }

    /// Returns the list of ERC-1155 ( NFT ) tokens transferred by an address, with optional
    /// filtering by token contract.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use foundry_block_explorers::account::TokenQueryOption;
    ///
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let address = "0x216CD350a4044e7016f14936663e2880Dd2A39d7".parse()?;
    /// let contract = "0x495f947276749ce646f68ac8c248420045cb7b5e".parse()?;
    /// let query = TokenQueryOption::ByAddressAndContract(address, contract);
    /// let events = client.get_erc1155_token_transfer_events(query, None).await?;
    /// # Ok(()) }
    /// ```
    pub async fn get_erc1155_token_transfer_events(
        &self,
        event_query_option: TokenQueryOption,
        params: Option<TxListParams>,
    ) -> Result<Vec<ERC1155TokenTransferEvent>> {
        let params = event_query_option.into_params(params.unwrap_or_default());
        let query = self.create_query("account", "token1155tx", params);
        let response: Response<Vec<ERC1155TokenTransferEvent>> = self.get_json(&query).await?;

        Ok(response.result)
    }

    /// Returns the list of blocks mined by an address.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let address = "0x9dd134d14d1e65f84b706d6f205cd5b1cd03a46b".parse()?;
    /// let blocks = client.get_mined_blocks(&address, None, None).await?;
    /// # Ok(()) }
    /// ```
    pub async fn get_mined_blocks(
        &self,
        address: &Address,
        block_type: Option<BlockType>,
        page_and_offset: Option<(u64, u64)>,
    ) -> Result<Vec<MinedBlock>> {
        let mut params = HashMap::new();
        params.insert("address", format!("{address:?}"));
        params.insert("blocktype", block_type.unwrap_or_default().to_string());
        if let Some((page, offset)) = page_and_offset {
            params.insert("page", page.to_string());
            params.insert("offset", offset.to_string());
        }
        let query = self.create_query("account", "getminedblocks", params);
        let response: Response<Vec<MinedBlock>> = self.get_json(&query).await?;

        Ok(response.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // <https://github.com/gakonst/ethers-rs/issues/2612>
    #[test]
    fn can_parse_response_2612() {
        let err = r#"{
  "status": "1",
  "message": "OK",
  "result": [
    {
      "blockNumber": "18185184",
      "timeStamp": "1695310607",
      "hash": "0x95983231acd079498b7628c6b6dd4866f559a23120fbce590c5dd7f10c7628af",
      "nonce": "1325609",
      "blockHash": "0x61e106aa2446ba06fe0217eb5bd9dae98a72b56dad2c2197f60a0798ce9f0dc6",
      "transactionIndex": "45",
      "from": "0xae2fc483527b8ef99eb5d9b44875f005ba1fae13",
      "to": "0x6b75d8af000000e20b7a7ddf000ba900b4009a80",
      "value": "23283064365",
      "gas": "107142",
      "gasPrice": "15945612744",
      "isError": "0",
      "txreceipt_status": "1",
      "input": "0xe061",
      "contractAddress": "",
      "cumulativeGasUsed": "3013734",
      "gasUsed": "44879",
      "confirmations": "28565",
      "methodId": "0xe061",
      "functionName": ""
    }
  ]
}"#;
        let _resp: Response<Vec<NormalTransaction>> = serde_json::from_str(err).unwrap();
    }
}
