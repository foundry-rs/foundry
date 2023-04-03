use ethers::{
    types::Address,
    utils::{get_contract_address, get_create2_address},
};
use revm::{CreateInputs, CreateScheme, SpecId};

/// Returns [Return::Continue] on an error, discarding the error.
///
/// Useful for inspectors that read state that might be invalid, but do not want to emit
/// appropriate errors themselves, instead opting to continue.
macro_rules! try_or_continue {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(_) => return Return::Continue,
        }
    };
}

/// Get the address of a contract creation
pub fn get_create_address(call: &CreateInputs, nonce: u64) -> Address {
    match call.scheme {
        CreateScheme::Create => get_contract_address(Address::from_slice(call.caller.as_bytes()), nonce),
        CreateScheme::Create2 { salt } => {
            get_create2_address(Address::from_slice(call.caller.as_bytes()), salt.to_be_bytes(), call.init_code.clone())
        }
    }
}

/// Get the gas used, accounting for refunds
pub fn gas_used(spec: SpecId, spent: u64, refunded: u64) -> u64 {
    let refund_quotient = if SpecId::enabled(spec, SpecId::LONDON) { 5 } else { 2 };
    spent - (refunded).min(spent / refund_quotient)
}
