//! ABI related helper functions

use ethers_core::{
    abi::{
        token::{LenientTokenizer, StrictTokenizer, Tokenizer},
        Event, Function, HumanReadableParser, ParamType, RawLog, Token,
    },
    types::{Address, Chain, I256, U256},
    utils::{hex, to_checksum},
};
use ethers_etherscan::{contract::ContractMetadata, errors::EtherscanError, Client};
use eyre::{ContextCompat, Result, WrapErr};
use std::{future::Future, pin::Pin, str::FromStr};
use yansi::Paint;

use crate::calc::to_exponential_notation;

/// Given a function and a vector of string arguments, it proceeds to convert the args to ethabi
/// Tokens and then ABI encode them.
pub fn encode_args(func: &Function, args: &[impl AsRef<str>]) -> Result<Vec<u8>> {
    let params = func
        .inputs
        .iter()
        .zip(args)
        .map(|(input, arg)| (&input.kind, arg.as_ref()))
        .collect::<Vec<_>>();
    let tokens = parse_tokens(params, true)?;
    Ok(func.encode_input(&tokens)?)
}

/// Decodes the calldata of the function
///
/// # Panics
///
/// If the `sig` is an invalid function signature
pub fn abi_decode(sig: &str, calldata: &str, input: bool, fn_selector: bool) -> Result<Vec<Token>> {
    let func = IntoFunction::into(sig);
    let calldata = hex::decode(calldata)?;
    let res = if input {
        // If function selector is prefixed in "calldata", remove it (first 4 bytes)
        if fn_selector {
            func.decode_input(&calldata[4..])?
        } else {
            func.decode_input(&calldata)?
        }
    } else {
        func.decode_output(&calldata)?
    };

    // in case the decoding worked but nothing was decoded
    if res.is_empty() {
        eyre::bail!("no data was decoded")
    }

    Ok(res)
}

/// Parses string input as Token against the expected ParamType
pub fn parse_tokens<'a, I: IntoIterator<Item = (&'a ParamType, &'a str)>>(
    params: I,
    lenient: bool,
) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();

    for (param, value) in params.into_iter() {
        let mut token = if lenient {
            LenientTokenizer::tokenize(param, value)
        } else {
            StrictTokenizer::tokenize(param, value)
        };
        if token.is_err() && value.starts_with("0x") {
            match param {
                ParamType::FixedBytes(32) => {
                    if value.len() < 66 {
                        let padded_value = [value, &"0".repeat(66 - value.len())].concat();
                        token = if lenient {
                            LenientTokenizer::tokenize(param, &padded_value)
                        } else {
                            StrictTokenizer::tokenize(param, &padded_value)
                        };
                    }
                }
                ParamType::Uint(_) => {
                    // try again if value is hex
                    if let Ok(value) = U256::from_str(value).map(|v| v.to_string()) {
                        token = if lenient {
                            LenientTokenizer::tokenize(param, &value)
                        } else {
                            StrictTokenizer::tokenize(param, &value)
                        };
                    }
                }
                // TODO: Not sure what to do here. Put the no effect in for now, but that is not
                // ideal. We could attempt massage for every value type?
                _ => {}
            }
        }

        let token = token.map(sanitize_token).wrap_err_with(|| {
            format!("Failed to parse `{value}`, expected value of type: {param}")
        })?;
        tokens.push(token);
    }
    Ok(tokens)
}

/// Cleans up potential shortcomings of the ethabi Tokenizer.
///
/// For example: parsing a string array with a single empty string: `[""]`, is returned as
///
/// ```text
///     [
///        String(
///            "\"\"",
///        ),
///    ],
/// ```
///
/// But should just be
///
/// ```text
///     [
///        String(
///            "",
///        ),
///    ],
/// ```
///
/// This will handle this edge case
pub fn sanitize_token(token: Token) -> Token {
    match token {
        Token::Array(tokens) => {
            let mut sanitized = Vec::with_capacity(tokens.len());
            for token in tokens {
                let token = match token {
                    Token::String(val) => {
                        let val = match val.as_str() {
                            // this is supposed to be an empty string
                            "\"\"" | "''" => "".to_string(),
                            _ => val,
                        };
                        Token::String(val)
                    }
                    _ => sanitize_token(token),
                };
                sanitized.push(token)
            }
            Token::Array(sanitized)
        }
        _ => token,
    }
}

