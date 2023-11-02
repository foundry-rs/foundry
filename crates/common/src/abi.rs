//! ABI related helper functions.

use alloy_dyn_abi::{DynSolType, DynSolValue, FunctionExt, JsonAbiExt};
use alloy_json_abi::{AbiItem, Event, Function};
use alloy_primitives::{hex, Address, Log};
use ethers_core::types::Chain;
use eyre::{ContextCompat, Result};
use foundry_block_explorers::{contract::ContractMetadata, errors::EtherscanError, Client};
use std::{future::Future, pin::Pin};

/// Given a function and a vector of string arguments, it proceeds to convert the args to alloy
/// [DynSolValue]s and then ABI encode them.
pub fn encode_function_args<I, S>(func: &Function, args: I) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let params = std::iter::zip(&func.inputs, args)
        .map(|(input, arg)| coerce_value(&input.selector_type(), arg.as_ref()))
        .collect::<Result<Vec<_>>>()?;
    func.abi_encode_input(params.as_slice()).map_err(Into::into)
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

    let mut calldata = calldata.as_slice();
    // If function selector is prefixed in "calldata", remove it (first 4 bytes)
    if input && fn_selector && calldata.len() >= 4 {
        calldata = &calldata[4..];
    }

    let res = if input {
        func.abi_decode_input(&calldata, false)?
    } else {
        func.abi_decode_output(&calldata, false)?
    };

    // in case the decoding worked but nothing was decoded
    if res.is_empty() {
        eyre::bail!("no data was decoded")
    }

    Ok(res)
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
    if let Ok(func) = Function::parse(sig) {
        Ok(func)
    } else {
        // Try to parse as human readable ABI.
        let item = match AbiItem::parse(sig) {
            Ok(item) => match item {
                AbiItem::Function(func) => func,
                _ => return Err(eyre::eyre!("Expected function, got {:?}", item)),
            },
            Err(e) => return Err(e.into()),
        };
        Ok(item.into_owned().to_owned())
    }
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
    use super::*;
    use alloy_dyn_abi::EventExt;
    use alloy_primitives::{B256, U256};

    #[test]
    fn test_get_func() {
        let func = get_func("function foo(uint256 a, uint256 b) returns (uint256)");
        assert!(func.is_ok());
        let func = func.unwrap();
        assert_eq!(func.name, "foo");
        assert_eq!(func.inputs.len(), 2);
        assert_eq!(func.inputs[0].ty, "uint256");
        assert_eq!(func.inputs[1].ty, "uint256");

        // Stripped down function, which [Function] can parse.
        let func = get_func("foo(bytes4 a, uint8 b)(bytes4)");
        assert!(func.is_ok());
        let func = func.unwrap();
        assert_eq!(func.name, "foo");
        assert_eq!(func.inputs.len(), 2);
        assert_eq!(func.inputs[0].ty, "bytes4");
        assert_eq!(func.inputs[1].ty, "uint8");
        assert_eq!(func.outputs[0].ty, "bytes4");
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
}
