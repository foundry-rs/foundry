/// Decoding helpers
pub mod decode;

/// Call trace arena, decoding and formatting
pub mod trace;

/// Debugger data structures
pub mod debug;

/// Forge test execution backends
pub mod executor;

use ethers::types::{ActionType, CallType};
pub use executor::abi;

/// Fuzzing wrapper for executors
pub mod fuzz;

/// utils for working with revm
pub mod utils;

// Re-exports
pub use ethers::types::Address;
pub use hashbrown::HashMap;
pub use revm;

use once_cell::sync::Lazy;
pub static CALLER: Lazy<Address> = Lazy::new(Address::random);

use revm::{CallScheme, CreateScheme};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CallKind {
    Call,
    StaticCall,
    CallCode,
    DelegateCall,
    Create,
}

impl Default for CallKind {
    fn default() -> Self {
        CallKind::Call
    }
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
    fn from(_: CreateScheme) -> Self {
        CallKind::Create
    }
}

impl From<CallKind> for ActionType {
    fn from(kind: CallKind) -> Self {
        match kind {
            CallKind::Call | CallKind::StaticCall | CallKind::DelegateCall | CallKind::CallCode => {
                ActionType::Call
            }
            CallKind::Create => ActionType::Create,
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
        }
    }
}