/// Pretty print a slice of tokens.
pub fn format_tokens(tokens: &[Token]) -> impl Iterator<Item = String> + '_ {
    tokens.iter().map(format_token)
}

/// Gets pretty print strings for tokens
pub fn format_token(param: &Token) -> String {
    match param {
        Token::Address(addr) => to_checksum(addr, None),
        Token::FixedBytes(bytes) => hex::encode_prefixed(bytes),
        Token::Bytes(bytes) => hex::encode_prefixed(bytes),
        Token::Int(num) => format!("{}", I256::from_raw(*num)),
        Token::Uint(num) => format_uint_with_exponential_notation_hint(*num),
        Token::Bool(b) => format!("{b}"),
        Token::String(s) => s.to_string(),
        Token::FixedArray(tokens) => {
            let string = tokens.iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("[{string}]")
        }
        Token::Array(tokens) => {
            let string = tokens.iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("[{string}]")
        }
        Token::Tuple(tokens) => {
            let string = tokens.iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("({string})")
        }
    }
}

/// Gets pretty print strings for tokens, without adding
/// exponential notation hints for large numbers (e.g. [1e7] for 10000000)
pub fn format_token_raw(param: &Token) -> String {
    match param {
        Token::Uint(num) => format!("{}", num),
        Token::FixedArray(tokens) | Token::Array(tokens) => {
            let string = tokens.iter().map(format_token_raw).collect::<Vec<String>>().join(", ");
            format!("[{string}]")
        }
        Token::Tuple(tokens) => {
            let string = tokens.iter().map(format_token_raw).collect::<Vec<String>>().join(", ");
            format!("({string})")
        }
        _ => format_token(param),
    }
}

/// Formats a U256 number to string, adding an exponential notation _hint_ if it
/// is larger than `10_000`, with a precision of `4` figures, and trimming the
/// trailing zeros.
///
/// Examples:
///
/// ```text
///   0 -> "0"
///   1234 -> "1234"
///   1234567890 -> "1234567890 [1.234e9]"
///   1000000000000000000 -> "1000000000000000000 [1e18]"
///   10000000000000000000000 -> "10000000000000000000000 [1e22]"
/// ```
pub fn format_uint_with_exponential_notation_hint(num: U256) -> String {
    if num.lt(&U256::from(10_000)) {
        return num.to_string()
    }

    let exp = to_exponential_notation(num, 4, true);
    format!("{} {}", num, Paint::default(format!("[{}]", exp)).dimmed())
}

/// Helper trait for converting types to Functions. Helpful for allowing the `call`
/// function on the EVM to be generic over `String`, `&str` and `Function`.
pub trait IntoFunction {
    /// Consumes self and produces a function
    ///
    /// # Panic
    ///
    /// This function does not return a Result, so it is expected that the consumer
    /// uses it correctly so that it does not panic.
    fn into(self) -> Function;
}

impl IntoFunction for Function {
    fn into(self) -> Function {
        self
    }
}

impl IntoFunction for String {
    fn into(self) -> Function {
        IntoFunction::into(self.as_str())
    }
}

impl<'a> IntoFunction for &'a str {
    fn into(self) -> Function {
        HumanReadableParser::parse_function(self)
            .unwrap_or_else(|_| panic!("could not convert {self} to function"))
    }
}

