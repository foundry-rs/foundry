#[macro_use]
extern crate tracing;

/// Decoding helpers
pub mod decode;

/// Call tracing
/// Contains a call trace arena, decoding and formatting utilities
pub mod trace;

/// Debugger data structures
pub mod debug;

/// Coverage data structures
pub mod coverage;

/// Forge test execution backends
pub mod executor;

use ethers::types::{ActionType, CallType, H160};
pub use executor::abi;

/// Fuzzing wrapper for executors
pub mod fuzz;

/// utils for working with revm
pub mod utils;

// Re-exports
pub use ethers::types::Address;
pub use hashbrown;
use revm::interpreter::{CallScheme, CreateScheme};
pub use revm::{self, primitives::HashMap};
use serde::{Deserialize, Serialize};

/// Stores the caller address to be used as _sender_ account for:
///     - deploying Test contracts
///     - deploying Script contracts
///
/// The address was derived from `address(uint160(uint256(keccak256("foundry default caller"))))`
/// and is equal to 0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38.
pub const CALLER: Address = H160([
    0x18, 0x04, 0xc8, 0xAB, 0x1F, 0x12, 0xE6, 0xbb, 0xF3, 0x89, 0x4D, 0x40, 0x83, 0xF3, 0x3E, 0x07,
    0x30, 0x9D, 0x1F, 0x38,
]);

/// Stores the default test contract address: 0xb4c79daB8f259C7Aee6E5b2Aa729821864227e84
pub const TEST_CONTRACT_ADDRESS: Address = H160([
    180, 199, 157, 171, 143, 37, 156, 122, 238, 110, 91, 42, 167, 41, 130, 24, 100, 34, 126, 132,
]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[derive(Default)]
pub enum CallKind {
    #[default]
    Call,
    StaticCall,
    CallCode,
    DelegateCall,
    Create,
    Create2,
}

impl From<CallScheme> for CallKind {
    fn from(scheme: CallScheme) -> Self {
        match scheme {
            CallScheme::Call => CallKind::Call,
            CallScheme::StaticCall => CallKind::StaticCall,
            CallScheme::CallCode => CallKind::CallCode,
            CallScheme::DelegateCall => CallKind::DelegateCall,
        }
    }
}

impl From<CreateScheme> for CallKind {
    fn from(create: CreateScheme) -> Self {
        match create {
            CreateScheme::Create => CallKind::Create,
            CreateScheme::Create2 { .. } => CallKind::Create2,
        }
    }
}

impl From<CallKind> for ActionType {
    fn from(kind: CallKind) -> Self {
        match kind {
            CallKind::Call | CallKind::StaticCall | CallKind::DelegateCall | CallKind::CallCode => {
                ActionType::Call
            }
            CallKind::Create => ActionType::Create,
            CallKind::Create2 => ActionType::Create,
        }
    }
}

impl From<CallKind> for CallType {
    fn from(ty: CallKind) -> Self {
        match ty {
            CallKind::Call => CallType::Call,
            CallKind::StaticCall => CallType::StaticCall,
            CallKind::CallCode => CallType::CallCode,
            CallKind::DelegateCall => CallType::DelegateCall,
            CallKind::Create => CallType::None,
            CallKind::Create2 => CallType::None,
        }
    }
}
