//! Block RPC types.

use crate::Transaction;
use alloc::{collections::BTreeMap, vec::Vec};
use alloy_consensus::{BlockHeader, Sealed, TxEnvelope};
use alloy_eips::eip4895::Withdrawals;
use alloy_network_primitives::{
    BlockResponse, BlockTransactions, HeaderResponse, TransactionResponse,
};
use alloy_primitives::{Address, BlockHash, Bloom, Bytes, Sealable, B256, B64, U256};
use alloy_rlp::Encodable;
use core::ops::{Deref, DerefMut};

pub use alloy_eips::{
    calc_blob_gasprice, calc_excess_blob_gas, BlockHashOrNumber, BlockId, BlockNumHash,
    BlockNumberOrTag, ForkBlock, RpcBlockHash,
};
use alloy_eips::{eip7840::BlobParams, Encodable2718};

/// Block representation for RPC.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Block<T = Transaction<TxEnvelope>, H = Header> {
    /// Header of the block.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub header: H,
    /// Uncles' hashes.
    #[cfg_attr(feature = "serde", serde(default))]
    pub uncles: Vec<B256>,
    /// Block Transactions. In the case of an uncle block, this field is not included in RPC
    /// responses, and when deserialized, it will be set to [BlockTransactions::Uncle].
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "BlockTransactions::uncle",
            skip_serializing_if = "BlockTransactions::is_uncle"
        )
    )]
    pub transactions: BlockTransactions<T>,
    /// Withdrawals in the block.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub withdrawals: Option<Withdrawals>,
}

// cannot derive, as the derive impl would constrain `where T: Default`
impl<T, H: Default> Default for Block<T, H> {
    fn default() -> Self {
        Self {
            header: Default::default(),
            uncles: Default::default(),
            transactions: Default::default(),
            withdrawals: Default::default(),
        }
    }
}

impl<T, H> Block<T, H> {
    /// Creates a new empty block (without transactions).
    pub const fn empty(header: H) -> Self {
        Self::new(header, BlockTransactions::Full(vec![]))
    }

    /// Creates a new [`Block`] with the given header and transactions.
    ///
    /// Note: This does not set the withdrawals for the block.
    ///
    /// ```
    /// use alloy_eips::eip4895::Withdrawals;
    /// use alloy_network_primitives::BlockTransactions;
    /// use alloy_rpc_types_eth::{Block, Header, Transaction};
    /// let block = Block::new(
    ///     Header::new(alloy_consensus::Header::default()),
    ///     BlockTransactions::<Transaction>::Full(vec![]),
    /// )
    /// .with_withdrawals(Some(Withdrawals::default()));
    /// ```
    pub const fn new(header: H, transactions: BlockTransactions<T>) -> Self {
        Self { header, uncles: vec![], transactions, withdrawals: None }
    }

    /// Apply a function to the block, returning the modified block.
    pub fn apply<F>(self, f: F) -> Self
    where
        F: FnOnce(Self) -> Self,
    {
        f(self)
    }

    /// Sets the transactions for the block.
    pub fn with_transactions(mut self, transactions: BlockTransactions<T>) -> Self {
        self.transactions = transactions;
        self
    }

    /// Sets the withdrawals for the block.
    pub fn with_withdrawals(mut self, withdrawals: Option<Withdrawals>) -> Self {
        self.withdrawals = withdrawals;
        self
    }

    /// Sets the uncles for the block.
    pub fn with_uncles(mut self, uncles: Vec<B256>) -> Self {
        self.uncles = uncles;
        self
    }

    /// Converts the block's header type by applying a function to it.
    pub fn map_header<U>(self, f: impl FnOnce(H) -> U) -> Block<T, U> {
        Block {
            header: f(self.header),
            uncles: self.uncles,
            transactions: self.transactions,
            withdrawals: self.withdrawals,
        }
    }

    /// Converts the block's header type by applying a fallible function to it.
    pub fn try_map_header<U, E>(self, f: impl FnOnce(H) -> Result<U, E>) -> Result<Block<T, U>, E> {
        Ok(Block {
            header: f(self.header)?,
            uncles: self.uncles,
            transactions: self.transactions,
            withdrawals: self.withdrawals,
        })
    }

    /// Converts the block's transaction type by applying a function to each transaction.
    ///
    /// Returns the block with the new transaction type.
    pub fn map_transactions<U>(self, f: impl FnMut(T) -> U) -> Block<U, H> {
        Block {
            header: self.header,
            uncles: self.uncles,
            transactions: self.transactions.map(f),
            withdrawals: self.withdrawals,
        }
    }

    /// Converts the block's transaction type by applying a fallible function to each transaction.
    ///
    /// Returns the block with the new transaction type if all transactions were successfully.
    pub fn try_map_transactions<U, E>(
        self,
        f: impl FnMut(T) -> Result<U, E>,
    ) -> Result<Block<U, H>, E> {
        Ok(Block {
            header: self.header,
            uncles: self.uncles,
            transactions: self.transactions.try_map(f)?,
            withdrawals: self.withdrawals,
        })
    }

    /// Calculate the transaction root for the full transactions in this block type.
    ///
    /// Returns `None` if the `transactions` is not the [`BlockTransactions::Full`] variant.
    pub fn calculate_transactions_root(&self) -> Option<B256>
    where
        T: Encodable2718,
    {
        self.transactions.calculate_transactions_root()
    }
}

impl<T: TransactionResponse, H> Block<T, H> {
    /// Converts a block with Tx hashes into a full block.
    pub fn into_full_block(self, txs: Vec<T>) -> Self {
        Self { transactions: txs.into(), ..self }
    }
}

impl<T, H: Sealable + Encodable> Block<T, Header<H>> {
    /// Constructs an "uncle block" from the provided header.
    ///
    /// This function creates a new [`Block`] structure for uncle blocks (ommer blocks),
    /// using the provided [`alloy_consensus::Header`].
    pub fn uncle_from_header(header: H) -> Self {
        let block = alloy_consensus::Block::<TxEnvelope, H>::uncle(header);
        let size = U256::from(block.length());
        Self {
            uncles: vec![],
            header: Header::from_consensus(block.header.seal_slow(), None, Some(size)),
            transactions: BlockTransactions::Uncle,
            withdrawals: None,
        }
    }
}

