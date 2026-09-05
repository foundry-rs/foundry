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

pub struct SimpleCast;

impl SimpleCast {
    /// Performs ABI encoding to produce the hexadecimal calldata with the given arguments.
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// assert_eq!(
    ///     "0xb3de648b0000000000000000000000000000000000000000000000000000000000000001",
    ///     Cast::calldata_encode("f(uint256 a)", &["1"]).unwrap().as_str()
    /// );
    /// # Ok::<_, eyre::Report>(())
    /// ```
    pub fn calldata_encode(sig: impl AsRef<str>, args: &[impl AsRef<str>]) -> Result<String> {
        let func = get_func(sig.as_ref())?;
        let calldata = encode_function_args(&func, args)?;
        Ok(hex::encode_prefixed(calldata))
    }

    /// Returns the slot number for a given mapping key and slot.
    ///
    /// Given `mapping(k => v) m`, for a key `k` the slot number of its associated `v` is
    /// `keccak256(concat(h(k), p))`, where `h` is the padding function for `k`'s type, and `p`
    /// is slot number of the mapping `m`.
    ///
    /// See [the Solidity documentation](https://docs.soliditylang.org/en/latest/internals/layout_in_storage.html#mappings-and-dynamic-arrays)
    /// for more details.
    ///
    /// # Example
    ///
    /// ```
    /// # use cast::SimpleCast as Cast;
    ///
    /// // Value types.
    /// assert_eq!(
    ///     Cast::index("address", "0xD0074F4E6490ae3f888d1d4f7E3E43326bD3f0f5", "2").unwrap().as_str(),
    ///     "0x9525a448a9000053a4d151336329d6563b7e80b24f8e628e95527f218e8ab5fb"
    /// );
    /// assert_eq!(
    ///     Cast::index("uint256", "42", "6").unwrap().as_str(),
    ///     "0xfc808b0f31a1e6b9cf25ff6289feae9b51017b392cc8e25620a94a38dcdafcc1"
    /// );
    ///
    /// // Strings and byte arrays.
    /// assert_eq!(
    ///     Cast::index("string", "hello", "1").unwrap().as_str(),
    ///     "0x8404bb4d805e9ca2bd5dd5c43a107e935c8ec393caa7851b353b3192cd5379ae"
    /// );
    /// # Ok::<_, eyre::Report>(())
    /// ```
    pub fn index(key_type: &str, key: &str, slot_number: &str) -> Result<String> {
        let mut hasher = Keccak256::new();

        let k_ty = DynSolType::parse(key_type).wrap_err("Could not parse type")?;
        let k = k_ty.coerce_str(key).wrap_err("Could not parse value")?;
        match k_ty {
            // For value types, `h` pads the value to 32 bytes in the same way as when storing the
            // value in memory.
            DynSolType::Bool
            | DynSolType::Int(_)
            | DynSolType::Uint(_)
            | DynSolType::FixedBytes(_)
            | DynSolType::Address
            | DynSolType::Function => hasher.update(k.as_word().unwrap()),

            // For strings and byte arrays, `h(k)` is just the unpadded data.
            DynSolType::String | DynSolType::Bytes => hasher.update(k.as_packed_seq().unwrap()),

            DynSolType::Array(..)
            | DynSolType::FixedArray(..)
            | DynSolType::Tuple(..)
            | DynSolType::CustomStruct { .. } => {
                eyre::bail!("Type `{k_ty}` is not supported as a mapping key");
            }
        }

        let p = DynSolType::Uint(256)
            .coerce_str(slot_number)
            .wrap_err("Could not parse slot number")?;
        let p = p.as_word().unwrap();
        hasher.update(p);

        let location = hasher.finalize();
        Ok(location.to_string())
    }

    /// Keccak-256 hashes arbitrary data
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// assert_eq!(
    ///     Cast::keccak("foo")?,
    ///     "0x41b1a0649752af1b28b3dc29a1556eee781e4a4c3a1f7f53f90fa834de098c4d"
    /// );
    /// assert_eq!(
    ///     Cast::keccak("123abc")?,
    ///     "0xb1f1c74a1ba56f07a892ea1110a39349d40f66ca01d245e704621033cb7046a4"
    /// );
    /// assert_eq!(
    ///     Cast::keccak("0x12")?,
    ///     "0x5fa2358263196dbbf23d1ca7a509451f7a2f64c15837bfbb81298b1e3e24e4fa"
    /// );
    /// assert_eq!(
    ///     Cast::keccak("12")?,
    ///     "0x7f8b6b088b6d74c2852fc86c796dca07b44eed6fb3daf5e6b59f7c364db14528"
    /// );
    /// # Ok::<_, eyre::Report>(())
    /// ```
    pub fn keccak(data: &str) -> Result<String> {
        // Hex-decode if data starts with 0x.
        let hash = if data.starts_with("0x") {
            keccak256(hex::decode(data.trim_end())?)
        } else {
            keccak256(data)
        };
        Ok(hash.to_string())
    }

