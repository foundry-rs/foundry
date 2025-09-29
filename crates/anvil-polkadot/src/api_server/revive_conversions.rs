use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::Address;
use alloy_rpc_types::{AccessList, SignedAuthorization, TransactionRequest};
use polkadot_sdk::{
    pallet_revive::evm::{
        AccessListEntry, AuthorizationListEntry, BlockNumberOrTagOrHash, BlockTag, Byte, Bytes,
        GenericTransaction, InputOrData,
    },
    sp_core,
};
use subxt::utils::{H160, H256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlloyU256(alloy_primitives::U256);

impl From<polkadot_sdk::sp_core::U256> for AlloyU256 {
    fn from(value: polkadot_sdk::sp_core::U256) -> Self {
        let mut bytes = [0u8; 32];
        value.write_as_big_endian(&mut bytes);
        Self(alloy_primitives::U256::from_be_bytes(bytes))
    }
}

impl AlloyU256 {
    pub fn inner(&self) -> alloy_primitives::U256 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubstrateU256(sp_core::U256);

impl From<alloy_primitives::U256> for SubstrateU256 {
    fn from(value: alloy_primitives::U256) -> Self {
        Self(sp_core::U256::from_big_endian(&value.to_be_bytes::<32>()))
    }
}

impl SubstrateU256 {
    pub fn inner(&self) -> sp_core::U256 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviveAddress(H160);

impl ReviveAddress {
    pub fn new(addr: H160) -> Self {
        Self(addr)
    }

    pub fn inner(&self) -> H160 {
        self.0
    }
}

impl From<Address> for ReviveAddress {
    fn from(addr: Address) -> Self {
        Self(H160::from_slice(addr.0.as_ref()))
    }
}

impl From<ReviveAddress> for Address {
    fn from(value: ReviveAddress) -> Self {
        Self(alloy_primitives::U160::from_be_bytes(*value.0.as_fixed_bytes()).into())
    }
}

#[derive(Debug, Clone)]
pub struct ReviveBlockId(BlockNumberOrTagOrHash);

impl ReviveBlockId {
    pub fn inner(self) -> BlockNumberOrTagOrHash {
        self.0
    }
}

impl From<Option<BlockId>> for ReviveBlockId {
    fn from(block_id: Option<BlockId>) -> Self {
        Self(block_id.map_or(
            BlockNumberOrTagOrHash::BlockTag(BlockTag::Latest),
            |b_id| match b_id {
                BlockId::Hash(rpc_hash) => BlockNumberOrTagOrHash::BlockHash(H256::from_slice(
                    rpc_hash.block_hash.as_slice(),
                )),
                BlockId::Number(number_or_tag) => match number_or_tag {
                    BlockNumberOrTag::Number(num) => BlockNumberOrTagOrHash::BlockNumber(
                        polkadot_sdk::pallet_revive::U256::from(num),
                    ),
                    BlockNumberOrTag::Latest => BlockNumberOrTagOrHash::BlockTag(BlockTag::Latest),
                    BlockNumberOrTag::Earliest => {
                        BlockNumberOrTagOrHash::BlockTag(BlockTag::Earliest)
                    }
                    BlockNumberOrTag::Pending => {
                        BlockNumberOrTagOrHash::BlockTag(BlockTag::Pending)
                    }
                    BlockNumberOrTag::Safe => BlockNumberOrTagOrHash::BlockTag(BlockTag::Safe),
                    BlockNumberOrTag::Finalized => {
                        BlockNumberOrTagOrHash::BlockTag(BlockTag::Finalized)
                    }
                },
            },
        ))
    }
}

#[derive(Debug, Clone)]
pub struct ReviveAccessList(Vec<AccessListEntry>);

impl ReviveAccessList {
    pub fn inner(self) -> Vec<AccessListEntry> {
        self.0
    }
}

impl From<AccessList> for ReviveAccessList {
    fn from(value: AccessList) -> Self {
        Self(
            value
                .0
                .into_iter()
                .map(|access_list_entry| AccessListEntry {
                    address: ReviveAddress::from(access_list_entry.address).inner(),
                    storage_keys: access_list_entry
                        .storage_keys
                        .into_iter()
                        .map(|key| H256::from_slice(key.as_ref()))
                        .collect(),
                })
                .collect(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct ReviveAuthorizationListEntry(AuthorizationListEntry);

impl ReviveAuthorizationListEntry {
    pub fn inner(self) -> AuthorizationListEntry {
        self.0
    }
}

impl From<SignedAuthorization> for ReviveAuthorizationListEntry {
    fn from(value: SignedAuthorization) -> Self {
        Self(AuthorizationListEntry {
            chain_id: SubstrateU256::from(value.inner().chain_id).inner(),
            address: ReviveAddress::from(value.inner().address).inner(),
            nonce: value.inner().nonce.into(),
            y_parity: value.y_parity().into(),
            r: SubstrateU256::from(value.r()).inner(),
            s: SubstrateU256::from(value.s()).inner(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReviveBytes(Bytes);

impl From<alloy_primitives::Bytes> for ReviveBytes {
    fn from(value: alloy_primitives::Bytes) -> Self {
        Self(Bytes::from(value.to_vec()))
    }
}

impl ReviveBytes {
    pub fn inner(self) -> Bytes {
        self.0
    }
}

pub(crate) fn convert_to_generic_transaction(
    transaction_request: TransactionRequest,
) -> GenericTransaction {
    GenericTransaction {
        access_list: transaction_request
            .access_list
            .map(|access_list| ReviveAccessList::from(access_list).inner()),
        authorization_list: transaction_request.authorization_list.map_or(
            Default::default(),
            |authorization_list| {
                authorization_list
                    .into_iter()
                    .map(|entry| ReviveAuthorizationListEntry::from(entry).inner())
                    .collect()
            },
        ),
        blob_versioned_hashes: transaction_request
            .blob_versioned_hashes
            .unwrap_or_default()
            .into_iter()
            .map(|b256| H256::from_slice(b256.as_ref()))
            .collect(),
        blobs: transaction_request
            .sidecar
            .unwrap_or_default()
            .blobs
            .into_iter()
            .map(|blob| Bytes::from(blob.0.to_vec()))
            .collect(),
        chain_id: transaction_request.chain_id.map(sp_core::U256::from),
        from: transaction_request.from.map(|addr| ReviveAddress::from(addr).inner()),
        gas: transaction_request.gas.map(sp_core::U256::from),
        gas_price: transaction_request.gas_price.map(sp_core::U256::from),
        input: InputOrData::from(
            ReviveBytes::from(transaction_request.input.into_input().unwrap_or_default()).inner(),
        ),
        max_fee_per_blob_gas: transaction_request.max_fee_per_blob_gas.map(sp_core::U256::from),
        max_fee_per_gas: transaction_request.max_fee_per_gas.map(sp_core::U256::from),
        max_priority_fee_per_gas: transaction_request
            .max_priority_fee_per_gas
            .map(sp_core::U256::from),
        nonce: transaction_request.nonce.map(sp_core::U256::from),
        to: transaction_request
            .to
            .and_then(|tx_kind| tx_kind.into_to())
            .map(|addr| ReviveAddress::from(addr).inner()),
        r#type: transaction_request.transaction_type.map(Byte::from),
        value: transaction_request.value.map(|value| SubstrateU256::from(value).inner()),
    }
}
