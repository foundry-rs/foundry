use super::{fmt_err, Cheatcodes, Result};
use crate::abi::HEVMCalls;
use ethers::{
    abi::{ethereum_types::FromDecStrErr, ParamType, Token},
    prelude::*,
};
use foundry_macros::UIfmt;
use revm::{Database, EVMData};

pub fn parse(s: &str, ty: &ParamType) -> Result {
    parse_token(s, ty)
        .map(|token| abi::encode(&[token]).into())
        .map_err(|e| fmt_err!("Failed to parse `{s}` as type `{ty}`: {e}"))
}

pub fn parse_array<I, T>(values: I, ty: &ParamType) -> Result
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let mut values = values.into_iter();
    match values.next() {
        Some(first) if !first.as_ref().is_empty() => {
            let tokens = std::iter::once(first)
                .chain(values)
                .map(|v| parse_token(v.as_ref(), ty))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(abi::encode(&[Token::Array(tokens)]).into())
        }
        // return the empty encoded Bytes when values is empty or the first element is empty
        _ => Ok(abi::encode(&[Token::String(String::new())]).into()),
    }
}

fn parse_token(s: &str, ty: &ParamType) -> Result<Token, String> {
    match ty {
        ParamType::Bool => {
            s.to_ascii_lowercase().parse().map(Token::Bool).map_err(|e| e.to_string())
        }
        ParamType::Uint(256) => parse_uint(s).map(Token::Uint),
        ParamType::Int(256) => parse_int(s).map(Token::Int),
        ParamType::Address => s.parse().map(Token::Address).map_err(|e| e.to_string()),
        ParamType::FixedBytes(32) => parse_bytes(s).map(Token::FixedBytes),
        ParamType::Bytes => parse_bytes(s).map(Token::Bytes),
        ParamType::String => Ok(Token::String(s.to_string())),
        _ => Err("unsupported type".into()),
    }
}

fn parse_int(s: &str) -> Result<U256, String> {
    // Only parse hex strings prefixed by 0x or decimal integer strings

    // Hex string may start with "0x", "+0x", or "-0x" which needs to be stripped for
    // `I256::from_hex_str`
    if s.starts_with("0x") || s.starts_with("+0x") || s.starts_with("-0x") {
        return s
            .replacen("0x", "", 1)
            .parse::<I256>()
            .map_err(|err| err.to_string())
            .map(|v| v.into_raw())
    }

    // Decimal string may start with '+' or '-' followed by numeric characters
    if s.chars().all(|c| c.is_numeric() || c == '+' || c == '-') {
        return match I256::from_dec_str(s) {
            Ok(val) => Ok(val),
            Err(dec_err) => s.parse::<I256>().map_err(|hex_err| {
                format!("could not parse value as decimal or hex: {dec_err}, {hex_err}")
            }),
        }
        .map(|v| v.into_raw())
    };

    // Throw if string doesn't conform to either of the two patterns
    Err(ParseI256Error::InvalidDigit.to_string())
}

fn parse_uint(s: &str) -> Result<U256, String> {
    // Only parse hex strings prefixed by 0x or decimal numeric strings

    // Hex strings prefixed by 0x
    if s.starts_with("0x") {
        return s.parse::<U256>().map_err(|err| err.to_string())
    };

    // Decimal strings containing only numeric characters
    if s.chars().all(|c| c.is_numeric()) {
        return match U256::from_dec_str(s) {
            Ok(val) => Ok(val),
            Err(dec_err) => s.parse::<U256>().map_err(|hex_err| {
                format!("could not parse value as decimal or hex: {dec_err}, {hex_err}")
            }),
        }
    };

    // Throw if string doesn't conform to either of the two patterns
    Err(FromDecStrErr::InvalidCharacter.to_string())
}

fn parse_bytes(s: &str) -> Result<Vec<u8>, String> {
    hex::decode(s).map_err(|e| e.to_string())
}

#[instrument(level = "error", name = "util", target = "evm::cheatcodes", skip_all)]
pub fn apply<DB: Database>(
    _state: &mut Cheatcodes,
    _data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result> {
    Some(match call {
        HEVMCalls::ToString0(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString1(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString2(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString3(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString4(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString5(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ParseBytes(inner) => parse(&inner.0, &ParamType::Bytes),
        HEVMCalls::ParseAddress(inner) => parse(&inner.0, &ParamType::Address),
        HEVMCalls::ParseUint(inner) => parse(&inner.0, &ParamType::Uint(256)),
        HEVMCalls::ParseInt(inner) => parse(&inner.0, &ParamType::Int(256)),
        HEVMCalls::ParseBytes32(inner) => parse(&inner.0, &ParamType::FixedBytes(32)),
        HEVMCalls::ParseBool(inner) => parse(&inner.0, &ParamType::Bool),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::abi::AbiDecode;

    #[test]
    fn test_uint_env() {
        let pk = "0x10532cc9d0d992825c3f709c62c969748e317a549634fb2a9fa949326022e81f";
        let val: U256 = pk.parse().unwrap();
        let parsed = parse(pk, &ParamType::Uint(256)).unwrap();
        let decoded = U256::decode(&parsed).unwrap();
        assert_eq!(val, decoded);

        let parsed = parse(pk, &ParamType::Uint(256)).unwrap();
        let decoded = U256::decode(&parsed).unwrap();
        assert_eq!(val, decoded);

        let parsed = parse("1337", &ParamType::Uint(256)).unwrap();
        let decoded = U256::decode(&parsed).unwrap();
        assert_eq!(U256::from(1337u64), decoded);
    }

    #[test]
    fn test_int_env() {
        let val = U256::from(100u64);
        let parsed = parse(&val.to_string(), &ParamType::Int(256)).unwrap();
        let decoded = I256::decode(parsed).unwrap();
        assert_eq!(val, decoded.try_into().unwrap());

        let parsed = parse("100", &ParamType::Int(256)).unwrap();
        let decoded = I256::decode(parsed).unwrap();
        assert_eq!(U256::from(100u64), decoded.try_into().unwrap());
    }
}
