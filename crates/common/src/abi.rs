//! ABI related helper functions
use alloy_dyn_abi::{DynSolType, DynSolValue, FunctionExt, JsonAbiExt};
use alloy_json_abi::{Event, Function, AbiItem};
use alloy_primitives::{hex, Address, Log, U256};
use ethers_core::types::Chain;
use eyre::{ContextCompat, Result};
use foundry_block_explorers::{contract::ContractMetadata, errors::EtherscanError, Client};
use std::{future::Future, pin::Pin};
use yansi::Paint;

use crate::calc::to_exponential_notation;

/// Given a function and a vector of string arguments, it proceeds to convert the args to alloy
/// [DynSolValue]s and then ABI encode them.
pub fn encode_function_args(func: &Function, args: &[impl AsRef<str>]) -> Result<Vec<u8>> {
    let params: Result<Vec<_>> = func
        .inputs
        .iter()
        .zip(args)
        .map(|(input, arg)| (input.selector_type().clone(), arg.as_ref()))
        .map(|(ty, arg)| coerce_value(&ty, arg))
        .collect();
    Ok(func.abi_encode_input(params?.as_slice())?)
}

/// Decodes the calldata of the function
pub fn abi_decode_calldata(
    sig: &str,
    calldata: &str,
    input: bool,
    fn_selector: bool,
) -> Result<Vec<DynSolValue>> {
    let func = Function::parse(sig)?;
    let calldata = hex::decode(calldata)?;
    let res = if input {
        // If function selector is prefixed in "calldata", remove it (first 4 bytes)
        if fn_selector {
            func.abi_decode_input(&calldata[4..], false)?
        } else {
            func.abi_decode_input(&calldata, false)?
        }
    } else {
        func.abi_decode_output(&calldata, false)?
    };

    // in case the decoding worked but nothing was decoded
    if res.is_empty() {
        eyre::bail!("no data was decoded")
    }

    Ok(res)
}

/// Parses string input as Token against the expected ParamType
pub fn parse_tokens<'a, I: IntoIterator<Item = (&'a DynSolType, &'a str)>>(
    params: I,
) -> Result<Vec<DynSolValue>> {
    let mut tokens = Vec::new();

    for (param, value) in params.into_iter() {
        let token = DynSolType::coerce_str(param, value)?;
        tokens.push(token);
    }
    Ok(tokens)
}

/// Pretty print a slice of tokens.
pub fn format_tokens(tokens: &[DynSolValue]) -> impl Iterator<Item = String> + '_ {
    tokens.iter().map(format_token)
}

/// Gets pretty print strings for tokens
pub fn format_token(param: &DynSolValue) -> String {
    match param {
        DynSolValue::Address(addr) => addr.to_checksum(None),
        DynSolValue::FixedBytes(bytes, _) => hex::encode_prefixed(bytes),
        DynSolValue::Bytes(bytes) => hex::encode_prefixed(bytes),
        DynSolValue::Int(num, _) => format!("{}", num),
        DynSolValue::Uint(num, _) => format_uint_with_exponential_notation_hint(*num),
        DynSolValue::Bool(b) => format!("{b}"),
        DynSolValue::String(s) => s.to_string(),
        DynSolValue::FixedArray(tokens) => {
            let string = tokens.iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("[{string}]")
        }
        DynSolValue::Array(tokens) => {
            let string = tokens.iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("[{string}]")
        }
        DynSolValue::Tuple(tokens) => {
            let string = tokens.iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("({string})")
        }
        DynSolValue::CustomStruct { name: _, prop_names: _, tuple } => {
            let string = tuple.iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("({string})")
        }
        DynSolValue::Function(f) => {
            format!("{}", f.to_address_and_selector().1)
        }
    }
}

