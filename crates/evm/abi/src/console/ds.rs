//! DSTest log interface.

use super::{format_units_int, format_units_uint};
use alloy_primitives::hex;
use alloy_sol_types::sol;
use derive_more::Display;
use foundry_common_fmt::UIfmt;
use itertools::Itertools;

// Using UIfmt for consistent and user-friendly formatting

sol! {
#[sol(abi)]
#[derive(Display)]
interface Console {
    #[display("{}", val.pretty())]
    event log(string val);

    #[display("{}", hex::encode_prefixed(val))]
    event logs(bytes val);

    #[display("{}", val.pretty())]
    event log_address(address val);

    #[display("{}", val.pretty())]
    event log_bytes32(bytes32 val);

    #[display("{}", val.pretty())]
    event log_int(int val);

    #[display("{}", val.pretty())]
    event log_uint(uint val);

    #[display("{}", hex::encode_prefixed(val))]
    event log_bytes(bytes val);

    #[display("{}", val.pretty())]
    event log_string(string val);

    #[display("[{}]", val.iter().map(|v| v.pretty()).format(", "))]
    event log_array(uint256[] val);

    #[display("[{}]", val.iter().map(|v| v.pretty()).format(", "))]
    event log_array(int256[] val);

    #[display("[{}]", val.iter().map(|v| v.pretty()).format(", "))]
    event log_array(address[] val);

    #[display("{}: {}", key.pretty(), val.pretty())]
    event log_named_address(string key, address val);

    #[display("{}: {}", key.pretty(), val.pretty())]
    event log_named_bytes32(string key, bytes32 val);

    #[display("{}: {}", key.pretty(), format_units_int(val, decimals))]
    event log_named_decimal_int(string key, int val, uint decimals);

    #[display("{}: {}", key.pretty(), format_units_uint(val, decimals))]
    event log_named_decimal_uint(string key, uint val, uint decimals);

    #[display("{}: {}", key.pretty(), val.pretty())]
    event log_named_int(string key, int val);

    #[display("{}: {}", key.pretty(), val.pretty())]
    event log_named_uint(string key, uint val);

    #[display("{}: {}", key.pretty(), hex::encode_prefixed(val))]
    event log_named_bytes(string key, bytes val);

    #[display("{}: {}", key.pretty(), val.pretty())]
    event log_named_string(string key, string val);

    #[display("{}: [{}]", key.pretty(), val.iter().map(|v| v.pretty()).format(", "))]
    event log_named_array(string key, uint256[] val);

    #[display("{}: [{}]", key.pretty(), val.iter().map(|v| v.pretty()).format(", "))]
    event log_named_array(string key, int256[] val);

    #[display("{}: [{}]", key.pretty(), val.iter().map(|v| v.pretty()).format(", "))]
    event log_named_array(string key, address[] val);
}
}

pub use Console::*;