impl<T> Block<T> {
    /// Constructs block from a consensus block and `total_difficulty`.
    pub fn from_consensus(block: alloy_consensus::Block<T>, total_difficulty: Option<U256>) -> Self
    where
        T: Encodable,
    {
        let size = U256::from(block.length());
        let alloy_consensus::Block {
            header,
            body: alloy_consensus::BlockBody { transactions, ommers, withdrawals },
        } = block;

        Self {
            header: Header::from_consensus(header.seal_slow(), total_difficulty, Some(size)),
            uncles: ommers.into_iter().map(|h| h.hash_slow()).collect(),
            transactions: BlockTransactions::Full(transactions),
            withdrawals,
        }
    }

    /// Consumes the block and returns the [`alloy_consensus::Block`].
    ///
    /// This has two caveats:
    ///  - The returned block will always have empty uncles.
    ///  - If the block's transaction is not [`BlockTransactions::Full`], the returned block will
    ///    have an empty transaction vec.
    pub fn into_consensus(self) -> alloy_consensus::Block<T> {
        let Self { header, transactions, withdrawals, .. } = self;
        alloy_consensus::BlockBody {
            transactions: transactions.into_transactions_vec(),
            ommers: vec![],
            withdrawals,
        }
        .into_block(header.into_consensus())
    }
}

/// RPC representation of block header, wrapping a consensus header.
///
/// This wraps the consensus header and adds additional fields for RPC.
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Header<H = alloy_consensus::Header> {
    /// Hash of the block
    pub hash: BlockHash,
    /// Inner consensus header.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub inner: H,
    /// Total difficulty
    ///
    /// Note: This field is now effectively deprecated: <https://github.com/ethereum/execution-apis/pull/570>
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub total_difficulty: Option<U256>,
    /// Integer the size of this block in bytes.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub size: Option<U256>,
}

impl<H> Header<H> {
    /// Create a new [`Header`] from a consensus header.
    ///
    /// Note: This will compute the hash of the header.
    pub fn new(inner: H) -> Self
    where
        H: Sealable,
    {
        Self::from_sealed(Sealed::new(inner))
    }

    /// Create a new [`Header`] from a sealed consensus header.
    ///
    /// Note: This does not set the total difficulty or size of the block.
    pub fn from_sealed(header: Sealed<H>) -> Self {
        let (inner, hash) = header.into_parts();
        Self { hash, inner, total_difficulty: None, size: None }
    }

    /// Consumes the type and returns the [`Sealed`] header.
    pub fn into_sealed(self) -> Sealed<H> {
        Sealed::new_unchecked(self.inner, self.hash)
    }

    /// Consumes the type and returns the wrapped consensus header.
    pub fn into_consensus(self) -> H {
        self.inner
    }

    /// Create a new [`Header`] from a sealed consensus header and additional fields.
    pub fn from_consensus(
        header: Sealed<H>,
        total_difficulty: Option<U256>,
        size: Option<U256>,
    ) -> Self {
        let (inner, hash) = header.into_parts();
        Self { hash, inner, total_difficulty, size }
    }

    /// Set the total difficulty of the block.
    pub const fn with_total_difficulty(mut self, total_difficulty: Option<U256>) -> Self {
        self.total_difficulty = total_difficulty;
        self
    }

    /// Set the size of the block.
    pub const fn with_size(mut self, size: Option<U256>) -> Self {
        self.size = size;
        self
    }
}

impl<H> Deref for Header<H> {
    type Target = H;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<H> DerefMut for Header<H> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<H> AsRef<H> for Header<H> {
    fn as_ref(&self) -> &H {
        &self.inner
    }
}

impl<H: BlockHeader> Header<H> {
    /// Returns the blob fee for _this_ block according to the EIP-4844 spec.
    ///
    /// Returns `None` if `excess_blob_gas` is None
    pub fn blob_fee(&self) -> Option<u128> {
        self.inner.excess_blob_gas().map(calc_blob_gasprice)
    }

    /// Returns the blob fee for the next block according to the EIP-4844 spec.
    ///
    /// Returns `None` if `excess_blob_gas` is None.
    ///
    /// See also [Self::next_block_excess_blob_gas]
    pub fn next_block_blob_fee(&self, blob_params: BlobParams) -> Option<u128> {
        self.inner.next_block_blob_fee(blob_params)
    }

    /// Calculate excess blob gas for the next block according to the EIP-4844
    /// spec.
    ///
    /// Returns a `None` if no excess blob gas is set, no EIP-4844 support
    pub fn next_block_excess_blob_gas(&self, blob_params: BlobParams) -> Option<u64> {
        self.inner.next_block_excess_blob_gas(blob_params)
    }
}

impl<H: BlockHeader> BlockHeader for Header<H> {
    fn parent_hash(&self) -> B256 {
        self.inner.parent_hash()
    }

    fn ommers_hash(&self) -> B256 {
        self.inner.ommers_hash()
    }

    fn beneficiary(&self) -> Address {
        self.inner.beneficiary()
    }

    fn state_root(&self) -> B256 {
        self.inner.state_root()
    }

    fn transactions_root(&self) -> B256 {
        self.inner.transactions_root()
    }

    fn receipts_root(&self) -> B256 {
        self.inner.receipts_root()
    }

    fn withdrawals_root(&self) -> Option<B256> {
        self.inner.withdrawals_root()
    }

    fn logs_bloom(&self) -> Bloom {
        self.inner.logs_bloom()
    }

    fn difficulty(&self) -> U256 {
        self.inner.difficulty()
    }

    fn number(&self) -> u64 {
        self.inner.number()
    }

    fn gas_limit(&self) -> u64 {
        self.inner.gas_limit()
    }

    fn gas_used(&self) -> u64 {
        self.inner.gas_used()
    }

    fn timestamp(&self) -> u64 {
        self.inner.timestamp()
    }

    fn mix_hash(&self) -> Option<B256> {
        self.inner.mix_hash()
    }

    fn nonce(&self) -> Option<B64> {
        self.inner.nonce()
    }

    fn base_fee_per_gas(&self) -> Option<u64> {
        self.inner.base_fee_per_gas()
    }

    fn blob_gas_used(&self) -> Option<u64> {
        self.inner.blob_gas_used()
    }

