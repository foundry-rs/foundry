use ethers::{
    abi::{Function, ParamType, Tokenizable},
    core::abi::parse_abi,
    types::*,
};
use eyre::Result;
use std::str::FromStr;

// TODO: SethContract with common contract initializers? Same for SethProviders?
pub fn to_table(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Object(map) => {
            let mut s = String::new();
            for (k, v) in map.iter() {
                s.push_str(&format!("{: <20} {}\n", k, v));
            }
            s
        }
        _ => "".to_owned(),
    }
}

pub fn get_func(sig: &str) -> Result<Function> {
    // TODO: Make human readable ABI better / more minimal
    let abi = parse_abi(&[sig])?;
    // get the function
    let (_, func) = abi
        .functions
        .iter()
        .next()
        .ok_or_else(|| eyre::eyre!("function name not found"))?;
    let func = func
        .get(0)
        .ok_or_else(|| eyre::eyre!("functions array empty"))?;
    Ok(func.clone())
}

pub fn encode_args(func: &Function, args: Vec<String>) -> Result<Vec<u8>> {
    // Dynamically build up the calldata via the function sig
    let mut inputs = Vec::new();
    for (i, input) in func.inputs.iter().enumerate() {
        let input = match input.kind {
            // TODO: Do the rest of the types
            ParamType::Address => Address::from_str(&args[i])?.into_token(),
            ParamType::Uint(256) => if args[i].starts_with("0x") {
                U256::from_str(&args[i])?
            } else {
                U256::from_dec_str(&args[i])?
            }
            .into_token(),
            ParamType::String => args[i].clone().into_token(),
            _ => Address::zero().into_token(),
        };
        inputs.push(input);
    }
    Ok(func.encode_input(&inputs)?)
}
