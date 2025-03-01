//! [EIP-4788] constants.
//!
//! [EIP-4788]: https://eips.ethereum.org/EIPS/eip-4788

use alloy_primitives::{address, bytes, Address, Bytes};

/// The caller to be used when calling the EIP-4788 beacon roots contract at the beginning of the
/// block.
pub const SYSTEM_ADDRESS: Address = address!("fffffffffffffffffffffffffffffffffffffffe");

/// The address for the EIP-4788 beacon roots contract.
pub const BEACON_ROOTS_ADDRESS: Address = address!("000F3df6D732807Ef1319fB7B8bB8522d0Beac02");

/// The code for the EIP-4788 beacon roots contract.
pub static BEACON_ROOTS_CODE: Bytes = bytes!("3373fffffffffffffffffffffffffffffffffffffffe14604d57602036146024575f5ffd5b5f35801560495762001fff810690815414603c575f5ffd5b62001fff01545f5260205ff35b5f5ffd5b62001fff42064281555f359062001fff015500");