    fn excess_blob_gas(&self) -> Option<u64> {
        self.inner.excess_blob_gas()
    }

    fn parent_beacon_block_root(&self) -> Option<B256> {
        self.inner.parent_beacon_block_root()
    }

    fn requests_hash(&self) -> Option<B256> {
        self.inner.requests_hash()
    }

    fn extra_data(&self) -> &Bytes {
        self.inner.extra_data()
    }
}

impl<H: BlockHeader> HeaderResponse for Header<H> {
    fn hash(&self) -> BlockHash {
        self.hash
    }
}

impl From<Header> for alloy_consensus::Header {
    fn from(header: Header) -> Self {
        header.into_consensus()
    }
}

impl<H> From<Header<H>> for Sealed<H> {
    fn from(value: Header<H>) -> Self {
        value.into_sealed()
    }
}

/// Error that can occur when converting other types to blocks
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum BlockError {
    /// A transaction failed sender recovery
    #[error("transaction failed sender recovery")]
    InvalidSignature,
    /// A raw block failed to decode
    #[error("failed to decode raw block {0}")]
    RlpDecodeRawBlock(alloy_rlp::Error),
}

#[cfg(feature = "serde")]
impl From<Block> for alloy_serde::WithOtherFields<Block> {
    fn from(inner: Block) -> Self {
        Self { inner, other: Default::default() }
    }
}

#[cfg(feature = "serde")]
impl From<Header> for alloy_serde::WithOtherFields<Header> {
    fn from(inner: Header) -> Self {
        Self { inner, other: Default::default() }
    }
}

/// BlockOverrides is a set of header fields to override.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, rename_all = "camelCase", deny_unknown_fields))]
pub struct BlockOverrides {
    /// Overrides the block number.
    ///
    /// For `eth_callMany` this will be the block number of the first simulated block. Each
    /// following block increments its block number by 1
    // Note: geth uses `number`, erigon uses `blockNumber`
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none", alias = "blockNumber")
    )]
    pub number: Option<U256>,
    /// Overrides the difficulty of the block.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub difficulty: Option<U256>,
    /// Overrides the timestamp of the block.
    // Note: geth uses `time`, erigon uses `timestamp`
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            skip_serializing_if = "Option::is_none",
            alias = "timestamp",
            with = "alloy_serde::quantity::opt"
        )
    )]
    pub time: Option<u64>,
    /// Overrides the gas limit of the block.
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "alloy_serde::quantity::opt"
        )
    )]
    pub gas_limit: Option<u64>,
    /// Overrides the coinbase address of the block.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none", alias = "feeRecipient")
    )]
    pub coinbase: Option<Address>,
    /// Overrides the prevrandao of the block.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none", alias = "prevRandao")
    )]
    pub random: Option<B256>,
    /// Overrides the basefee of the block.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none", alias = "baseFeePerGas")
    )]
    pub base_fee: Option<U256>,
    /// A dictionary that maps blockNumber to a user-defined hash. It can be queried from the
    /// EVM opcode BLOCKHASH.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub block_hash: Option<BTreeMap<u64, B256>>,
}

impl<T: TransactionResponse, H> BlockResponse for Block<T, H> {
    type Header = H;
    type Transaction = T;

    fn header(&self) -> &Self::Header {
        &self.header
    }

    fn transactions(&self) -> &BlockTransactions<T> {
        &self.transactions
    }

    fn transactions_mut(&mut self) -> &mut BlockTransactions<Self::Transaction> {
        &mut self.transactions
    }
}

