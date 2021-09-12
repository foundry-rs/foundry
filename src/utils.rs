use ethers::{
    abi::{Function, ParamType, Token, Tokenizable},
    core::abi::parse_abi,
    types::*,
};
use eyre::Result;
use rustc_hex::FromHex;
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

pub fn encode_input(param: &ParamType, value: &str) -> Result<Token> {
    Ok(match param {
        // TODO: Do the rest of the types
        ParamType::Address => Address::from_str(&value)?.into_token(),
        ParamType::Bytes => Bytes::from(value.from_hex::<Vec<u8>>()?).into_token(),
        ParamType::FixedBytes(_) => value.from_hex::<Vec<u8>>()?.into_token(),
        ParamType::Uint(n) => {
            let radix = if value.starts_with("0x") { 16 } else { 10 };
            match n / 8 {
                1 => u8::from_str_radix(value, radix)?.into_token(),
                2 => u16::from_str_radix(value, radix)?.into_token(),
                3..=4 => u32::from_str_radix(value, radix)?.into_token(),
                5..=8 => u64::from_str_radix(value, radix)?.into_token(),
                9..=16 => u128::from_str_radix(value, radix)?.into_token(),
                17..=32 => if radix == 16 {
                    U256::from_str(value)?
                } else {
                    U256::from_dec_str(value)?
                }
                .into_token(),
                _ => eyre::bail!("unsupoprted solidity type uint{}", n),
            }
        }
        ParamType::Int(n) => {
            let radix = if value.starts_with("0x") { 16 } else { 10 };
            match n / 8 {
                1 => i8::from_str_radix(value, radix)?.into_token(),
                2 => i16::from_str_radix(value, radix)?.into_token(),
                3..=4 => i32::from_str_radix(value, radix)?.into_token(),
                5..=8 => i64::from_str_radix(value, radix)?.into_token(),
                9..=16 => i128::from_str_radix(value, radix)?.into_token(),
                17..=32 => if radix == 16 {
                    I256::from_str(value)?
                } else {
                    I256::from_dec_str(value)?
                }
                .into_token(),
                _ => eyre::bail!("unsupoprted solidity type uint{}", n),
            }
        }
        ParamType::Bool => bool::from_str(value)?.into_token(),
        ParamType::String => value.to_string().into_token(),
        ParamType::Array(_) => {
            unimplemented!()
        }
        ParamType::FixedArray(_, _) => {
            unimplemented!()
        }
        ParamType::Tuple(_) => {
            unimplemented!()
        }
    })
}

pub fn encode_args(func: &Function, args: Vec<String>) -> Result<Vec<u8>> {
    // Dynamically build up the calldata via the function sig
    let mut inputs = Vec::new();
    for (i, input) in func.inputs.iter().enumerate() {
        let input = encode_input(&input.kind, &args[i])?;
        inputs.push(input);
    }
    Ok(func.encode_input(&inputs)?)
}
