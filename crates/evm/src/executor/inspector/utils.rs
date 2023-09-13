use alloy_primitives::B256;

use revm::{
    interpreter::CreateInputs,
    primitives::{Address, CreateScheme, SpecId},
};

/// Returns [InstructionResult::Continue] on an error, discarding the error.
///
/// Useful for inspectors that read state that might be invalid, but do not want to emit
/// appropriate errors themselves, instead opting to continue.
macro_rules! try_or_continue {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(_) => return InstructionResult::Continue,
        }
    };
}

/// Get the address of a contract creation
pub fn get_create_address(call: &CreateInputs, nonce: u64) -> Address {
    match call.scheme {
        CreateScheme::Create => call.caller.create(nonce),
        CreateScheme::Create2 { salt } => {
            let init_code = alloy_primitives::Bytes(call.init_code.0.clone());
            let init_code_hash = alloy_primitives::keccak256(init_code);
            call.caller.create2(B256::from(salt), init_code_hash)
        }
    }
}

/// Get the gas used, accounting for refunds
pub fn gas_used(spec: SpecId, spent: u64, refunded: u64) -> u64 {
    let refund_quotient = if SpecId::enabled(spec, SpecId::LONDON) { 5 } else { 2 };
    spent - (refunded).min(spent / refund_quotient)
}
