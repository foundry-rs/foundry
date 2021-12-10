//! Hooks over Sputnik EVM execution which allow runtime logging and modification of chain state
//! from Solidity (cheatcodes).
pub mod memory_stackstate_owned;

pub mod cheatcode_handler;
use std::collections::HashMap;

pub use cheatcode_handler::CheatcodeHandler;

pub mod backend;

use ethers::types::{Address, H256, U256};
use sputnik::backend::{Backend, MemoryAccount, MemoryBackend};

#[derive(Clone, Debug, Default)]
/// Cheatcodes can be used to control the EVM context during setup or runtime,
/// which can be useful for simulations or specialized unit tests
pub struct Cheatcodes {
    /// The overriden block number
    pub block_number: Option<U256>,
    /// The overriden timestamp
    pub block_timestamp: Option<U256>,
    /// The overriden basefee
    pub block_base_fee_per_gas: Option<U256>,
    /// The overriden storage slots
    pub accounts: HashMap<Address, MemoryAccount>,
}

/// Extension trait over [`Backend`] which provides additional methods for interacting with the
/// state
pub trait BackendExt: Backend {
    fn set_storage(&mut self, address: Address, slot: H256, value: H256);
}

impl<'a> BackendExt for MemoryBackend<'a> {
    fn set_storage(&mut self, address: Address, slot: H256, value: H256) {
        let account = self.state_mut().entry(address).or_insert_with(Default::default);
        let slot = account.storage.entry(slot).or_insert_with(Default::default);
        *slot = value;
    }
}

ethers::contract::abigen!(
    HEVM,
    r#"[
            roll(uint256)
            warp(uint256)
            store(address,bytes32,bytes32)
            load(address,bytes32)(bytes32)
            ffi(string[])(bytes)
            addr(uint256)(address)
            sign(uint256,bytes32)(uint8,bytes32,bytes32)
            prank(address,address,bytes)(bool,bytes)
            deal(address,uint256)
            etch(address,bytes)
    ]"#,
);
pub use hevm_mod::HEVMCalls;

ethers::contract::abigen!(
    HevmConsole,
    r#"[
            event log(string)
            event logs                   (bytes)
            event log_address            (address)
            event log_bytes32            (bytes32)
            event log_int                (int)
            event log_uint               (uint)
            event log_bytes              (bytes)
            event log_string             (string)
            event log_named_address      (string key, address val)
            event log_named_bytes32      (string key, bytes32 val)
            event log_named_decimal_int  (string key, int val, uint decimals)
            event log_named_decimal_uint (string key, uint val, uint decimals)
            event log_named_int          (string key, int val)
            event log_named_uint         (string key, uint val)
            event log_named_bytes        (string key, bytes val)
            event log_named_string       (string key, string val)
            ]"#,
);

ethers::contract::abigen!(Console, "./testdata/console.json",);

// Manually implement log(uint), see https://github.com/gakonst/foundry/issues/197
pub struct LogUint {
    pub p_0: ethers::core::types::U256,
}
impl ethers::core::abi::AbiType for LogUint {
    fn param_type() -> ethers::core::abi::ParamType {
        ethers::core::abi::ParamType::Tuple(<[_]>::into_vec(Box::new([
            <ethers::core::types::U256 as ethers::core::abi::AbiType>::param_type(),
        ])))
    }
}
impl ethers::core::abi::AbiArrayType for LogUint {}
impl ethers::core::abi::Tokenizable for LogUint
where
    ethers::core::types::U256: ethers::core::abi::Tokenize,
{
    fn from_token(
        token: ethers::core::abi::Token,
    ) -> Result<Self, ethers::core::abi::InvalidOutputType>
    where
        Self: Sized,
    {
        if let ethers::core::abi::Token::Tuple(tokens) = token {
            if tokens.len() != 1 {
                return Err(ethers::core::abi::InvalidOutputType(::std::format!(
                    "Expected {} tokens, got {}: {:?}",
                    1,
                    tokens.len(),
                    tokens
                )));
            }

            let mut iter = tokens.into_iter();
            Ok(Self {
                p_0: ethers::core::abi::Tokenizable::from_token(iter.next().unwrap())?,
            })
        } else {
            Err(ethers::core::abi::InvalidOutputType(::std::format!(
                "Expected Tuple, got {:?}",
                token
            )))
        }
    }
    fn into_token(self) -> ethers::core::abi::Token {
        ethers::core::abi::Token::Tuple(<[_]>::into_vec(Box::new([self.p_0.into_token()])))
    }
}
impl ethers::core::abi::TokenizableItem for LogUint where
    ethers::core::types::U256: ethers::core::abi::Tokenize
{
}
impl ethers::contract::EthCall for LogUint {
    fn function_name() -> ::std::borrow::Cow<'static, str> {
        "log".into()
    }
    fn selector() -> ethers::core::types::Selector {
        [245, 177, 187, 169]
    }
    fn abi_signature() -> ::std::borrow::Cow<'static, str> {
        "log(uint)".into()
    }
}
impl ethers::core::abi::AbiDecode for LogUint {
    fn decode(bytes: impl AsRef<[u8]>) -> Result<Self, ethers::core::abi::AbiError> {
        let bytes = bytes.as_ref();
        if bytes.len() < 4 || bytes[..4] != <Self as ethers::contract::EthCall>::selector() {
            return Err(ethers::contract::AbiError::WrongSelector);
        }
        let data_types = [ethers::core::abi::ParamType::Uint(256usize)];
        let data_tokens = ethers::core::abi::decode(&data_types, &bytes[4..])?;
        Ok(<Self as ethers::core::abi::Tokenizable>::from_token(
            ethers::core::abi::Token::Tuple(data_tokens),
        )?)
    }
}
impl ethers::core::abi::AbiEncode for LogUint {
    fn encode(self) -> ::std::vec::Vec<u8> {
        let tokens = ethers::core::abi::Tokenize::into_tokens(self);
        let selector = <Self as ethers::contract::EthCall>::selector();
        let encoded = ethers::core::abi::encode(&tokens);
        selector
            .iter()
            .copied()
            .chain(encoded.into_iter())
            .collect()
    }
}
impl ::std::fmt::Display for LogUint {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        write!(f, "{:?}", self.p_0)?;
        Ok(())
    }
}