/// Gets pretty print strings for tokens, without adding
/// exponential notation hints for large numbers (e.g. [1e7] for 10000000)
pub fn format_token_raw(param: &DynSolValue) -> String {
    match param {
        DynSolValue::Uint(num, _) => format!("{}", num),
        DynSolValue::FixedArray(tokens) | DynSolValue::Array(tokens) => {
            let string = tokens.iter().map(format_token_raw).collect::<Vec<String>>().join(", ");
            format!("[{string}]")
        }
        DynSolValue::Tuple(tokens) => {
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
        Function::parse(self).expect("could not parse function")
    }
}

/// Given a function signature string, it tries to parse it as a `Function`
pub fn get_func(sig: &str) -> Result<Function> {
    let item = match AbiItem::parse(sig) {
        Ok(item) => match item {
            AbiItem::Function(func) => func,
            _ => return Err(eyre::eyre!("Expected function, got {:?}", item)),
        }
        Err(e) => return Err(e.into())
    };
    Ok(item.into_owned().to_owned())
}

/// Given an event signature string, it tries to parse it as a `Event`
pub fn get_event(sig: &str) -> Result<Event> {
    let sig = sig.strip_prefix("event").unwrap_or(sig).trim();
    Ok(Event::parse(sig)?)
}

/// Given an event without indexed parameters and a rawlog, it tries to return the event with the
/// proper indexed parameters. Otherwise, it returns the original event.
pub fn get_indexed_event(mut event: Event, raw_log: &Log) -> Event {
    if !event.anonymous && raw_log.topics().len() > 1 {
        let indexed_params = raw_log.topics().len() - 1;
        let num_inputs = event.inputs.len();
        let num_address_params = event.inputs.iter().filter(|p| p.ty == "address").count();

        event.inputs.iter_mut().enumerate().for_each(|(index, param)| {
            if param.name.is_empty() {
                param.name = format!("param{index}");
            }
            if num_inputs == indexed_params ||
                (num_address_params == indexed_params && param.ty == "address")
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
        let res = encode_function_args(&func, args);
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

/// Helper function to coerce a value to a [DynSolValue] given a type string
fn coerce_value(ty: &str, arg: &str) -> Result<DynSolValue> {
    let ty = DynSolType::parse(ty)?;
    Ok(DynSolType::coerce_str(&ty, arg)?)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use alloy_dyn_abi::EventExt;
    use alloy_primitives::B256;

    #[test]
    fn parse_hex_uint_tokens() {
        let param = DynSolType::Uint(256);

        let tokens = parse_tokens(std::iter::once((&param, "100"))).unwrap();
        assert_eq!(tokens, vec![DynSolValue::Uint(U256::from(100), 256)]);

        let val: U256 = U256::from(100u64);
        let hex_val = format!("0x{val:x}");
        let tokens = parse_tokens(std::iter::once((&param, hex_val.as_str()))).unwrap();
        assert_eq!(tokens, vec![DynSolValue::Uint(U256::from(100), 256)]);
    }

    #[test]
    fn test_indexed_only_address() {
        let event = get_event("event Ev(address,uint256,address)").unwrap();

        let param0 = B256::random();
        let param1 = vec![3; 32];
        let param2 = B256::random();
        let log = Log::new_unchecked(vec![event.selector(), param0, param2], param1.clone().into());
        let event = get_indexed_event(event, &log);

        assert_eq!(event.inputs.len(), 3);

        // Only the address fields get indexed since total_params > num_indexed_params
        let parsed = event.decode_log(&log, false).unwrap();

        assert_eq!(event.inputs.iter().filter(|param| param.indexed).count(), 2);
        assert_eq!(parsed.indexed[0], DynSolValue::Address(Address::from_word(param0)));
        assert_eq!(parsed.body[0], DynSolValue::Uint(U256::from_be_bytes([3; 32]), 256));
        assert_eq!(parsed.indexed[1], DynSolValue::Address(Address::from_word(param2)));
    }

    #[test]
    fn test_indexed_all() {
        let event = get_event("event Ev(address,uint256,address)").unwrap();

        let param0 = B256::random();
        let param1 = vec![3; 32];
        let param2 = B256::random();
        let log = Log::new_unchecked(
            vec![event.selector(), param0, B256::from_slice(&param1), param2],
            vec![].into(),
        );
        let event = get_indexed_event(event, &log);

        assert_eq!(event.inputs.len(), 3);

        // All parameters get indexed since num_indexed_params == total_params
        assert_eq!(event.inputs.iter().filter(|param| param.indexed).count(), 3);
        let parsed = event.decode_log(&log, false).unwrap();

        assert_eq!(parsed.indexed[0], DynSolValue::Address(Address::from_word(param0)));
        assert_eq!(parsed.indexed[1], DynSolValue::Uint(U256::from_be_bytes([3; 32]), 256));
        assert_eq!(parsed.indexed[2], DynSolValue::Address(Address::from_word(param2)));
    }

    #[test]
    fn test_format_token_addr() {
        // copied from testcases in https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md
        let eip55 = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";
        assert_eq!(
            format_token(&DynSolValue::Address(Address::from_str(&eip55.to_lowercase()).unwrap())),
            eip55.to_string()
        );

        // copied from testcases in https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1191.md
        let eip1191 = "0xFb6916095cA1Df60bb79ce92cE3EA74c37c5d359";
        assert_ne!(
            format_token(&DynSolValue::Address(
                Address::from_str(&eip1191.to_lowercase()).unwrap()
            )),
            eip1191.to_string()
        );
    }
}
