//! Cast is a Swiss Army knife for interacting with Ethereum applications from the command line.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![recursion_limit = "256"]

#[macro_use]
extern crate foundry_common;
#[macro_use]
extern crate tracing;

use alloy_consensus::{
    BlockHeader,
    transaction::{Recovered, SignerRecoverable},
};
use alloy_dyn_abi::{DynSolType, DynSolValue, Specifier};
use alloy_eips::Encodable2718;
use alloy_network::{AnyNetwork, BlockResponse, Network};
use alloy_primitives::{
    Address, B256, I256, Keccak256, LogData, Selector, TxHash, U64, U256, hex,
    utils::{ParseUnits, Unit, keccak256},
};
use alloy_provider::{Provider, network::eip2718::Decodable2718};
use alloy_rlp::{Decodable, Encodable};
use base::{Base, NumberWithBase};
use eyre::{Context, ContextCompat, OptionExt, Result};
use foundry_block_explorers::Client;
use foundry_common::{
    abi::{encode_function_args, encode_function_args_packed, get_event, get_func},
    compile::etherscan_project,
    flatten,
    fmt::*,
    fs, shell,
};
use foundry_config::Chain;
use foundry_evm::core::bytecode::InstIter;
#[cfg(feature = "optimism")]
use op_alloy_consensus as _;

use rayon::prelude::*;
use serde::Serialize;
use std::{
    fmt::Write,
    path::PathBuf,
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
};

pub use foundry_evm::*;

pub mod args;
pub mod cmd;
pub mod opts;
pub mod tempo;

pub mod base;
pub mod call_spec;
pub(crate) mod debug;
mod rlp_converter;
pub mod rpc_trace;
pub mod tx;

use rlp_converter::Item;

const MAX_CONCURRENT_RPC_REQUESTS: usize = 5;

pub(crate) fn strip_0x(s: &str) -> &str {
    s.strip_prefix("0x").unwrap_or(s)
}

/// Encodes the topic of an indexed event parameter.
///
/// Value types are encoded as their 32-byte word. Reference types are hashed over the special
/// in-place encoding defined for indexed event parameters, which differs from regular ABI
/// encoding: `string` and `bytes` contribute their raw contents, and array or struct members are
/// concatenated recursively without any offsets or length prefixes.
///
/// See <https://docs.soliditylang.org/en/latest/abi-spec.html#encoding-of-indexed-event-parameters>
pub(crate) fn encode_event_topic(value: &DynSolValue) -> B256 {
    if let Some(word) = value.as_word() {
        return word;
    }
    // Top-level `string` and `bytes` hash their raw contents without padding.
    if let Some(bytes) = value.as_packed_seq() {
        return keccak256(bytes);
    }
    let mut preimage = Vec::new();
    encode_event_topic_preimage(value, &mut preimage);
    keccak256(preimage)
}

/// Encodes a value into the in-place preimage of an indexed event parameter: words as-is,
/// `string`/`bytes` right-padded to a multiple of 32 bytes, and sequences as the concatenation of
/// their encoded members.
fn encode_event_topic_preimage(value: &DynSolValue, out: &mut Vec<u8>) {
    if let Some(word) = value.as_word() {
        out.extend_from_slice(word.as_slice());
    } else if let Some(bytes) = value.as_packed_seq() {
        let pad = bytes.len().next_multiple_of(32) - bytes.len();
        out.extend_from_slice(bytes);
        out.resize(out.len() + pad, 0);
    } else if let Some(values) = value.as_fixed_seq().or_else(|| value.as_array()) {
        for value in values {
            encode_event_topic_preimage(value, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DynSolValue, SimpleCast as Cast, serialize_value_as_json};
    use alloy_primitives::{U256, hex};

    /// Compares [`super::encode_event_topic`] against alloy's static [`EventTopic`]
    /// implementation, which `sol!`-generated events use to compute indexed topics.
    #[test]
    fn encode_event_topic_matches_static_encoding() {
        use alloy_primitives::{Address, Bytes, U256};
        use alloy_sol_types::{EventTopic, sol_data};

        let uint = |n: u64| DynSolValue::Uint(U256::from(n), 256);
        let string = |s: &str| DynSolValue::String(s.into());
        let topic = |v: &DynSolValue| super::encode_event_topic(v);

        let long = "abcdefghijklmnopqrstuvwxyz0123456789abcd";
        for s in ["", "hello", long] {
            assert_eq!(
                topic(&string(s)),
                <sol_data::String as EventTopic>::encode_topic(&s.to_string()).0,
                "string {s:?}"
            );
        }

        let bytes = hex::decode("deadbeef").unwrap();
        assert_eq!(
            topic(&DynSolValue::Bytes(bytes.clone())),
            <sol_data::Bytes as EventTopic>::encode_topic(&Bytes::from(bytes)).0,
        );

        let addr = Address::repeat_byte(0x42);
        assert_eq!(
            topic(&DynSolValue::Address(addr)),
            <sol_data::Address as EventTopic>::encode_topic(&addr).0,
        );

        assert_eq!(
            topic(&DynSolValue::Array(vec![uint(1), uint(2)])),
            <sol_data::Array<sol_data::Uint<256>> as EventTopic>::encode_topic(&vec![
                U256::from(1),
                U256::from(2)
            ])
            .0,
        );

        assert_eq!(
            topic(&DynSolValue::FixedArray(vec![uint(7), uint(9)])),
            <sol_data::FixedArray<sol_data::Uint<256>, 2> as EventTopic>::encode_topic(&[
                U256::from(7),
                U256::from(9)
            ])
            .0,
        );

        assert_eq!(
            topic(&DynSolValue::Array(vec![string("alpha"), string(long)])),
            <sol_data::Array<sol_data::String> as EventTopic>::encode_topic(&vec![
                "alpha".to_string(),
                long.to_string()
            ])
            .0,
        );

        assert_eq!(
            topic(&DynSolValue::Tuple(vec![uint(7), string("hello")])),
            <(sol_data::Uint<256>, sol_data::String) as EventTopic>::encode_topic(&(
                U256::from(7),
                "hello".to_string()
            ))
            .0,
        );

        assert_eq!(
            topic(&DynSolValue::Array(vec![
                DynSolValue::Array(vec![uint(1)]),
                DynSolValue::Array(vec![uint(2), uint(3)]),
            ])),
            <sol_data::Array<sol_data::Array<sol_data::Uint<256>>> as EventTopic>::encode_topic(
                &vec![vec![U256::from(1)], vec![U256::from(2), U256::from(3)]]
            )
            .0,
        );
    }

    // <https://github.com/foundry-rs/foundry/issues/2681>
}
