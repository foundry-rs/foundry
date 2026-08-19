//! ABI related helper functions.

use alloy_chains::Chain;
use alloy_dyn_abi::{DynSolType, DynSolValue, FunctionExt, JsonAbiExt};
use alloy_json_abi::{Error, Event, Function, Param};
use alloy_primitives::{Address, LogData, hex, map::HashSet};
use eyre::{Context, ContextCompat, Result};
use foundry_block_explorers::{Client, contract::ContractMetadata, errors::EtherscanError};
use std::pin::Pin;

const MAX_PROXY_DEPTH: usize = 16;

pub fn encode_args<I, S>(inputs: &[Param], args: I) -> Result<Vec<DynSolValue>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<S> = args.into_iter().collect();

    if inputs.len() != args.len() {
        eyre::bail!("encode length mismatch: expected {} types, got {}", inputs.len(), args.len());
    }

    std::iter::zip(inputs, args)
        .map(|(input, arg)| coerce_value(&input.selector_type(), arg.as_ref()))
        .collect()
}

/// Given a function and a vector of string arguments, it proceeds to convert the args to alloy
/// [DynSolValue]s and then ABI encode them, prefixes the encoded data with the function selector.
pub fn encode_function_args<I, S>(func: &Function, args: I) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    Ok(func.abi_encode_input(&encode_args(&func.inputs, args)?)?)
}

/// Given a function and a vector of string arguments, it proceeds to convert the args to alloy
/// [DynSolValue]s and then ABI encode them. Doesn't prefix the function selector.
pub fn encode_function_args_raw<I, S>(func: &Function, args: I) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    Ok(func.abi_encode_input_raw(&encode_args(&func.inputs, args)?)?)
}

/// Given a function and a vector of string arguments, it proceeds to convert the args to alloy
/// [DynSolValue]s and encode them using the packed encoding.
pub fn encode_function_args_packed<I, S>(func: &Function, args: I) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<S> = args.into_iter().collect();

    if func.inputs.len() != args.len() {
        eyre::bail!(
            "encode length mismatch: expected {} types, got {}",
            func.inputs.len(),
            args.len(),
        );
    }

    let params: Vec<Vec<u8>> = std::iter::zip(&func.inputs, args)
        .map(|(input, arg)| coerce_value(&input.selector_type(), arg.as_ref()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|v| v.abi_encode_packed())
        .collect();

    Ok(params.concat())
}

/// Decodes the calldata of the function
pub fn abi_decode_calldata(
    sig: &str,
    calldata: &str,
    input: bool,
    fn_selector: bool,
) -> Result<Vec<DynSolValue>> {
    let func = get_func(sig)?;
    let calldata = hex::decode(calldata)?;

    let mut calldata = calldata.as_slice();
    // If function selector is prefixed in "calldata", remove it (first 4 bytes)
    if input && fn_selector && calldata.len() >= 4 {
        calldata = &calldata[4..];
    }

    let res =
        if input { func.abi_decode_input(calldata) } else { func.abi_decode_output(calldata) }?;

    // in case the decoding worked but nothing was decoded
    if res.is_empty() {
        eyre::bail!("no data was decoded");
    }

    Ok(res)
}

/// Given a function signature string, it tries to parse it as a `Function`
pub fn get_func(sig: &str) -> Result<Function> {
    Function::parse(sig).wrap_err("could not parse function signature")
}

/// Given an event signature string, it tries to parse it as a `Event`
pub fn get_event(sig: &str) -> Result<Event> {
    Event::parse(sig).wrap_err("could not parse event signature")
}

/// Given an error signature string, it tries to parse it as a `Error`
pub fn get_error(sig: &str) -> Result<Error> {
    Error::parse(sig).wrap_err("could not parse error signature")
}

