use ethers::{
    types::Address,
    utils::{get_contract_address, get_create2_address},
};
use revm::interpreter::CreateInputs;
use revm::primitives::{CreateScheme, SpecId};

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
        CreateScheme::Create => get_contract_address(call.caller, nonce),
        CreateScheme::Create2 { salt } => {
            let mut buffer: [u8; 4 * 8] = [0; 4 * 8];
            salt.to_big_endian(&mut buffer);
            get_create2_address(call.caller, buffer, call.init_code.clone())
        }
    }
}

/// Get the gas used, accounting for refunds
pub fn gas_used(spec: SpecId, spent: u64, refunded: u64) -> u64 {
    let refund_quotient = if SpecId::enabled(spec, SpecId::LONDON) { 5 } else { 2 };
    spent - (refunded).min(spent / refund_quotient)
}