    /// Performs the left shift operation (<<) on a number
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// assert_eq!(Cast::left_shift("16", "10", Some("10"), "hex")?, "0x4000");
    /// assert_eq!(Cast::left_shift("255", "16", Some("dec"), "hex")?, "0xff0000");
    /// assert_eq!(Cast::left_shift("0xff", "16", None, "hex")?, "0xff0000");
    /// # Ok::<_, eyre::Report>(())
    /// ```
    pub fn left_shift(
        value: &str,
        bits: &str,
        base_in: Option<&str>,
        base_out: &str,
    ) -> Result<String> {
        Self::shift(value, bits, base_in, base_out, |value, bits| value << bits)
    }

    /// Performs the right shift operation (>>) on a number
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// assert_eq!(Cast::right_shift("0x4000", "10", None, "dec")?, "16");
    /// assert_eq!(Cast::right_shift("16711680", "16", Some("10"), "hex")?, "0xff");
    /// assert_eq!(Cast::right_shift("0xff0000", "16", None, "hex")?, "0xff");
    /// # Ok::<(), eyre::Report>(())
    /// ```
    pub fn right_shift(
        value: &str,
        bits: &str,
        base_in: Option<&str>,
        base_out: &str,
    ) -> Result<String> {
        Self::shift(value, bits, base_in, base_out, |value, bits| {
            value.wrapping_shr(bits.saturating_to())
        })
    }

    /// Parses `value` and `bits`, applies `shift` and formats the result with the `base_out`
    /// prefix.
    fn shift(
        value: &str,
        bits: &str,
        base_in: Option<&str>,
        base_out: &str,
        shift: impl FnOnce(U256, U256) -> U256,
    ) -> Result<String> {
        let base_out = base_out.parse()?;
        let value = NumberWithBase::parse_uint(value, base_in)?.number();
        let bits = NumberWithBase::parse_uint(bits, None)?.number();
        Ok(format!("{:#?}", NumberWithBase::from(shift(value, bits)).with_base(base_out)))
    }

    /// Fetches source code of verified contracts from etherscan.
    ///
    /// # Example
    ///
    /// ```
    /// # use cast::SimpleCast as Cast;
    /// # use foundry_config::NamedChain;
    /// # async fn foo() -> eyre::Result<()> {
    /// assert_eq!(
    ///     "/*
    ///             - Bytecode Verification performed was compared on second iteration -
    ///             This file is part of the DAO.....",
    ///     Cast::etherscan_source(
    ///         NamedChain::Mainnet.into(),
    ///         "0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".to_string(),
    ///         Some("<etherscan_api_key>".to_string()),
    ///         None,
    ///         None
    ///     )
    ///     .await
    ///     .unwrap()
    ///     .as_str()
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub async fn etherscan_source(
        chain: Chain,
        contract_address: String,
        etherscan_api_key: Option<String>,
        explorer_api_url: Option<String>,
        explorer_url: Option<String>,
    ) -> Result<String> {
        let client = explorer_client(chain, etherscan_api_key, explorer_api_url, explorer_url)?;
        let metadata = client.contract_source_code(contract_address.parse()?).await?;
        Ok(metadata.source_code())
    }