/// Given an event without indexed parameters and a rawlog, it tries to return the event with the
/// proper indexed parameters. Otherwise, it returns the original event.
pub fn get_indexed_event(mut event: Event, raw_log: &LogData) -> Event {
    if !event.anonymous && raw_log.topics().len() > 1 {
        let indexed_params = raw_log.topics().len() - 1;
        let num_inputs = event.inputs.len();
        let num_address_params = event.inputs.iter().filter(|p| p.ty == "address").count();

        event.inputs.iter_mut().enumerate().for_each(|(index, param)| {
            if param.name.is_empty() {
                param.name = format!("param{index}");
            }
            if num_inputs == indexed_params
                || (num_address_params == indexed_params && param.ty == "address")
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
    etherscan_api_url: Option<&str>,
) -> Result<Function> {
    let client = if let Some(api_url) = etherscan_api_url {
        Client::builder().with_api_key(etherscan_api_key).with_api_url(api_url)?.build()?
    } else {
        Client::new(chain, etherscan_api_key)?
    };
    let source = find_source(client, contract).await?;
    let metadata = source.items.first().wrap_err("etherscan returned empty metadata")?;

    let mut abi = metadata.abi()?;
    let funcs = abi.functions.remove(function_name).unwrap_or_default();

    for func in funcs {
        let res = encode_function_args(&func, args);
        if res.is_ok() {
            return Ok(func);
        }
    }

    Err(eyre::eyre!("Function not found in abi"))
}

/// If the code at `address` is a proxy, recurse through its implementations and return metadata for
/// the full chain, with the final implementation first.
pub fn find_source(
    client: Client,
    address: Address,
) -> Pin<Box<dyn Future<Output = Result<ContractMetadata>> + Send>> {
    Box::pin(async move {
        find_source_inner(client, address, HashSet::default(), 0, true).await.map_err(Into::into)
    })
}

/// The same as [`find_source`], but does not report proxy traversal status to the user.
pub fn find_source_quiet(
    client: Client,
    address: Address,
) -> Pin<Box<dyn Future<Output = Result<ContractMetadata, EtherscanError>> + Send>> {
    find_source_inner(client, address, HashSet::default(), 0, false)
}

fn find_source_inner(
    client: Client,
    address: Address,
    mut visited: HashSet<Address>,
    depth: usize,
    report_proxy: bool,
) -> Pin<Box<dyn Future<Output = Result<ContractMetadata, EtherscanError>> + Send>> {
    Box::pin(async move {
        if depth >= MAX_PROXY_DEPTH {
            return Err(EtherscanError::Unknown(format!(
                "proxy chain exceeds maximum depth of {MAX_PROXY_DEPTH}"
            )));
        }
        if !visited.insert(address) {
            return Err(EtherscanError::Unknown(format!("proxy cycle detected at {address}")));
        }

        trace!(%address, "find Etherscan source");
        let source = client.contract_source_code(address).await?;
        let metadata = source
            .items
            .first()
            .ok_or_else(|| EtherscanError::Unknown("Etherscan returned no data".to_string()))?;
        if metadata.proxy == 0 {
            Ok(source)
        } else {
            let implementation = metadata.implementation.ok_or_else(|| {
                EtherscanError::Unknown(format!("proxy at {address} has no implementation address"))
            })?;
            if report_proxy {
                sh_status!(
                    "Contract at {address} is a proxy, trying to fetch source at {implementation}..."
                )
                .map_err(|err| EtherscanError::Unknown(err.to_string()))?;
            }
            match find_source_inner(client, implementation, visited, depth + 1, report_proxy).await
            {
                Ok(mut impl_source) => {
                    impl_source.items.extend(source.items);
                    Ok(impl_source)
                }
                Err(EtherscanError::ContractCodeNotVerified(unverified))
                    if unverified == implementation =>
                {
                    error!(%implementation, "implementation source code not verified");
                    Ok(source)
                }
                Err(err) => Err(err),
            }
        }
    })
}

/// Helper function to coerce a value to a [DynSolValue] given a type string
pub fn coerce_value(ty: &str, arg: &str) -> Result<DynSolValue> {
    let ty = DynSolType::parse(ty)?;
    Ok(DynSolType::coerce_str(&ty, arg)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_dyn_abi::EventExt;
    use alloy_primitives::{B256, U256};
    use axum::{
        Json, Router,
        extract::{Query, State},
        routing::get,
    };
    use serde_json::{Value, json};
    use std::{collections::HashMap as StdHashMap, sync::Arc};
    use tokio::task::JoinHandle;

    fn source_response(name: &str, abi: Value, implementation: Option<Address>) -> Value {
        let mut metadata = json!({
            "SourceCode": "",
            "ABI": abi.to_string(),
            "ContractName": name,
            "CompilerVersion": "v0.8.26+commit.8a97fa7a",
            "OptimizationUsed": "0",
            "OptimizationRuns": "0",
            "ConstructorArguments": "",
            "EVMVersion": "Default",
            "IsProxy": if implementation.is_some() { "1" } else { "0" }
        });
        if let Some(implementation) = implementation {
            metadata["Implementation"] = json!(implementation);
        }
        json!({ "status": "1", "message": "OK", "result": [metadata] })
    }

    async fn explorer_client(responses: StdHashMap<Address, Value>) -> (Client, JoinHandle<()>) {
        async fn handler(
            State(responses): State<Arc<StdHashMap<Address, Value>>>,
            Query(query): Query<StdHashMap<String, String>>,
        ) -> Json<Value> {
            let address = query["address"].parse::<Address>().unwrap();
            Json(responses[&address].clone())
        }

        let app = Router::new().route("/", get(handler)).with_state(Arc::new(responses));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client =
            Client::builder().with_api_url(&url).unwrap().with_url(&url).unwrap().build().unwrap();
        (client, handle)
    }

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
        let log = LogData::new_unchecked(vec![event.selector(), param0, param2], param1.into());
        let event = get_indexed_event(event, &log);

        assert_eq!(event.inputs.len(), 3);

        // Only the address fields get indexed since total_params > num_indexed_params
        let parsed = event.decode_log(&log).unwrap();

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
        let log = LogData::new_unchecked(
            vec![event.selector(), param0, B256::from_slice(&param1), param2],
            vec![].into(),
        );
        let event = get_indexed_event(event, &log);

        assert_eq!(event.inputs.len(), 3);

        // All parameters get indexed since num_indexed_params == total_params
        assert_eq!(event.inputs.iter().filter(|param| param.indexed).count(), 3);
        let parsed = event.decode_log(&log).unwrap();

        assert_eq!(parsed.indexed[0], DynSolValue::Address(Address::from_word(param0)));
        assert_eq!(parsed.indexed[1], DynSolValue::Uint(U256::from_be_bytes([3; 32]), 256));
        assert_eq!(parsed.indexed[2], DynSolValue::Address(Address::from_word(param2)));
    }

    #[test]
    fn test_encode_args_length_validation() {
        use alloy_json_abi::Param;

        let params = vec![
            Param {
                name: "a".to_string(),
                ty: "uint256".to_string(),
                internal_type: None,
                components: vec![],
            },
            Param {
                name: "b".to_string(),
                ty: "address".to_string(),
                internal_type: None,
                components: vec![],
            },
        ];

        // Less arguments than parameters
        let args = vec!["1"];
        let res = encode_args(&params, &args);
        assert!(res.is_err());
        assert!(format!("{}", res.unwrap_err()).contains("encode length mismatch"));

        // Exact number of arguments and parameters
        let args = vec!["1", "0x0000000000000000000000000000000000000001"];
        let res = encode_args(&params, &args);
        assert!(res.is_ok());
        let values = res.unwrap();
        assert_eq!(values.len(), 2);

        // More arguments than parameters
        let args = vec!["1", "0x0000000000000000000000000000000000000001", "extra"];
        let res = encode_args(&params, &args);
        assert!(res.is_err());
        assert!(format!("{}", res.unwrap_err()).contains("encode length mismatch"));
    }

    #[tokio::test]
    async fn find_source_accumulates_proxy_chain_metadata() {
        let proxy = Address::repeat_byte(0x11);
        let implementation = Address::repeat_byte(0x22);
        let responses = StdHashMap::from([
            (
                proxy,
                source_response(
                    "Proxy",
                    json!([{
                        "anonymous": false,
                        "inputs": [],
                        "name": "ProxyEvent",
                        "type": "event"
                    }]),
                    Some(implementation),
                ),
            ),
            (
                implementation,
                source_response(
                    "Implementation",
                    json!([{
                        "anonymous": false,
                        "inputs": [],
                        "name": "ImplementationEvent",
                        "type": "event"
                    }]),
                    None,
                ),
            ),
        ]);
        let (client, server) = explorer_client(responses).await;

        let source = find_source(client, proxy).await.unwrap();
        server.abort();

        assert_eq!(source.items.len(), 2);
        assert_eq!(source.items[0].contract_name, "Implementation");
        assert_eq!(source.items[1].contract_name, "Proxy");
        assert!(source.items.iter().all(|item| item.abi().unwrap().events().count() == 1));
    }

    #[tokio::test]
    async fn find_source_retains_proxy_metadata_for_unverified_implementation() {
        let proxy = Address::repeat_byte(0x11);
        let implementation = Address::repeat_byte(0x22);
        let responses = StdHashMap::from([
            (proxy, source_response("Proxy", json!([]), Some(implementation))),
            (
                implementation,
                json!({
                    "status": "0",
                    "message": "NOTOK",
                    "result": "Contract source code not verified"
                }),
            ),
        ]);
        let (client, server) = explorer_client(responses).await;

        let source = find_source(client, proxy).await.unwrap();
        server.abort();

        assert_eq!(source.items.len(), 1);
        assert_eq!(source.items[0].contract_name, "Proxy");
    }

    #[tokio::test]
    async fn find_source_rejects_proxy_without_implementation() {
        let proxy = Address::repeat_byte(0x11);
        let mut response = source_response("Proxy", json!([]), None);
        response["result"][0]["IsProxy"] = json!("1");
        let (client, server) = explorer_client(StdHashMap::from([(proxy, response)])).await;

        let error = find_source(client, proxy).await.unwrap_err();
        server.abort();

        assert!(error.to_string().contains("has no implementation address"));
    }
}
