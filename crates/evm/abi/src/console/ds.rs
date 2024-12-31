//! DSTest log interface.

use super::{format_units_int, format_units_uint};
use alloy_primitives::hex;
use alloy_sol_types::sol;
use derive_more::Display;
use itertools::Itertools;

// TODO: Use `UiFmt`

sol! {
#[sol(abi)]
#[derive(Display)]
interface Console {
    #[display("{val}")]
    event log(string val);

    #[display("{}", hex::encode_prefixed(val))]
    event logs(bytes val);

    #[display("{val}")]
    event log_address(address val);

    #[display("{val}")]
    event log_bytes32(bytes32 val);

    #[display("{val}")]
    event log_int(int val);

    #[display("{val}")]
    event log_uint(uint val);

    #[display("{}", hex::encode_prefixed(val))]
    event log_bytes(bytes val);

    #[display("{val}")]
    event log_string(string val);

    #[display("[{}]", val.iter().format(", "))]
    event log_array(uint256[] val);

    #[display("[{}]", val.iter().format(", "))]
    event log_array(int256[] val);

    #[display("[{}]", val.iter().format(", "))]
    event log_array(address[] val);

    #[display("{key}: {val}")]
    event log_named_address(string key, address val);

    #[display("{key}: {val}")]
    event log_named_bytes32(string key, bytes32 val);

    #[display("{key}: {}", format_units_int(val, decimals))]
    event log_named_decimal_int(string key, int val, uint decimals);

    #[display("{key}: {}", format_units_uint(val, decimals))]
    event log_named_decimal_uint(string key, uint val, uint decimals);

    #[display("{key}: {val}")]
    event log_named_int(string key, int val);

    #[display("{key}: {val}")]
    event log_named_uint(string key, uint val);

    #[display("{key}: {}", hex::encode_prefixed(val))]
    event log_named_bytes(string key, bytes val);

    #[display("{key}: {val}")]
    event log_named_string(string key, string val);

    #[display("{key}: [{}]", val.iter().format(", "))]
    event log_named_array(string key, uint256[] val);

    #[display("{key}: [{}]", val.iter().format(", "))]
    event log_named_array(string key, int256[] val);

    #[display("{key}: [{}]", val.iter().format(", "))]
    event log_named_array(string key, address[] val);
}
}

pub use Console::*;
