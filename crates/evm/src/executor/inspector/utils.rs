use bytes::Buf;

use revm::{
    interpreter::CreateInputs,
    primitives::{Address, CreateScheme, SpecId},
};

use crate::utils::{ru256_to_u256};

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
        CreateScheme::Create => Address::create(&call.caller, nonce),
        CreateScheme::Create2 { salt } => {
            let salt = ru256_to_u256(salt);
            let mut salt_bytes = [0u8; 32];
            salt.to_big_endian(&mut salt_bytes);
            let init_code =
                alloy_primitives::Bytes(call.init_code.clone().0).to_owned().0.copy_to_bytes(32);
            let init_code_hash = alloy_primitives::keccak256(init_code);
            Address::create2(&call.caller, salt_bytes, init_code_hash)
        }
    }
}

/// Get the gas used, accounting for refunds
pub fn gas_used(spec: SpecId, spent: u64, refunded: u64) -> u64 {
    let refund_quotient = if SpecId::enabled(spec, SpecId::LONDON) { 5 } else { 2 };
    spent - (refunded).min(spent / refund_quotient)
}