    /// Fetches the source code of verified contracts from etherscan and expands the resulting
    /// files to a directory for easy perusal.
    ///
    /// # Example
    ///
    /// ```
    /// # use cast::SimpleCast as Cast;
    /// # use foundry_config::NamedChain;
    /// # use std::path::PathBuf;
    /// # async fn expand() -> eyre::Result<()> {
    /// Cast::expand_etherscan_source_to_directory(
    ///     NamedChain::Mainnet.into(),
    ///     "0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".to_string(),
    ///     Some("<etherscan_api_key>".to_string()),
    ///     PathBuf::from("output_dir"),
    ///     None,
    ///     None,
    /// )
    /// .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn expand_etherscan_source_to_directory(
        chain: Chain,
        contract_address: String,
        etherscan_api_key: Option<String>,
        output_directory: PathBuf,
        explorer_api_url: Option<String>,
        explorer_url: Option<String>,
    ) -> eyre::Result<()> {
        let client = explorer_client(chain, etherscan_api_key, explorer_api_url, explorer_url)?;
        let meta = client.contract_source_code(contract_address.parse()?).await?;
        let source_tree = meta.source_tree();
        source_tree.write_to(&output_directory)?;
        Ok(())
    }

    /// Fetches the source code of verified contracts from etherscan, flattens it and writes it to
    /// the given path or stdout.
    pub async fn etherscan_source_flatten(
        chain: Chain,
        contract_address: String,
        etherscan_api_key: Option<String>,
        output_path: Option<PathBuf>,
        explorer_api_url: Option<String>,
        explorer_url: Option<String>,
    ) -> Result<()> {
        let client = explorer_client(chain, etherscan_api_key, explorer_api_url, explorer_url)?;
        let metadata = client.contract_source_code(contract_address.parse()?).await?;
        let Some(metadata) = metadata.items.first() else {
            eyre::bail!("Empty contract source code");
        };

        let tmp = tempfile::tempdir()?;
        let project = etherscan_project(metadata, tmp.path())?;
        let target_path = project.find_contract_path(&metadata.contract_name)?;

        let flattened = flatten(project, &target_path)?;

        if let Some(path) = output_path {
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, flattened)?;
            sh_status!("Flattened file written at {}", path.display())?
        } else {
            sh_println!("{flattened}")?
        }

        Ok(())
    }

    /// Disassembles hex encoded bytecode into individual / human readable opcodes
    ///
    /// # Example
    ///
    /// ```
    /// use alloy_primitives::hex;
    /// use cast::SimpleCast as Cast;
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let bytecode = "0x608060405260043610603f57600035";
    /// let opcodes = Cast::disassemble(&hex::decode(bytecode)?)?;
    /// println!("{}", opcodes);
    /// # Ok(())
    /// # }
    /// ```
    pub fn disassemble(code: &[u8]) -> Result<String> {
        let mut output = String::new();
        for (pc, inst) in InstIter::new(code).with_pc() {
            writeln!(output, "{pc:08x}: {inst}")?;
        }
        Ok(output)
    }

    /// Gets the selector for a given function signature
    /// Optimizes if the `optimize` parameter is set to a number of leading zeroes
    ///
    /// # Example
    ///
    /// ```
    /// use cast::SimpleCast as Cast;
    ///
    /// assert_eq!(Cast::get_selector("foo()", 0)?.0, String::from("0xc2985578"));
    /// assert_eq!(Cast::get_selector("foo(address,uint256)", 0)?.0, String::from("0xbd0d639f"));
    /// # Ok::<(), eyre::Error>(())
    /// ```
    pub fn get_selector(signature: &str, optimize: usize) -> Result<(String, String)> {
        if optimize > 4 {
            eyre::bail!("number of leading zeroes must not be greater than 4");
        }
        if optimize == 0 {
            let selector = get_func(signature)?.selector();
            return Ok((selector.to_string(), String::from(signature)));
        }
        let Some((name, params)) = signature.split_once('(') else {
            eyre::bail!("invalid function signature");
        };

        let num_threads = rayon::current_num_threads();
        let found = AtomicBool::new(false);

        // Each thread walks its own residue class of nonces until one of them finds a match.
        (0..num_threads as u32)
            .into_par_iter()
            .find_map_any(|mut nonce| {
                while nonce < u32::MAX && !found.load(Ordering::Relaxed) {
                    let input = format!("{name}{nonce}({params}");
                    let selector = &keccak256(input.as_bytes())[..4];
                    if selector.iter().take_while(|&&byte| byte == 0).count() == optimize {
                        found.store(true, Ordering::Relaxed);
                        return Some((hex::encode_prefixed(selector), input));
                    }
                    nonce += num_threads as u32;
                }
                None
            })
            .ok_or_eyre("No selector found")
    }

    /// Extracts function selectors, arguments and state mutability from bytecode
    ///
    /// # Example
    ///
    /// ```
    /// use alloy_primitives::fixed_bytes;
    /// use cast::SimpleCast as Cast;
    ///
    /// let bytecode = "6080604052348015600e575f80fd5b50600436106026575f3560e01c80632125b65b14602a575b5f80fd5b603a6035366004603c565b505050565b005b5f805f60608486031215604d575f80fd5b833563ffffffff81168114605f575f80fd5b925060208401356001600160a01b03811681146079575f80fd5b915060408401356001600160e01b03811681146093575f80fd5b80915050925092509256";
    /// let functions = Cast::extract_functions(bytecode)?;
    /// assert_eq!(functions, vec![(fixed_bytes!("0x2125b65b"), "uint32,address,uint224".to_string(), "pure")]);
    /// # Ok::<(), eyre::Report>(())
    /// ```
    pub fn extract_functions(bytecode: &str) -> Result<Vec<(Selector, String, &str)>> {
        let code = hex::decode(bytecode)?;
        let info = evmole::contract_info(
            evmole::ContractInfoArgs::new(&code)
                .with_selectors()
                .with_arguments()
                .with_state_mutability(),
        );
        Ok(info
            .functions
            .expect("functions extraction was requested")
            .into_iter()
            .filter(|f| f.dispatch == evmole::SelectorDispatch::Abi)
            .map(|f| {
                let arguments = f
                    .arguments
                    .expect("arguments extraction was requested")
                    .iter()
                    .map(|t| t.sol_type_name())
                    .collect::<Vec<_>>()
                    .join(",");
                let mutability =
                    f.state_mutability.expect("state_mutability extraction was requested");
                (f.selector.into(), arguments, mutability.as_json_str())
            })
            .collect())
    }

    /// Decodes a raw EIP2718 transaction payload
    /// Returns details about the typed transaction and ECSDA signature components
    ///
    /// # Example
    ///
    /// ```
    /// use alloy_network::Ethereum;
    /// use cast::SimpleCast as Cast;
    ///
    /// let tx = "0x02f8f582a86a82058d8459682f008508351050808303fd84948e42f2f4101563bf679975178e880fd87d3efd4e80b884659ac74b00000000000000000000000080f0c1c49891dcfdd40b6e0f960f84e6042bcb6f000000000000000000000000b97ef9ef8734c71904d8002f8b6bc66dd9c48a6e00000000000000000000000000000000000000000000000000000000007ff4e20000000000000000000000000000000000000000000000000000000000000064c001a05d429597befe2835396206781b199122f2e8297327ed4a05483339e7a8b2022aa04c23a7f70fb29dda1b4ee342fb10a625e9b8ddc6a603fb4e170d4f6f37700cb8";
    /// let tx_envelope = Cast::decode_raw_transaction::<Ethereum>(&tx)?;
    /// # Ok::<(), eyre::Report>(())
    pub fn decode_raw_transaction<N: Network<TxEnvelope: SignerRecoverable + Serialize>>(
        tx: &str,
    ) -> Result<String> {
        let tx_hex = hex::decode(tx)?;
        let tx: N::TxEnvelope = Decodable2718::decode_2718(&mut tx_hex.as_slice())?;
        if let Ok(signer) = tx.recover_signer() {
            Ok(serde_json::to_string_pretty(&Recovered::new_unchecked(tx, signer))?)
        } else {
            Ok(serde_json::to_string_pretty(&tx)?)
        }
    }
}

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