/// Given a function signature string, it tries to parse it as a `Function`
pub fn get_func(sig: &str) -> Result<Function> {
    Ok(match HumanReadableParser::parse_function(sig) {
        Ok(func) => func,
        Err(err) => {
            if let Ok(constructor) = HumanReadableParser::parse_constructor(sig) {
                #[allow(deprecated)]
                Function {
                    name: "constructor".to_string(),
                    inputs: constructor.inputs,
                    outputs: vec![],
                    constant: None,
                    state_mutability: Default::default(),
                }
            } else {
                // we return the `Function` parse error as this case is more likely
                return Err(err.into())
            }
        }
    })
}

/// Given an event signature string, it tries to parse it as a `Event`
pub fn get_event(sig: &str) -> Result<Event> {
    Ok(HumanReadableParser::parse_event(sig)?)
}

/// Given an event without indexed parameters and a rawlog, it tries to return the event with the
/// proper indexed parameters. Otherwise, it returns the original event.
pub fn get_indexed_event(mut event: Event, raw_log: &RawLog) -> Event {
    if !event.anonymous && raw_log.topics.len() > 1 {
        let indexed_params = raw_log.topics.len() - 1;
        let num_inputs = event.inputs.len();
        let num_address_params =
            event.inputs.iter().filter(|p| p.kind == ParamType::Address).count();

        event.inputs.iter_mut().enumerate().for_each(|(index, param)| {
            if param.name.is_empty() {
                param.name = format!("param{index}");
            }
            if num_inputs == indexed_params ||
                (num_address_params == indexed_params && param.kind == ParamType::Address)
            {
                param.indexed = true;
            }
        })
    }
    event
}

/// Given a function name, address, and args, tries to parse it as a `Function` by fetching the
/// abi from etherscan. If the address is a proxy, fetches the ABI of the implementation contract.
pub async fn get_func_etherscan(
    function_name: &str,
    contract: Address,
    args: &[String],
    chain: Chain,
    etherscan_api_key: &str,
) -> Result<Function> {
    let client = Client::new(chain, etherscan_api_key)?;
    let source = find_source(client, contract).await?;
    let metadata = source.items.first().wrap_err("etherscan returned empty metadata")?;

    let mut abi = metadata.abi()?;
    let funcs = abi.functions.remove(function_name).unwrap_or_default();

    for func in funcs {
        let res = encode_args(&func, args);
        if res.is_ok() {
            return Ok(func)
        }
    }

    Err(eyre::eyre!("Function not found in abi"))
}

