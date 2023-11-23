use alloy_primitives::{hex, I256, U256};
use alloy_sol_types::sol;
use derive_more::Display;
use foundry_common::types::ToEthers;
use itertools::Itertools;

// TODO: Use `UiFmt`

sol! {
#[sol(abi)]
#[derive(Display)]
interface Console {
    #[display(fmt = "{val}")]
    event log(string val);

    #[display(fmt = "{}", "hex::encode_prefixed(val)")]
    event logs(bytes val);

    #[display(fmt = "{val}")]
    event log_address(address val);

    #[display(fmt = "{val}")]
    event log_bytes32(bytes32 val);

    #[display(fmt = "{val}")]
    event log_int(int val);

    #[display(fmt = "{val}")]
    event log_uint(uint val);

    #[display(fmt = "{}", "hex::encode_prefixed(val)")]
    event log_bytes(bytes val);

    #[display(fmt = "{val}")]
    event log_string(string val);

    #[display(fmt = "[{}]", "val.iter().format(\", \")")]
    event log_array(uint256[] val);

    #[display(fmt = "[{}]", "val.iter().format(\", \")")]
    event log_array(int256[] val);

    #[display(fmt = "[{}]", "val.iter().format(\", \")")]
    event log_array(address[] val);

    #[display(fmt = "{key}: {val}")]
    event log_named_address(string key, address val);

    #[display(fmt = "{key}: {val}")]
    event log_named_bytes32(string key, bytes32 val);

    #[display(fmt = "{key}: {}", "format_units_int(val, decimals)")]
    event log_named_decimal_int(string key, int val, uint decimals);

    #[display(fmt = "{key}: {}", "format_units_uint(val, decimals)")]
    event log_named_decimal_uint(string key, uint val, uint decimals);

    #[display(fmt = "{key}: {val}")]
    event log_named_int(string key, int val);

    #[display(fmt = "{key}: {val}")]
    event log_named_uint(string key, uint val);

    #[display(fmt = "{key}: {}", "hex::encode_prefixed(val)")]
    event log_named_bytes(string key, bytes val);

    #[display(fmt = "{key}: {val}")]
    event log_named_string(string key, string val);

    #[display(fmt = "{key}: [{}]", "val.iter().format(\", \")")]
    event log_named_array(string key, uint256[] val);

    #[display(fmt = "{key}: [{}]", "val.iter().format(\", \")")]
    event log_named_array(string key, int256[] val);

    #[display(fmt = "{key}: [{}]", "val.iter().format(\", \")")]
    event log_named_array(string key, address[] val);
}
}

fn format_units_int(x: &I256, decimals: &U256) -> String {
    let (sign, x) = x.into_sign_and_abs();
    format!("{sign}{}", format_units_uint(&x, decimals))
}

fn format_units_uint(x: &U256, decimals: &U256) -> String {
    // TODO: rm ethers_core
    match ethers_core::utils::format_units(x.to_ethers(), decimals.saturating_to::<u32>()) {
        Ok(s) => s,
        Err(_) => x.to_string(),
    }
}