fn explorer_client(
    chain: Chain,
    api_key: Option<String>,
    api_url: Option<String>,
    explorer_url: Option<String>,
) -> Result<Client> {
    let mut builder = Client::builder();

    let deduced = chain.etherscan_urls();

    let explorer_url = explorer_url
        .or(deduced.map(|d| d.1.to_string()))
        .ok_or_eyre("Please provide the explorer browser URL using `--explorer-url`")?;
    builder = builder.with_url(explorer_url)?;

    let api_url = api_url
        .or(deduced.map(|d| d.0.to_string()))
        .ok_or_eyre("Please provide the explorer API URL using `--explorer-api-url`")?;
    builder = builder.with_api_url(api_url)?;

    if let Some(api_key) = api_key {
        builder = builder.with_api_key(api_key);
    }

    builder.build().map_err(Into::into)
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
    #[test]
    fn calldata_array() {
        assert_eq!(
            "0xcde2baba0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000",
            Cast::calldata_encode("propose(string[])", &["[\"\"]"]).unwrap().as_str()
        );
    }

    #[test]
    fn calldata_bool() {
        assert_eq!(
            "0x6fae94120000000000000000000000000000000000000000000000000000000000000000",
            Cast::calldata_encode("bar(bool)", &["false"]).unwrap().as_str()
        );
    }

    #[test]
    fn disassemble_incomplete_sequence() {
        let incomplete = &hex!("60"); // PUSH1
        let disassembled = Cast::disassemble(incomplete).unwrap();
        assert_eq!(disassembled, "00000000: PUSH1\n");

        let complete = &hex!("6000"); // PUSH1 0x00
        let disassembled = Cast::disassemble(complete).unwrap();
        assert_eq!(disassembled, "00000000: PUSH1 0x00\n");

        let incomplete = &hex!("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"); // PUSH32 with 31 bytes
        let disassembled = Cast::disassemble(incomplete).unwrap();
        assert_eq!(disassembled, "00000000: PUSH32\n");

        let complete = &hex!("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"); // PUSH32 with 32 bytes
        let disassembled = Cast::disassemble(complete).unwrap();
        assert_eq!(
            disassembled,
            "00000000: PUSH32 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\n"
        );
    }
}