/// If the code at `address` is a proxy, recurse until we find the implementation.
pub fn find_source(
    client: Client,
    address: Address,
) -> Pin<Box<dyn Future<Output = Result<ContractMetadata>>>> {
    Box::pin(async move {
        tracing::trace!("find etherscan source for: {:?}", address);
        let source = client.contract_source_code(address).await?;
        let metadata = source.items.first().wrap_err("Etherscan returned no data")?;
        if metadata.proxy == 0 {
            Ok(source)
        } else {
            let implementation = metadata.implementation.unwrap();
            println!(
                "Contract at {address} is a proxy, trying to fetch source at {implementation:?}..."
            );
            match find_source(client, implementation).await {
                impl_source @ Ok(_) => impl_source,
                Err(e) => {
                    let err = EtherscanError::ContractCodeNotVerified(address).to_string();
                    if e.to_string() == err {
                        tracing::error!("{}", err);
                        Ok(source)
                    } else {
                        Err(e)
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers_core::types::H256;

    #[test]
    fn can_sanitize_token() {
        let token =
            Token::Array(LenientTokenizer::tokenize_array("[\"\"]", &ParamType::String).unwrap());
        let sanitized = sanitize_token(token);
        assert_eq!(sanitized, Token::Array(vec![Token::String("".to_string())]));

        let token =
            Token::Array(LenientTokenizer::tokenize_array("['']", &ParamType::String).unwrap());
        let sanitized = sanitize_token(token);
        assert_eq!(sanitized, Token::Array(vec![Token::String("".to_string())]));

        let token = Token::Array(
            LenientTokenizer::tokenize_array("[\"\",\"\"]", &ParamType::String).unwrap(),
        );
        let sanitized = sanitize_token(token);
        assert_eq!(
            sanitized,
            Token::Array(vec![Token::String("".to_string()), Token::String("".to_string())])
        );

        let token =
            Token::Array(LenientTokenizer::tokenize_array("['','']", &ParamType::String).unwrap());
        let sanitized = sanitize_token(token);
        assert_eq!(
            sanitized,
            Token::Array(vec![Token::String("".to_string()), Token::String("".to_string())])
        );
    }

    #[test]
    fn parse_hex_uint_tokens() {
        let param = ParamType::Uint(256);

        let tokens = parse_tokens(std::iter::once((&param, "100")), true).unwrap();
        assert_eq!(tokens, vec![Token::Uint(100u64.into())]);

        let val: U256 = 100u64.into();
        let hex_val = format!("0x{val:x}");
        let tokens = parse_tokens(std::iter::once((&param, hex_val.as_str())), true).unwrap();
        assert_eq!(tokens, vec![Token::Uint(100u64.into())]);
    }

    #[test]
    fn test_indexed_only_address() {
        let event = get_event("event Ev(address,uint256,address)").unwrap();

        let param0 = H256::random();
        let param1 = vec![3; 32];
        let param2 = H256::random();
        let log = RawLog { topics: vec![event.signature(), param0, param2], data: param1.clone() };
        let event = get_indexed_event(event, &log);

        assert_eq!(event.inputs.len(), 3);

        // Only the address fields get indexed since total_params > num_indexed_params
        let parsed = event.parse_log(log).unwrap();

        assert_eq!(event.inputs.iter().filter(|param| param.indexed).count(), 2);
        assert_eq!(parsed.params[0].name, "param0");
        assert_eq!(parsed.params[0].value, Token::Address(param0.into()));
        assert_eq!(parsed.params[1].name, "param1");
        assert_eq!(parsed.params[1].value, Token::Uint(U256::from_big_endian(&param1)));
        assert_eq!(parsed.params[2].name, "param2");
        assert_eq!(parsed.params[2].value, Token::Address(param2.into()));
    }

    #[test]
    fn test_indexed_all() {
        let event = get_event("event Ev(address,uint256,address)").unwrap();

        let param0 = H256::random();
        let param1 = vec![3; 32];
        let param2 = H256::random();
        let log = RawLog {
            topics: vec![event.signature(), param0, H256::from_slice(&param1), param2],
            data: vec![],
        };
        let event = get_indexed_event(event, &log);

        assert_eq!(event.inputs.len(), 3);

        // All parameters get indexed since num_indexed_params == total_params
        assert_eq!(event.inputs.iter().filter(|param| param.indexed).count(), 3);
        let parsed = event.parse_log(log).unwrap();

        assert_eq!(parsed.params[0].name, "param0");
        assert_eq!(parsed.params[0].value, Token::Address(param0.into()));
        assert_eq!(parsed.params[1].name, "param1");
        assert_eq!(parsed.params[1].value, Token::Uint(U256::from_big_endian(&param1)));
        assert_eq!(parsed.params[2].name, "param2");
        assert_eq!(parsed.params[2].value, Token::Address(param2.into()));
    }

    #[test]
    fn test_format_token_addr() {
        // copied from testcases in https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md
        let eip55 = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";
        assert_eq!(
            format_token(&Token::Address(Address::from_str(&eip55.to_lowercase()).unwrap())),
            eip55.to_string()
        );

        // copied from testcases in https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1191.md
        let eip1191 = "0xFb6916095cA1Df60bb79ce92cE3EA74c37c5d359";
        assert_ne!(
            format_token(&Token::Address(Address::from_str(&eip1191.to_lowercase()).unwrap())),
            eip1191.to_string()
        );
    }
}
