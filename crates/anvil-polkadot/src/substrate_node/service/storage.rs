use codec::{Decode, Encode};
use polkadot_sdk::{
    frame_support::BoundedVec,
    frame_system,
    pallet_balances::AccountData,
    parachains_common::{AccountId, Nonce},
    sp_core::ConstU32,
};
use substrate_runtime::{Balance, Hash};

#[derive(Encode, Decode)]
pub struct ReviveAccountInfo {
    pub account_type: AccountType,
    pub dust: u32,
}

#[derive(Encode, Decode)]
pub enum AccountType {
    Contract(ContractInfo),
    EOA,
}

#[derive(Encode, Decode)]
pub struct ContractInfo {
    pub trie_id: BoundedVec<u8, ConstU32<128>>,
    pub code_hash: Hash,
    pub storage_bytes: u32,
    pub storage_items: u32,
    pub storage_byte_deposit: Balance,
    pub storage_item_deposit: Balance,
    pub storage_base_deposit: Balance,
    pub immutable_data_len: u32,
}

#[derive(Encode, Decode)]
pub struct CodeInfo {
    pub owner: AccountId,
    #[codec(compact)]
    pub deposit: Balance,
    #[codec(compact)]
    pub refcount: u64,
    pub code_len: u32,
    pub code_type: ByteCodeType,
    pub behaviour_version: u32,
}

#[derive(Encode, Decode)]
pub enum ByteCodeType {
    Pvm,
    Evm,
}

pub type SystemAccountInfo = frame_system::AccountInfo<Nonce, AccountData<Balance>>;

pub mod well_known_keys {
    use codec::Encode;
    use polkadot_sdk::{
        parachains_common::AccountId,
        sp_core::{H160, H256, blake2_128, twox_128},
    };

    // Hex-encoded key: 0xc2261276cc9d1f8598ea4b6a74b15c2f57c875e4cff74148e4628f264b974c80
    pub const TOTAL_ISSUANCE: [u8; 32] = [
        194, 38, 18, 118, 204, 157, 31, 133, 152, 234, 75, 106, 116, 177, 92, 47, 87, 200, 117,
        228, 207, 247, 65, 72, 228, 98, 143, 38, 75, 151, 76, 128,
    ];

    // Hex-encoded key: 0x9527366927478e710d3f7fb77c6d1f89
    pub const CHAIN_ID: [u8; 16] = [
        149u8, 39u8, 54u8, 105u8, 39u8, 71u8, 142u8, 113u8, 13u8, 63u8, 127u8, 183u8, 124u8, 109u8,
        31u8, 137u8,
    ];

    // Hex-encoded key: 0xf0c365c3cf59d671eb72da0e7a4113c49f1f0515f462cdcf84e0f1d6045dfcbb
    pub const TIMESTAMP: [u8; 32] = [
        240, 195, 101, 195, 207, 89, 214, 113, 235, 114, 218, 14, 122, 65, 19, 196, 159, 31, 5, 21,
        244, 98, 205, 207, 132, 224, 241, 214, 4, 93, 252, 187,
    ];

    // Hex-encoded key: 0x26aa394eea5630e07c48ae0c9558cef702a5c1b19ab7a04f536c519aca4983ac
    // twox_128(b"System") ++ twox_128(b"Number")
    // corresponds to `System::Number` storage item in pallet-system
    pub const BLOCK_NUMBER_KEY: [u8; 32] = [
        38u8, 170u8, 57u8, 78u8, 234u8, 86u8, 48u8, 224u8, 124u8, 72u8, 174u8, 12u8, 149u8, 88u8,
        206u8, 247u8, 2u8, 165u8, 193u8, 177u8, 154u8, 183u8, 160u8, 79u8, 83u8, 108u8, 81u8,
        154u8, 202u8, 73u8, 131u8, 172u8,
    ];

    pub fn system_account_info(account_id: AccountId) -> Vec<u8> {
        let mut key = Vec::new();
        key.extend_from_slice(&twox_128("System".as_bytes()));
        key.extend_from_slice(&twox_128("Account".as_bytes()));
        key.extend_from_slice(&blake2_128(account_id.as_ref()));
        key.extend_from_slice(&account_id.encode());

        key
    }

    pub fn revive_account_info(address: H160) -> Vec<u8> {
        let mut key = Vec::new();
        key.extend_from_slice(&twox_128("Revive".as_bytes()));
        key.extend_from_slice(&twox_128("AccountInfoOf".as_bytes()));
        key.extend_from_slice(&address.encode());

        key
    }

    pub fn pristine_code(code_hash: H256) -> Vec<u8> {
        let mut key = Vec::new();
        key.extend_from_slice(&twox_128("Revive".as_bytes()));
        key.extend_from_slice(&twox_128("PristineCode".as_bytes()));
        key.extend_from_slice(&code_hash.encode());

        key
    }

    pub fn code_info(code_hash: H256) -> Vec<u8> {
        let mut key = Vec::new();
        key.extend_from_slice(&twox_128("Revive".as_bytes()));
        key.extend_from_slice(&twox_128("CodeInfoOf".as_bytes()));
        key.extend_from_slice(&code_hash.encode());

        key
    }
}