/// Bad block representation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct BadBlock {
    /// Underlying block object.
    block: Block,
    /// Hash of the block.
    hash: BlockHash,
    /// RLP encoded block header.
    rlp: Bytes,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{hex, keccak256, Bloom, B64};
    use arbitrary::Arbitrary;
    use rand::Rng;
    use similar_asserts::assert_eq;

    #[test]
    fn arbitrary_header() {
        let mut bytes = [0u8; 1024];
        rand::thread_rng().fill(bytes.as_mut_slice());
        let _: Header = Header::arbitrary(&mut arbitrary::Unstructured::new(&bytes)).unwrap();
    }

    #[test]
    #[cfg(all(feature = "jsonrpsee-types", feature = "serde"))]
    fn serde_json_header() {
        use jsonrpsee_types::SubscriptionResponse;
        let resp = r#"{"jsonrpc":"2.0","method":"eth_subscribe","params":{"subscription":"0x7eef37ff35d471f8825b1c8f67a5d3c0","result":{"hash":"0x7a7ada12e140961a32395059597764416499f4178daf1917193fad7bd2cc6386","parentHash":"0xdedbd831f496e705e7f2ec3c8dcb79051040a360bf1455dbd7eb8ea6ad03b751","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","miner":"0x0000000000000000000000000000000000000000","stateRoot":"0x0000000000000000000000000000000000000000000000000000000000000000","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","number":"0x8","gasUsed":"0x0","gasLimit":"0x1c9c380","extraData":"0x","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","timestamp":"0x642aa48f","difficulty":"0x0","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000"}}}"#;
        let _header: SubscriptionResponse<'_, Header> = serde_json::from_str(resp).unwrap();

        let resp = r#"{"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"0x1a14b6bdcf4542fabf71c4abee244e47","result":{"author":"0x000000568b9b5a365eaa767d42e74ed88915c204","difficulty":"0x1","extraData":"0x4e65746865726d696e6420312e392e32322d302d6463373666616366612d32308639ad8ff3d850a261f3b26bc2a55e0f3a718de0dd040a19a4ce37e7b473f2d7481448a1e1fd8fb69260825377c0478393e6055f471a5cf839467ce919a6ad2700","gasLimit":"0x7a1200","gasUsed":"0x0","hash":"0xa4856602944fdfd18c528ef93cc52a681b38d766a7e39c27a47488c8461adcb0","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","miner":"0x0000000000000000000000000000000000000000","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","number":"0x434822","parentHash":"0x1a9bdc31fc785f8a95efeeb7ae58f40f6366b8e805f47447a52335c95f4ceb49","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x261","stateRoot":"0xf38c4bf2958e541ec6df148e54ce073dc6b610f8613147ede568cb7b5c2d81ee","totalDifficulty":"0x633ebd","timestamp":"0x604726b0","transactions":[],"transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","uncles":[]}}}"#;
        let _header: SubscriptionResponse<'_, Header> = serde_json::from_str(resp).unwrap();
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_block() {
        use alloy_primitives::B64;

        let block = Block {
            header: Header {
                hash: B256::with_last_byte(1),
                inner: alloy_consensus::Header {
                    parent_hash: B256::with_last_byte(2),
                    ommers_hash: B256::with_last_byte(3),
                    beneficiary: Address::with_last_byte(4),
                    state_root: B256::with_last_byte(5),
                    transactions_root: B256::with_last_byte(6),
                    receipts_root: B256::with_last_byte(7),
                    withdrawals_root: Some(B256::with_last_byte(8)),
                    number: 9,
                    gas_used: 10,
                    gas_limit: 11,
                    extra_data: vec![1, 2, 3].into(),
                    logs_bloom: Default::default(),
                    timestamp: 12,
                    difficulty: U256::from(13),
                    mix_hash: B256::with_last_byte(14),
                    nonce: B64::with_last_byte(15),
                    base_fee_per_gas: Some(20),
                    blob_gas_used: None,
                    excess_blob_gas: None,
                    parent_beacon_block_root: None,
                    requests_hash: None,
                },
                total_difficulty: Some(U256::from(100000)),
                size: None,
            },
            uncles: vec![B256::with_last_byte(17)],
            transactions: vec![B256::with_last_byte(18)].into(),
            withdrawals: Some(Default::default()),
        };
        let serialized = serde_json::to_string(&block).unwrap();
        assert_eq!(
            serialized,
            r#"{"hash":"0x0000000000000000000000000000000000000000000000000000000000000001","parentHash":"0x0000000000000000000000000000000000000000000000000000000000000002","sha3Uncles":"0x0000000000000000000000000000000000000000000000000000000000000003","miner":"0x0000000000000000000000000000000000000004","stateRoot":"0x0000000000000000000000000000000000000000000000000000000000000005","transactionsRoot":"0x0000000000000000000000000000000000000000000000000000000000000006","receiptsRoot":"0x0000000000000000000000000000000000000000000000000000000000000007","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","difficulty":"0xd","number":"0x9","gasLimit":"0xb","gasUsed":"0xa","timestamp":"0xc","extraData":"0x010203","mixHash":"0x000000000000000000000000000000000000000000000000000000000000000e","nonce":"0x000000000000000f","baseFeePerGas":"0x14","withdrawalsRoot":"0x0000000000000000000000000000000000000000000000000000000000000008","totalDifficulty":"0x186a0","uncles":["0x0000000000000000000000000000000000000000000000000000000000000011"],"transactions":["0x0000000000000000000000000000000000000000000000000000000000000012"],"withdrawals":[]}"#
        );
        let deserialized: Block = serde_json::from_str(&serialized).unwrap();
        assert_eq!(block, deserialized);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_uncle_block() {
        use alloy_primitives::B64;

        let block = Block {
            header: Header {
                hash: B256::with_last_byte(1),
                inner: alloy_consensus::Header {
                    parent_hash: B256::with_last_byte(2),
                    ommers_hash: B256::with_last_byte(3),
                    beneficiary: Address::with_last_byte(4),
                    state_root: B256::with_last_byte(5),
                    transactions_root: B256::with_last_byte(6),
                    receipts_root: B256::with_last_byte(7),
                    withdrawals_root: Some(B256::with_last_byte(8)),
                    number: 9,
                    gas_used: 10,
                    gas_limit: 11,
                    extra_data: vec![1, 2, 3].into(),
                    logs_bloom: Default::default(),
                    timestamp: 12,
                    difficulty: U256::from(13),
                    mix_hash: B256::with_last_byte(14),
                    nonce: B64::with_last_byte(15),
                    base_fee_per_gas: Some(20),
                    blob_gas_used: None,
                    excess_blob_gas: None,
                    parent_beacon_block_root: None,
                    requests_hash: None,
                },
                size: None,
                total_difficulty: Some(U256::from(100000)),
            },
            uncles: vec![],
            transactions: BlockTransactions::Uncle,
            withdrawals: None,
        };
        let serialized = serde_json::to_string(&block).unwrap();
        assert_eq!(
            serialized,
            r#"{"hash":"0x0000000000000000000000000000000000000000000000000000000000000001","parentHash":"0x0000000000000000000000000000000000000000000000000000000000000002","sha3Uncles":"0x0000000000000000000000000000000000000000000000000000000000000003","miner":"0x0000000000000000000000000000000000000004","stateRoot":"0x0000000000000000000000000000000000000000000000000000000000000005","transactionsRoot":"0x0000000000000000000000000000000000000000000000000000000000000006","receiptsRoot":"0x0000000000000000000000000000000000000000000000000000000000000007","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","difficulty":"0xd","number":"0x9","gasLimit":"0xb","gasUsed":"0xa","timestamp":"0xc","extraData":"0x010203","mixHash":"0x000000000000000000000000000000000000000000000000000000000000000e","nonce":"0x000000000000000f","baseFeePerGas":"0x14","withdrawalsRoot":"0x0000000000000000000000000000000000000000000000000000000000000008","totalDifficulty":"0x186a0","uncles":[]}"#
        );
        let deserialized: Block = serde_json::from_str(&serialized).unwrap();
        assert_eq!(block, deserialized);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_block_with_withdrawals_set_as_none() {
        let block = Block {
            header: Header {
                hash: B256::with_last_byte(1),
                inner: alloy_consensus::Header {
                    parent_hash: B256::with_last_byte(2),
                    ommers_hash: B256::with_last_byte(3),
                    beneficiary: Address::with_last_byte(4),
                    state_root: B256::with_last_byte(5),
                    transactions_root: B256::with_last_byte(6),
                    receipts_root: B256::with_last_byte(7),
                    withdrawals_root: None,
                    number: 9,
                    gas_used: 10,
                    gas_limit: 11,
                    extra_data: vec![1, 2, 3].into(),
                    logs_bloom: Bloom::default(),
                    timestamp: 12,
                    difficulty: U256::from(13),
                    mix_hash: B256::with_last_byte(14),
                    nonce: B64::with_last_byte(15),
                    base_fee_per_gas: Some(20),
                    blob_gas_used: None,
                    excess_blob_gas: None,
                    parent_beacon_block_root: None,
                    requests_hash: None,
                },
                total_difficulty: Some(U256::from(100000)),
                size: None,
            },
            uncles: vec![B256::with_last_byte(17)],
            transactions: vec![B256::with_last_byte(18)].into(),
            withdrawals: None,
        };
        let serialized = serde_json::to_string(&block).unwrap();
        assert_eq!(
            serialized,
            r#"{"hash":"0x0000000000000000000000000000000000000000000000000000000000000001","parentHash":"0x0000000000000000000000000000000000000000000000000000000000000002","sha3Uncles":"0x0000000000000000000000000000000000000000000000000000000000000003","miner":"0x0000000000000000000000000000000000000004","stateRoot":"0x0000000000000000000000000000000000000000000000000000000000000005","transactionsRoot":"0x0000000000000000000000000000000000000000000000000000000000000006","receiptsRoot":"0x0000000000000000000000000000000000000000000000000000000000000007","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","difficulty":"0xd","number":"0x9","gasLimit":"0xb","gasUsed":"0xa","timestamp":"0xc","extraData":"0x010203","mixHash":"0x000000000000000000000000000000000000000000000000000000000000000e","nonce":"0x000000000000000f","baseFeePerGas":"0x14","totalDifficulty":"0x186a0","uncles":["0x0000000000000000000000000000000000000000000000000000000000000011"],"transactions":["0x0000000000000000000000000000000000000000000000000000000000000012"]}"#
        );
        let deserialized: Block = serde_json::from_str(&serialized).unwrap();
        assert_eq!(block, deserialized);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn block_overrides() {
        let s = r#"{"blockNumber": "0xe39dd0"}"#;
        let _overrides = serde_json::from_str::<BlockOverrides>(s).unwrap();
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_rich_block() {
        let s = r#"{
    "hash": "0xb25d0e54ca0104e3ebfb5a1dcdf9528140854d609886a300946fd6750dcb19f4",
    "parentHash": "0x9400ec9ef59689c157ac89eeed906f15ddd768f94e1575e0e27d37c241439a5d",
    "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
    "miner": "0x829bd824b016326a401d083b33d092293333a830",
    "stateRoot": "0x546e330050c66d02923e7f1f3e925efaf64e4384eeecf2288f40088714a77a84",
    "transactionsRoot": "0xd5eb3ad6d7c7a4798cc5fb14a6820073f44a941107c5d79dac60bd16325631fe",
    "receiptsRoot": "0xb21c41cbb3439c5af25304e1405524c885e733b16203221900cb7f4b387b62f0",
    "logsBloom": "0x1f304e641097eafae088627298685d20202004a4a59e4d8900914724e2402b028c9d596660581f361240816e82d00fa14250c9ca89840887a381efa600288283d170010ab0b2a0694c81842c2482457e0eb77c2c02554614007f42aaf3b4dc15d006a83522c86a240c06d241013258d90540c3008888d576a02c10120808520a2221110f4805200302624d22092b2c0e94e849b1e1aa80bc4cc3206f00b249d0a603ee4310216850e47c8997a20aa81fe95040a49ca5a420464600e008351d161dc00d620970b6a801535c218d0b4116099292000c08001943a225d6485528828110645b8244625a182c1a88a41087e6d039b000a180d04300d0680700a15794",
    "difficulty": "0xc40faff9c737d",
    "number": "0xa9a230",
    "gasLimit": "0xbe5a66",
    "gasUsed": "0xbe0fcc",
    "timestamp": "0x5f93b749",
    "totalDifficulty": "0x3dc957fd8167fb2684a",
    "extraData": "0x7070796520e4b883e5bda9e7a59ee4bb99e9b1bc0103",
    "mixHash": "0xd5e2b7b71fbe4ddfe552fb2377bf7cddb16bbb7e185806036cee86994c6e97fc",
    "nonce": "0x4722f2acd35abe0f",
    "uncles": [],
    "transactions": [
        "0xf435a26acc2a9ef73ac0b73632e32e29bd0e28d5c4f46a7e18ed545c93315916"
    ],
    "size": "0xaeb6"
}"#;

        let block = serde_json::from_str::<alloy_serde::WithOtherFields<Block>>(s).unwrap();
        let serialized = serde_json::to_string(&block).unwrap();
        let block2 =
            serde_json::from_str::<alloy_serde::WithOtherFields<Block>>(&serialized).unwrap();
        assert_eq!(block, block2);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_missing_uncles_block() {
        let s = r#"{
            "baseFeePerGas":"0x886b221ad",
            "blobGasUsed":"0x0",
            "difficulty":"0x0",
            "excessBlobGas":"0x0",
            "extraData":"0x6265617665726275696c642e6f7267",
            "gasLimit":"0x1c9c380",
            "gasUsed":"0xb0033c",
            "hash":"0x85cdcbe36217fd57bf2c33731d8460657a7ce512401f49c9f6392c82a7ccf7ac",
            "logsBloom":"0xc36919406572730518285284f2293101104140c0d42c4a786c892467868a8806f40159d29988002870403902413a1d04321320308da2e845438429e0012a00b419d8ccc8584a1c28f82a415d04eab8a5ae75c00d07761acf233414c08b6d9b571c06156086c70ea5186e9b989b0c2d55c0213c936805cd2ab331589c90194d070c00867549b1e1be14cb24500b0386cd901197c1ef5a00da453234fa48f3003dcaa894e3111c22b80e17f7d4388385a10720cda1140c0400f9e084ca34fc4870fb16b472340a2a6a63115a82522f506c06c2675080508834828c63defd06bc2331b4aa708906a06a560457b114248041e40179ebc05c6846c1e922125982f427",
            "miner":"0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5",
            "mixHash":"0x4c068e902990f21f92a2456fc75c59bec8be03b7f13682b6ebd27da56269beb5",
            "nonce":"0x0000000000000000",
            "number":"0x128c6df",
            "parentBeaconBlockRoot":"0x2843cb9f7d001bd58816a915e685ed96a555c9aeec1217736bd83a96ebd409cc",
            "parentHash":"0x90926e0298d418181bd20c23b332451e35fd7d696b5dcdc5a3a0a6b715f4c717",
            "receiptsRoot":"0xd43aa19ecb03571d1b86d89d9bb980139d32f2f2ba59646cd5c1de9e80c68c90",
            "sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            "size":"0xdcc3",
            "stateRoot":"0x707875120a7103621fb4131df59904cda39de948dfda9084a1e3da44594d5404",
            "timestamp":"0x65f5f4c3",
            "transactionsRoot":"0x889a1c26dc42ba829dab552b779620feac231cde8a6c79af022bdc605c23a780",
            "withdrawals":[
               {
                  "index":"0x24d80e6",
                  "validatorIndex":"0x8b2b6",
                  "address":"0x7cd1122e8e118b12ece8d25480dfeef230da17ff",
                  "amount":"0x1161f10"
               }
            ],
            "withdrawalsRoot":"0x360c33f20eeed5efbc7d08be46e58f8440af5db503e40908ef3d1eb314856ef7"
         }"#;

        let block = serde_json::from_str::<Block>(s).unwrap();
        let serialized = serde_json::to_string(&block).unwrap();
        let block2 = serde_json::from_str::<Block>(&serialized).unwrap();
        assert_eq!(block, block2);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_block_containing_uncles() {
        let s = r#"{
            "baseFeePerGas":"0x886b221ad",
            "blobGasUsed":"0x0",
            "difficulty":"0x0",
            "excessBlobGas":"0x0",
            "extraData":"0x6265617665726275696c642e6f7267",
            "gasLimit":"0x1c9c380",
            "gasUsed":"0xb0033c",
            "hash":"0x85cdcbe36217fd57bf2c33731d8460657a7ce512401f49c9f6392c82a7ccf7ac",
            "logsBloom":"0xc36919406572730518285284f2293101104140c0d42c4a786c892467868a8806f40159d29988002870403902413a1d04321320308da2e845438429e0012a00b419d8ccc8584a1c28f82a415d04eab8a5ae75c00d07761acf233414c08b6d9b571c06156086c70ea5186e9b989b0c2d55c0213c936805cd2ab331589c90194d070c00867549b1e1be14cb24500b0386cd901197c1ef5a00da453234fa48f3003dcaa894e3111c22b80e17f7d4388385a10720cda1140c0400f9e084ca34fc4870fb16b472340a2a6a63115a82522f506c06c2675080508834828c63defd06bc2331b4aa708906a06a560457b114248041e40179ebc05c6846c1e922125982f427",
            "miner":"0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5",
            "mixHash":"0x4c068e902990f21f92a2456fc75c59bec8be03b7f13682b6ebd27da56269beb5",
            "nonce":"0x0000000000000000",
            "number":"0x128c6df",
            "parentBeaconBlockRoot":"0x2843cb9f7d001bd58816a915e685ed96a555c9aeec1217736bd83a96ebd409cc",
            "parentHash":"0x90926e0298d418181bd20c23b332451e35fd7d696b5dcdc5a3a0a6b715f4c717",
            "receiptsRoot":"0xd43aa19ecb03571d1b86d89d9bb980139d32f2f2ba59646cd5c1de9e80c68c90",
            "sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            "size":"0xdcc3",
            "stateRoot":"0x707875120a7103621fb4131df59904cda39de948dfda9084a1e3da44594d5404",
            "timestamp":"0x65f5f4c3",
            "transactionsRoot":"0x889a1c26dc42ba829dab552b779620feac231cde8a6c79af022bdc605c23a780",
            "uncles": ["0x123a1c26dc42ba829dab552b779620feac231cde8a6c79af022bdc605c23a780", "0x489a1c26dc42ba829dab552b779620feac231cde8a6c79af022bdc605c23a780"],
            "withdrawals":[
               {
                  "index":"0x24d80e6",
                  "validatorIndex":"0x8b2b6",
                  "address":"0x7cd1122e8e118b12ece8d25480dfeef230da17ff",
                  "amount":"0x1161f10"
               }
            ],
            "withdrawalsRoot":"0x360c33f20eeed5efbc7d08be46e58f8440af5db503e40908ef3d1eb314856ef7"
         }"#;

        let block = serde_json::from_str::<Block>(s).unwrap();
        assert_eq!(block.uncles.len(), 2);
        let serialized = serde_json::to_string(&block).unwrap();
        let block2 = serde_json::from_str::<Block>(&serialized).unwrap();
        assert_eq!(block, block2);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_empty_block() {
        let s = r#"{
    "hash": "0xb25d0e54ca0104e3ebfb5a1dcdf9528140854d609886a300946fd6750dcb19f4",
    "parentHash": "0x9400ec9ef59689c157ac89eeed906f15ddd768f94e1575e0e27d37c241439a5d",
    "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
    "miner": "0x829bd824b016326a401d083b33d092293333a830",
    "stateRoot": "0x546e330050c66d02923e7f1f3e925efaf64e4384eeecf2288f40088714a77a84",
    "transactionsRoot": "0xd5eb3ad6d7c7a4798cc5fb14a6820073f44a941107c5d79dac60bd16325631fe",
    "receiptsRoot": "0xb21c41cbb3439c5af25304e1405524c885e733b16203221900cb7f4b387b62f0",
    "logsBloom": "0x1f304e641097eafae088627298685d20202004a4a59e4d8900914724e2402b028c9d596660581f361240816e82d00fa14250c9ca89840887a381efa600288283d170010ab0b2a0694c81842c2482457e0eb77c2c02554614007f42aaf3b4dc15d006a83522c86a240c06d241013258d90540c3008888d576a02c10120808520a2221110f4805200302624d22092b2c0e94e849b1e1aa80bc4cc3206f00b249d0a603ee4310216850e47c8997a20aa81fe95040a49ca5a420464600e008351d161dc00d620970b6a801535c218d0b4116099292000c08001943a225d6485528828110645b8244625a182c1a88a41087e6d039b000a180d04300d0680700a15794",
    "difficulty": "0xc40faff9c737d",
    "number": "0xa9a230",
    "gasLimit": "0xbe5a66",
    "gasUsed": "0xbe0fcc",
    "timestamp": "0x5f93b749",
    "totalDifficulty": "0x3dc957fd8167fb2684a",
    "extraData": "0x7070796520e4b883e5bda9e7a59ee4bb99e9b1bc0103",
    "mixHash": "0xd5e2b7b71fbe4ddfe552fb2377bf7cddb16bbb7e185806036cee86994c6e97fc",
    "nonce": "0x4722f2acd35abe0f",
    "uncles": [],
    "transactions": [],
    "size": "0xaeb6"
}"#;

        let block = serde_json::from_str::<Block>(s).unwrap();
        assert!(block.transactions.is_empty());
        assert!(block.transactions.as_transactions().is_some());
    }

    #[test]
    #[cfg(feature = "serde")]
    fn recompute_block_hash() {
        let s = r#"{
    "hash": "0xb25d0e54ca0104e3ebfb5a1dcdf9528140854d609886a300946fd6750dcb19f4",
    "parentHash": "0x9400ec9ef59689c157ac89eeed906f15ddd768f94e1575e0e27d37c241439a5d",
    "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
    "miner": "0x829bd824b016326a401d083b33d092293333a830",
    "stateRoot": "0x546e330050c66d02923e7f1f3e925efaf64e4384eeecf2288f40088714a77a84",
    "transactionsRoot": "0xd5eb3ad6d7c7a4798cc5fb14a6820073f44a941107c5d79dac60bd16325631fe",
    "receiptsRoot": "0xb21c41cbb3439c5af25304e1405524c885e733b16203221900cb7f4b387b62f0",
    "logsBloom": "0x1f304e641097eafae088627298685d20202004a4a59e4d8900914724e2402b028c9d596660581f361240816e82d00fa14250c9ca89840887a381efa600288283d170010ab0b2a0694c81842c2482457e0eb77c2c02554614007f42aaf3b4dc15d006a83522c86a240c06d241013258d90540c3008888d576a02c10120808520a2221110f4805200302624d22092b2c0e94e849b1e1aa80bc4cc3206f00b249d0a603ee4310216850e47c8997a20aa81fe95040a49ca5a420464600e008351d161dc00d620970b6a801535c218d0b4116099292000c08001943a225d6485528828110645b8244625a182c1a88a41087e6d039b000a180d04300d0680700a15794",
    "difficulty": "0xc40faff9c737d",
    "number": "0xa9a230",
    "gasLimit": "0xbe5a66",
    "gasUsed": "0xbe0fcc",
    "timestamp": "0x5f93b749",
    "totalDifficulty": "0x3dc957fd8167fb2684a",
    "extraData": "0x7070796520e4b883e5bda9e7a59ee4bb99e9b1bc0103",
    "mixHash": "0xd5e2b7b71fbe4ddfe552fb2377bf7cddb16bbb7e185806036cee86994c6e97fc",
    "nonce": "0x4722f2acd35abe0f",
    "uncles": [],
    "transactions": [],
    "size": "0xaeb6"
}"#;
        let block = serde_json::from_str::<Block>(s).unwrap();
        let header = block.clone().header.inner;
        let recomputed_hash = keccak256(alloy_rlp::encode(&header));
        assert_eq!(recomputed_hash, block.header.hash);

        let s2 = r#"{
            "baseFeePerGas":"0x886b221ad",
            "blobGasUsed":"0x0",
            "difficulty":"0x0",
            "excessBlobGas":"0x0",
            "extraData":"0x6265617665726275696c642e6f7267",
            "gasLimit":"0x1c9c380",
            "gasUsed":"0xb0033c",
            "hash":"0x85cdcbe36217fd57bf2c33731d8460657a7ce512401f49c9f6392c82a7ccf7ac",
            "logsBloom":"0xc36919406572730518285284f2293101104140c0d42c4a786c892467868a8806f40159d29988002870403902413a1d04321320308da2e845438429e0012a00b419d8ccc8584a1c28f82a415d04eab8a5ae75c00d07761acf233414c08b6d9b571c06156086c70ea5186e9b989b0c2d55c0213c936805cd2ab331589c90194d070c00867549b1e1be14cb24500b0386cd901197c1ef5a00da453234fa48f3003dcaa894e3111c22b80e17f7d4388385a10720cda1140c0400f9e084ca34fc4870fb16b472340a2a6a63115a82522f506c06c2675080508834828c63defd06bc2331b4aa708906a06a560457b114248041e40179ebc05c6846c1e922125982f427",
            "miner":"0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5",
            "mixHash":"0x4c068e902990f21f92a2456fc75c59bec8be03b7f13682b6ebd27da56269beb5",
            "nonce":"0x0000000000000000",
            "number":"0x128c6df",
            "parentBeaconBlockRoot":"0x2843cb9f7d001bd58816a915e685ed96a555c9aeec1217736bd83a96ebd409cc",
            "parentHash":"0x90926e0298d418181bd20c23b332451e35fd7d696b5dcdc5a3a0a6b715f4c717",
            "receiptsRoot":"0xd43aa19ecb03571d1b86d89d9bb980139d32f2f2ba59646cd5c1de9e80c68c90",
            "sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            "size":"0xdcc3",
            "stateRoot":"0x707875120a7103621fb4131df59904cda39de948dfda9084a1e3da44594d5404",
            "timestamp":"0x65f5f4c3",
            "transactionsRoot":"0x889a1c26dc42ba829dab552b779620feac231cde8a6c79af022bdc605c23a780",
            "withdrawals":[
               {
                  "index":"0x24d80e6",
                  "validatorIndex":"0x8b2b6",
                  "address":"0x7cd1122e8e118b12ece8d25480dfeef230da17ff",
                  "amount":"0x1161f10"
               }
            ],
            "withdrawalsRoot":"0x360c33f20eeed5efbc7d08be46e58f8440af5db503e40908ef3d1eb314856ef7"
         }"#;
        let block2 = serde_json::from_str::<Block>(s2).unwrap();
        let header = block2.clone().header.inner;
        let recomputed_hash = keccak256(alloy_rlp::encode(&header));
        assert_eq!(recomputed_hash, block2.header.hash);
    }

    #[test]
    fn header_roundtrip_conversion() {
        // Setup a RPC header
        let rpc_header = Header {
            hash: B256::with_last_byte(1),
            inner: alloy_consensus::Header {
                parent_hash: B256::with_last_byte(2),
                ommers_hash: B256::with_last_byte(3),
                beneficiary: Address::with_last_byte(4),
                state_root: B256::with_last_byte(5),
                transactions_root: B256::with_last_byte(6),
                receipts_root: B256::with_last_byte(7),
                withdrawals_root: None,
                number: 9,
                gas_used: 10,
                gas_limit: 11,
                extra_data: vec![1, 2, 3].into(),
                logs_bloom: Bloom::default(),
                timestamp: 12,
                difficulty: U256::from(13),
                mix_hash: B256::with_last_byte(14),
                nonce: B64::with_last_byte(15),
                base_fee_per_gas: Some(20),
                blob_gas_used: None,
                excess_blob_gas: None,
                parent_beacon_block_root: None,
                requests_hash: None,
            },
            size: None,
            total_difficulty: None,
        };

        // Convert the RPC header to a primitive header
        let primitive_header = rpc_header.clone().inner;

        // Seal the primitive header
        let sealed_header: Sealed<alloy_consensus::Header> =
            primitive_header.seal(B256::with_last_byte(1));

        // Convert the sealed header back to a RPC header
        let roundtrip_rpc_header = Header::from_consensus(sealed_header, None, None);

        // Ensure the roundtrip conversion is correct
        assert_eq!(rpc_header, roundtrip_rpc_header);
    }

    #[test]
    fn test_consensus_header_to_rpc_block() {
        // Setup a RPC header
        let header = Header {
            hash: B256::with_last_byte(1),
            inner: alloy_consensus::Header {
                parent_hash: B256::with_last_byte(2),
                ommers_hash: B256::with_last_byte(3),
                beneficiary: Address::with_last_byte(4),
                state_root: B256::with_last_byte(5),
                transactions_root: B256::with_last_byte(6),
                receipts_root: B256::with_last_byte(7),
                withdrawals_root: None,
                number: 9,
                gas_used: 10,
                gas_limit: 11,
                extra_data: vec![1, 2, 3].into(),
                logs_bloom: Bloom::default(),
                timestamp: 12,
                difficulty: U256::from(13),
                mix_hash: B256::with_last_byte(14),
                nonce: B64::with_last_byte(15),
                base_fee_per_gas: Some(20),
                blob_gas_used: None,
                excess_blob_gas: None,
                parent_beacon_block_root: None,
                requests_hash: None,
            },
            total_difficulty: None,
            size: Some(U256::from(505)),
        };

        // Convert the RPC header to a primitive header
        let primitive_header = header.clone().inner;

        // Convert the primitive header to a RPC uncle block
        let block: Block<Transaction> = Block::uncle_from_header(primitive_header);

        // Ensure the block is correct
        assert_eq!(
            block,
            Block {
                header: Header {
                    hash: B256::from(hex!(
                        "379bd1414cf69a9b86fb4e0e6b05a2e4b14cb3d5af057e13ccdc2192cb9780b2"
                    )),
                    ..header
                },
                uncles: vec![],
                transactions: BlockTransactions::Uncle,
                withdrawals: None,
            }
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_bad_block() {
        use alloy_primitives::B64;

        let block = Block {
            header: Header {
                hash: B256::with_last_byte(1),
                inner: alloy_consensus::Header {
                    parent_hash: B256::with_last_byte(2),
                    ommers_hash: B256::with_last_byte(3),
                    beneficiary: Address::with_last_byte(4),
                    state_root: B256::with_last_byte(5),
                    transactions_root: B256::with_last_byte(6),
                    receipts_root: B256::with_last_byte(7),
                    withdrawals_root: Some(B256::with_last_byte(8)),
                    number: 9,
                    gas_used: 10,
                    gas_limit: 11,
                    extra_data: vec![1, 2, 3].into(),
                    logs_bloom: Default::default(),
                    timestamp: 12,
                    difficulty: U256::from(13),
                    mix_hash: B256::with_last_byte(14),
                    nonce: B64::with_last_byte(15),
                    base_fee_per_gas: Some(20),
                    blob_gas_used: None,
                    excess_blob_gas: None,
                    parent_beacon_block_root: None,
                    requests_hash: None,
                },
                total_difficulty: Some(U256::from(100000)),
                size: Some(U256::from(19)),
            },
            uncles: vec![B256::with_last_byte(17)],
            transactions: vec![B256::with_last_byte(18)].into(),
            withdrawals: Some(Default::default()),
        };
        let hash = block.header.hash;
        let rlp = Bytes::from("header");

        let bad_block = BadBlock { block, hash, rlp };

        let serialized = serde_json::to_string(&bad_block).unwrap();
        assert_eq!(
            serialized,
            r#"{"block":{"hash":"0x0000000000000000000000000000000000000000000000000000000000000001","parentHash":"0x0000000000000000000000000000000000000000000000000000000000000002","sha3Uncles":"0x0000000000000000000000000000000000000000000000000000000000000003","miner":"0x0000000000000000000000000000000000000004","stateRoot":"0x0000000000000000000000000000000000000000000000000000000000000005","transactionsRoot":"0x0000000000000000000000000000000000000000000000000000000000000006","receiptsRoot":"0x0000000000000000000000000000000000000000000000000000000000000007","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","difficulty":"0xd","number":"0x9","gasLimit":"0xb","gasUsed":"0xa","timestamp":"0xc","extraData":"0x010203","mixHash":"0x000000000000000000000000000000000000000000000000000000000000000e","nonce":"0x000000000000000f","baseFeePerGas":"0x14","withdrawalsRoot":"0x0000000000000000000000000000000000000000000000000000000000000008","totalDifficulty":"0x186a0","size":"0x13","uncles":["0x0000000000000000000000000000000000000000000000000000000000000011"],"transactions":["0x0000000000000000000000000000000000000000000000000000000000000012"],"withdrawals":[]},"hash":"0x0000000000000000000000000000000000000000000000000000000000000001","rlp":"0x686561646572"}"#
        );

        let deserialized: BadBlock = serde_json::from_str(&serialized).unwrap();
        assert_eq!(bad_block, deserialized);
    }

    // <https://github.com/succinctlabs/kona/issues/31>
    #[test]
    fn deserde_tenderly_block() {
        let s = include_str!("../testdata/tenderly.sepolia.json");
        let _block: Block = serde_json::from_str(s).unwrap();
    }
}
