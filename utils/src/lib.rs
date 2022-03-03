#![doc = include_str!("../README.md")]
use ethers_addressbook::contract;
use ethers_core::{
    abi::{
        self, parse_abi,
        token::{LenientTokenizer, StrictTokenizer, Tokenizer},
        Abi, AbiParser, Event, EventParam, Function, Param, ParamType, Token,
    },
    types::*,
};
use ethers_etherscan::Client;
use ethers_solc::artifacts::{BytecodeObject, CompactBytecode, CompactContractBytecode};
use eyre::{Result, WrapErr};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    env::VarError,
    fmt,
};

use tokio::runtime::{Handle, Runtime};

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum RuntimeOrHandle {
    Runtime(Runtime),
    Handle(Handle),
}

impl Default for RuntimeOrHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeOrHandle {
    pub fn new() -> RuntimeOrHandle {
        match Handle::try_current() {
            Ok(handle) => RuntimeOrHandle::Handle(handle),
            Err(_) => RuntimeOrHandle::Runtime(Runtime::new().expect("Failed to start runtime")),
        }
    }

    pub fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        match &self {
            RuntimeOrHandle::Runtime(runtime) => runtime.block_on(f),
            RuntimeOrHandle::Handle(handle) => tokio::task::block_in_place(|| handle.block_on(f)),
        }
    }
}

pub enum SelectorOrSig {
    Selector(String),
    Sig(Vec<String>),
}

pub struct PossibleSigs {
    method: SelectorOrSig,
    data: Vec<String>,
}
impl PossibleSigs {
    fn new() -> Self {
        PossibleSigs { method: SelectorOrSig::Selector("0x00000000".to_string()), data: vec![] }
    }
}
impl fmt::Display for PossibleSigs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.method {
            SelectorOrSig::Selector(selector) => {
                writeln!(f, "\n Method: {}", selector)?;
            }
            SelectorOrSig::Sig(sigs) => {
                writeln!(f, "\n Possible methods:")?;
                for sig in sigs {
                    writeln!(f, " - {}", sig)?;
                }
            }
        }

        writeln!(f, " ------------")?;
        for (i, row) in self.data.iter().enumerate() {
            let pad = if i < 10 { "  " } else { " " };
            writeln!(f, " [{}]:{}{}", i, pad, row)?;
        }
        Ok(())
    }
}

/// Recursively links bytecode given a target contract artifact name, the bytecode(s) to be linked,
/// a mapping of contract artifact name to bytecode, a dependency mapping, a mutable list that
/// will be filled with the predeploy libraries, initial nonce, and the sender.
pub fn recurse_link<'a>(
    // target name
    target: String,
    // to-be-modified/linked bytecode
    target_bytecode: (&'a mut CompactBytecode, &'a mut CompactBytecode),
    // Contracts
    contracts: &'a BTreeMap<String, CompactContractBytecode>,
    // fname => Vec<(fname, file, key)>
    dependency_tree: &'a BTreeMap<String, Vec<(String, String, String)>>,
    // library deployment vector
    deployment: &'a mut Vec<ethers_core::types::Bytes>,
    // nonce to start at
    init_nonce: U256,
    // sender
    sender: Address,
) {
    // check if we have dependencies
    if let Some(dependencies) = dependency_tree.get(&target) {
        // for each dependency, try to link
        dependencies.iter().for_each(|(next_target, file, key)| {
            // get the dependency
            let contract = contracts.get(next_target).expect("No target contract").clone();
            let mut next_target_bytecode = contract.bytecode.expect("No target bytecode");
            let mut next_target_runtime_bytecode = contract
                .deployed_bytecode
                .expect("No target runtime bytecode")
                .bytecode
                .expect("No target runtime");

            // make sure dependency is fully linked
            if let Some(deps) = dependency_tree.get(&target) {
                if !deps.is_empty() {
                    // actually link the nested dependencies to this dependency
                    recurse_link(
                        next_target.to_string(),
                        (&mut next_target_bytecode, &mut next_target_runtime_bytecode),
                        contracts,
                        dependency_tree,
                        deployment,
                        init_nonce,
                        sender,
                    );
                }
            }

            // calculate the address for linking this dependency
            let addr =
                ethers_core::utils::get_contract_address(sender, init_nonce + deployment.len());

            // link the dependency to the target
            target_bytecode.0.link(file.clone(), key.clone(), addr);
            target_bytecode.1.link(file, key, addr);

            // push the dependency into the library deployment vector
            deployment
                .push(next_target_bytecode.object.into_bytes().expect("Bytecode should be linked"));
        });
    }
}

const BASE_TX_COST: u64 = 21000;

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
        AbiParser::default()
            .parse_function(self)
            .unwrap_or_else(|_| panic!("could not convert {} to function", self))
    }
}

/// Given a gas value and a calldata array, it subtracts the calldata cost from the
/// gas value, as well as the 21k base gas cost for all transactions.
pub fn remove_extra_costs(gas: U256, calldata: &[u8]) -> U256 {
    let mut calldata_cost = 0;
    for i in calldata {
        if *i != 0 {
            // TODO: Check if EVM pre-eip2028 and charge 64
            // GTXDATANONZERO = 16
            calldata_cost += 16
        } else {
            // GTXDATAZERO = 4
            calldata_cost += 4;
        }
    }
    gas.saturating_sub(calldata_cost.into()).saturating_sub(BASE_TX_COST.into())
}

/// Flattens a group of contracts into maps of all events and functions
pub fn flatten_known_contracts(
    contracts: &BTreeMap<String, (Abi, Vec<u8>)>,
) -> (BTreeMap<[u8; 4], Function>, BTreeMap<H256, Event>, Abi) {
    let flattened_funcs: BTreeMap<[u8; 4], Function> = contracts
        .iter()
        .flat_map(|(_name, (abi, _code))| {
            abi.functions()
                .map(|func| (func.short_signature(), func.clone()))
                .collect::<BTreeMap<[u8; 4], Function>>()
        })
        .collect();

    let flattened_events: BTreeMap<H256, Event> = contracts
        .iter()
        .flat_map(|(_name, (abi, _code))| {
            abi.events()
                .map(|event| (event.signature(), event.clone()))
                .collect::<BTreeMap<H256, Event>>()
        })
        .collect();

    // We need this for better revert decoding, and want it in abi form
    let mut errors_abi = Abi::default();
    contracts.iter().for_each(|(_name, (abi, _code))| {
        abi.errors().for_each(|error| {
            let entry =
                errors_abi.errors.entry(error.name.clone()).or_insert_with(Default::default);
            entry.push(error.clone());
        });
    });
    (flattened_funcs, flattened_events, errors_abi)
}

/// Given an ABI encoded error string with the function signature `Error(string)`, it decodes
/// it and returns the revert error message.
pub fn decode_revert(error: &[u8], maybe_abi: Option<&Abi>) -> Result<String> {
    if error.len() >= 4 {
        match error[0..4] {
            // keccak(Panic(uint256))
            [78, 72, 123, 113] => {
                // ref: https://soliditydeveloper.com/solidity-0.8
                match error[error.len() - 1] {
                    1 => {
                        // assert
                        Ok("Assertion violated".to_string())
                    }
                    17 => {
                        // safemath over/underflow
                        Ok("Arithmetic over/underflow".to_string())
                    }
                    18 => {
                        // divide by 0
                        Ok("Division or modulo by 0".to_string())
                    }
                    33 => {
                        // conversion into non-existent enum type
                        Ok("Conversion into non-existent enum type".to_string())
                    }
                    34 => {
                        // incorrectly encoded storage byte array
                        Ok("Incorrectly encoded storage byte array".to_string())
                    }
                    49 => {
                        // pop() on empty array
                        Ok("`pop()` on empty array".to_string())
                    }
                    50 => {
                        // index out of bounds
                        Ok("Index out of bounds".to_string())
                    }
                    65 => {
                        // allocating too much memory or creating too large array
                        Ok("Memory allocation overflow".to_string())
                    }
                    81 => {
                        // calling a zero initialized variable of internal function type
                        Ok("Calling a zero initialized variable of internal function type"
                            .to_string())
                    }
                    _ => Err(eyre::Error::msg("Unsupported solidity builtin panic")),
                }
            }
            // keccak(Error(string))
            [8, 195, 121, 160] => {
                if let Ok(decoded) = abi::decode(&[abi::ParamType::String], &error[4..]) {
                    Ok(decoded[0].to_string())
                } else {
                    Err(eyre::Error::msg("Bad string decode"))
                }
            }
            // keccak(expectRevert(bytes))
            [242, 141, 206, 179] => {
                let err_data = &error[4..];
                if err_data.len() > 64 {
                    let len = U256::from(&err_data[32..64]).as_usize();
                    if err_data.len() > 64 + len {
                        let actual_err = &err_data[64..64 + len];
                        if let Ok(decoded) = decode_revert(actual_err, maybe_abi) {
                            // check if its a builtin
                            return Ok(decoded)
                        } else if let Ok(as_str) = String::from_utf8(actual_err.to_vec()) {
                            // check if its a true string
                            return Ok(as_str)
                        }
                    }
                }
                Err(eyre::Error::msg("Non-native error and not string"))
            }
            _ => {
                // try to decode a custom error if provided an abi
                if error.len() >= 4 {
                    if let Some(abi) = maybe_abi {
                        for abi_error in abi.errors() {
                            if abi_error.signature()[0..4] == error[0..4] {
                                // if we dont decode, dont return an error, try to decode as a
                                // string later
                                if let Ok(decoded) = abi_error.decode(&error[4..]) {
                                    let inputs = decoded
                                        .iter()
                                        .map(format_token)
                                        .collect::<Vec<String>>()
                                        .join(", ");
                                    return Ok(format!("{}({})", abi_error.name, inputs))
                                }
                            }
                        }
                    }
                }
                // evm_error will sometimes not include the function selector for the error,
                // optimistically try to decode
                if let Ok(decoded) = abi::decode(&[abi::ParamType::String], error) {
                    Ok(decoded[0].to_string())
                } else {
                    Err(eyre::Error::msg("Non-native error and not string"))
                }
            }
        }
    } else {
        Err(eyre::Error::msg("Not enough error data to decode"))
    }
}

/// Given a k/v serde object, it pretty prints its keys and values as a table.
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

/// Given a function signature string, it tries to parse it as a `Function`
pub fn get_func(sig: &str) -> Result<Function> {
    // TODO: Make human readable ABI better / more minimal
    let abi = parse_abi(&[sig])?;
    // get the function
    let (_, func) =
        abi.functions.iter().next().ok_or_else(|| eyre::eyre!("function name not found"))?;
    let func = func.get(0).ok_or_else(|| eyre::eyre!("functions array empty"))?;
    Ok(func.clone())
}

// Given a function name, address, and args, tries to parse it as a `Function` by fetching the
// abi from etherscan. If the address is a proxy, fetches the ABI of the implementation contract.
pub async fn get_func_etherscan(
    function_name: &str,
    contract: Address,
    args: &[String],
    chain: Chain,
    etherscan_api_key: String,
) -> Result<Function> {
    let client = Client::new(chain, etherscan_api_key)?;
    let metadata = &client.contract_source_code(contract).await?.items[0];

    let abi = if metadata.implementation.is_empty() {
        serde_json::from_str(&metadata.abi)?
    } else {
        let implementation = metadata.implementation.parse::<Address>()?;
        client.contract_abi(implementation).await?
    };

    let empty = vec![];
    let funcs = abi.functions.get(function_name).unwrap_or(&empty);

    for func in funcs {
        let res = encode_args(func, args);
        if res.is_ok() {
            return Ok(func.clone())
        }
    }

    Err(eyre::eyre!("Function not found in abi"))
}

/// Parses string input as Token against the expected ParamType
pub fn parse_tokens<'a, I: IntoIterator<Item = (&'a ParamType, &'a str)>>(
    params: I,
    lenient: bool,
) -> eyre::Result<Vec<Token>> {
    params
        .into_iter()
        .map(|(param, value)| {
            if lenient {
                LenientTokenizer::tokenize(param, value)
            } else {
                StrictTokenizer::tokenize(param, value)
            }
        })
        .collect::<Result<_, _>>()
        .wrap_err("Failed to parse tokens")
}

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

/// Fetches a function signature given the selector using 4byte.directory
pub async fn fourbyte(selector: &str) -> Result<Vec<(String, i32)>> {
    #[derive(Deserialize)]
    struct Decoded {
        text_signature: String,
        id: i32,
    }

    #[derive(Deserialize)]
    struct ApiResponse {
        results: Vec<Decoded>,
    }

    let selector = &selector.strip_prefix("0x").unwrap_or(selector);
    if selector.len() < 8 {
        return Err(eyre::eyre!("Invalid selector"))
    }
    let selector = &selector[..8];

    let url = format!("https://www.4byte.directory/api/v1/signatures/?hex_signature={}", selector);
    let res = reqwest::get(url).await?;
    let res = res.text().await?;
    let api_response = match serde_json::from_str::<ApiResponse>(&res) {
        Ok(inner) => inner,
        Err(err) => {
            eyre::bail!("Could not decode response:\n {}.\nError: {}", res, err)
        }
    };

    if api_response.results.is_empty() {
        eyre::bail!("no signature found for provided function selector")
    }

    Ok(api_response
        .results
        .into_iter()
        .map(|d| (d.text_signature, d.id))
        .collect::<Vec<(String, i32)>>())
}

pub async fn fourbyte_possible_sigs(calldata: &str, id: Option<String>) -> Result<Vec<String>> {
    let mut sigs = fourbyte(calldata).await?;

    match id {
        Some(id) => {
            let sig = match &id[..] {
                "earliest" => {
                    sigs.sort_by(|a, b| a.1.cmp(&b.1));
                    sigs.get(0)
                }
                "latest" => {
                    sigs.sort_by(|a, b| b.1.cmp(&a.1));
                    sigs.get(0)
                }
                _ => {
                    let id: i32 = id.parse().expect("Must be integer");
                    sigs = sigs
                        .iter()
                        .filter(|sig| sig.1 == id)
                        .cloned()
                        .collect::<Vec<(String, i32)>>();
                    sigs.get(0)
                }
            };
            match sig {
                Some(sig) => Ok(vec![sig.clone().0]),
                None => Ok(vec![]),
            }
        }
        None => {
            // filter for signatures that can be decoded
            Ok(sigs
                .iter()
                .map(|sig| sig.clone().0)
                .filter(|sig| {
                    let res = abi_decode(sig, calldata, true);
                    res.is_ok()
                })
                .collect::<Vec<String>>())
        }
    }
}

/// Fetches a event signature given the 32 byte topic using 4byte.directory
pub async fn fourbyte_event(topic: &str) -> Result<Vec<(String, i32)>> {
    #[derive(Deserialize)]
    struct Decoded {
        text_signature: String,
        id: i32,
    }

    #[derive(Deserialize)]
    struct ApiResponse {
        results: Vec<Decoded>,
    }

    let topic = &topic.strip_prefix("0x").unwrap_or(topic);
    if topic.len() < 64 {
        return Err(eyre::eyre!("Invalid topic"))
    }
    let topic = &topic[..8];

    let url =
        format!("https://www.4byte.directory/api/v1/event-signatures/?hex_signature={}", topic);
    let res = reqwest::get(url).await?;
    let api_response = res.json::<ApiResponse>().await?;

    Ok(api_response
        .results
        .into_iter()
        .map(|d| (d.text_signature, d.id))
        .collect::<Vec<(String, i32)>>())
}

/// Pretty print calldata and if available, fetch possible function signatures
///
/// ```no_run
/// 
/// use foundry_utils::pretty_calldata;
///
/// # async fn foo() -> eyre::Result<()> {
///   let pretty_data = pretty_calldata("0x70a08231000000000000000000000000d0074f4e6490ae3f888d1d4f7e3e43326bd3f0f5".to_string(), false).await?;
///   println!("{}",pretty_data);
/// # Ok(())
/// # }
/// ```

pub async fn pretty_calldata(calldata: impl AsRef<str>, offline: bool) -> Result<PossibleSigs> {
    let mut possible_info = PossibleSigs::new();
    let calldata = calldata.as_ref().trim_start_matches("0x");

    let selector =
        calldata.get(..8).ok_or_else(|| eyre::eyre!("calldata cannot be less that 4 bytes"))?;

    let sigs = if offline {
        vec![]
    } else {
        fourbyte(selector).await?.into_iter().map(|sig| sig.0).collect()
    };
    let (_, data) = calldata.split_at(8);

    if data.len() % 64 != 0 {
        eyre::bail!("\nInvalid calldata size")
    }

    let row_length = data.len() / 64;

    for row in 0..row_length {
        possible_info.data.push(data[64 * row..64 * (row + 1)].to_string());
    }
    if sigs.is_empty() {
        possible_info.method = SelectorOrSig::Selector(selector.to_string());
    } else {
        possible_info.method = SelectorOrSig::Sig(sigs);
    }
    Ok(possible_info)
}

pub fn abi_decode(sig: &str, calldata: &str, input: bool) -> Result<Vec<Token>> {
    let func = IntoFunction::into(sig);
    let calldata = calldata.strip_prefix("0x").unwrap_or(calldata);
    let calldata = hex::decode(calldata)?;
    let res = if input {
        // need to strip the function selector
        func.decode_input(&calldata[4..])?
    } else {
        func.decode_output(&calldata)?
    };

    // in case the decoding worked but nothing was decoded
    if res.is_empty() {
        eyre::bail!("no data was decoded")
    }

    Ok(res)
}

/// Resolves an input to [`NameOrAddress`]. The input could also be a contract/token name supported
/// by
/// [`ethers-addressbook`](https://github.com/gakonst/ethers-rs/tree/master/ethers-addressbook).
pub fn resolve_addr<T: Into<NameOrAddress>>(to: T, chain: Chain) -> eyre::Result<NameOrAddress> {
    Ok(match to.into() {
        NameOrAddress::Address(addr) => NameOrAddress::Address(addr),
        NameOrAddress::Name(contract_or_ens) => {
            if let Some(contract) = contract(&contract_or_ens) {
                NameOrAddress::Address(contract.address(chain).ok_or_else(|| {
                    eyre::eyre!(
                        "contract: {} not found in addressbook for network: {}",
                        contract_or_ens,
                        chain
                    )
                })?)
            } else {
                NameOrAddress::Name(contract_or_ens)
            }
        }
    })
}

/// Pretty print a slice of tokens.
pub fn format_tokens(tokens: &[Token]) -> impl Iterator<Item = String> + '_ {
    tokens.iter().map(format_token)
}

// Gets pretty print strings for tokens
pub fn format_token(param: &Token) -> String {
    match param {
        Token::Address(addr) => format!("{:?}", addr),
        Token::FixedBytes(bytes) => format!("0x{}", hex::encode(&bytes)),
        Token::Bytes(bytes) => format!("0x{}", hex::encode(&bytes)),
        Token::Int(mut num) => {
            if num.bit(255) {
                num = num - 1;
                format!("-{}", num.overflowing_neg().0)
            } else {
                num.to_string()
            }
        }
        Token::Uint(num) => num.to_string(),
        Token::Bool(b) => format!("{}", b),
        Token::String(s) => format!("{:?}", s),
        Token::FixedArray(tokens) => {
            let string = tokens.iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("[{}]", string)
        }
        Token::Array(tokens) => {
            let string = tokens.iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("[{}]", string)
        }
        Token::Tuple(tokens) => {
            let string = tokens.iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("({})", string)
        }
    }
}
/// Reads the `ETHERSCAN_API_KEY` env variable
pub fn etherscan_api_key() -> eyre::Result<String> {
    std::env::var("ETHERSCAN_API_KEY").map_err(|err| match err {
        VarError::NotPresent => {
            eyre::eyre!(
                r#"
  You need an Etherscan Api Key to verify contracts.
  Create one at https://etherscan.io/myapikey
  Then export it with \`export ETHERSCAN_API_KEY=xxxxxxxx'"#
            )
        }
        VarError::NotUnicode(err) => {
            eyre::eyre!("Invalid `ETHERSCAN_API_KEY`: {:?}", err)
        }
    })
}

// Helper for generating solidity abi encoder v2 field names.
const ASCII_LOWER: [char; 26] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z',
];

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().chain(c).collect(),
    }
}

// Returns the function parameter formatted as a string, as well as inserts into the provided
// `structs` set in order to create type definitions for any Abi Encoder v2 structs.
fn format_param(param: &Param, structs: &mut HashSet<String>) -> String {
    let kind = get_param_type(&param.kind, &param.name, param.internal_type.as_deref(), structs);

    // add `memory` if required (not needed for events, only for functions)
    let is_memory = matches!(
        param.kind,
        ParamType::Array(_) |
            ParamType::Bytes |
            ParamType::String |
            ParamType::FixedArray(_, _) |
            ParamType::Tuple(_),
    );
    let kind = if is_memory { format!("{} memory", kind) } else { kind };

    if param.name.is_empty() {
        kind
    } else {
        format!("{} {}", kind, param.name)
    }
}

fn format_event_params(param: &EventParam, structs: &mut HashSet<String>) -> String {
    let kind = get_param_type(&param.kind, &param.name, None, structs);

    if param.name.is_empty() {
        kind
    } else if param.indexed {
        format!("{} indexed {}", kind, param.name)
    } else {
        format!("{} {}", kind, param.name)
    }
}

fn get_param_type(
    kind: &ParamType,
    name: &str,
    internal_type: Option<&str>,
    structs: &mut HashSet<String>,
) -> String {
    let (kind, v2_struct) = match kind {
        // We need to do some extra work to parse ABI Encoder V2 types.
        ParamType::Tuple(ref args) => {
            let name = internal_type.map(|x| x.to_owned()).unwrap_or_else(|| capitalize(name));
            let name = if name.contains('.') {
                name.split('.').nth(1).expect("could not get struct name").to_owned()
            } else {
                name
            };

            // NB: This does not take into account recursive ABI Encoder v2 structs. Left
            // as future work.
            let args = args
                .iter()
                .enumerate()
                // Unfortunately Solidity does not support unnamed struct fields, so we
                // just codegen ones alphabetically.
                .map(|(i, x)| format!("{} {};", x, ASCII_LOWER[i]))
                .collect::<Vec<_>>()
                .join(" ");

            let v2_struct = format!("struct {} {{ {} }}", name, args);
            (name, Some(v2_struct))
        }
        // If not, just get the string of the param kind.
        _ => (kind.to_string(), None),
    };

    // if there was a v2 struct, push it for later usage
    if let Some(v2_struct) = v2_struct {
        structs.insert(v2_struct);
    }

    kind
}

/// This function takes a contract [`Abi`] and a name and proceeds to generate a Solidity
/// `interface` from that ABI. If the provided name is empty, then it defaults to `interface
/// Interface`.
///
/// This is done by iterating over the functions and their ABI inputs/outputs, and generating
/// function signatures/inputs/outputs according to the ABI.
///
/// Notes:
/// * ABI Encoder V2 is not supported yet
/// * Kudos to https://github.com/maxme/abi2solidity for the algorithm
pub fn abi_to_solidity(contract_abi: &Abi, mut contract_name: &str) -> Result<String> {
    let functions_iterator = contract_abi.functions();
    let events_iterator = contract_abi.events();
    if contract_name.trim().is_empty() {
        contract_name = "Interface";
    };

    // instantiate an array of all ABI Encoder v2 structs
    let mut structs = HashSet::new();

    let events = events_iterator
        .map(|event| {
            let inputs = event
                .inputs
                .iter()
                .map(|param| format_event_params(param, &mut structs))
                .collect::<Vec<String>>()
                .join(", ");

            let event_final = format!("event {}({})", event.name, inputs);
            format!("{};", event_final)
        })
        .collect::<Vec<_>>()
        .join("\n    ");

    let functions = functions_iterator
        .map(|function| {
            let inputs = function
                .inputs
                .iter()
                .map(|param| format_param(param, &mut structs))
                .collect::<Vec<String>>()
                .join(", ");
            let outputs = function
                .outputs
                .iter()
                .map(|param| format_param(param, &mut structs))
                .collect::<Vec<String>>()
                .join(", ");

            let mutability = match function.state_mutability {
                abi::StateMutability::Pure => "pure",
                abi::StateMutability::View => "view",
                abi::StateMutability::Payable => "payable",
                _ => "",
            };

            let mut func = format!("function {}({})", function.name, inputs);
            if !mutability.is_empty() {
                func = format!("{} {}", func, mutability);
            }
            func = format!("{} external", func);
            if !outputs.is_empty() {
                func = format!("{} returns ({})", func, outputs);
            }
            format!("{};", func)
        })
        .collect::<Vec<_>>()
        .join("\n    ");

    Ok(if structs.is_empty() {
        match events.is_empty() {
            true => format!(
                r#"interface {} {{
    {}
}}
"#,
                contract_name, functions
            ),
            false => format!(
                r#"interface {} {{
    {}

    {}
}}
"#,
                contract_name, events, functions
            ),
        }
    } else {
        let structs = structs.into_iter().collect::<Vec<_>>().join("\n    ");
        match events.is_empty() {
            true => format!(
                r#"interface {} {{
    {}

    {}
}}
"#,
                contract_name, structs, functions
            ),
            false => format!(
                r#"interface {} {{
    {}

    {}

    {}
}}
"#,
                contract_name, events, structs, functions
            ),
        }
    })
}

#[derive(Debug)]
pub struct PostLinkInput<'a, T, U> {
    pub contract: CompactContractBytecode,
    pub known_contracts: &'a mut BTreeMap<String, T>,
    pub fname: String,
    pub extra: &'a mut U,
    pub dependencies: Vec<ethers_core::types::Bytes>,
}

pub fn link<T, U>(
    contracts: &BTreeMap<String, CompactContractBytecode>,
    known_contracts: &mut BTreeMap<String, T>,
    sender: Address,
    extra: &mut U,
    link_key_construction: impl Fn(String, String) -> (String, String, String),
    post_link: impl Fn(PostLinkInput<T, U>) -> eyre::Result<()>,
) -> eyre::Result<()> {
    // we dont use mainnet state for evm_opts.sender so this will always be 1
    // I am leaving this here so that in the future if this needs to change,
    // its easy to find.
    let nonce = U256::one();

    // create a mapping of fname => Vec<(fname, file, key)>,
    let link_tree: BTreeMap<String, Vec<(String, String, String)>> = contracts
        .iter()
        .map(|(fname, contract)| {
            (
                fname.to_string(),
                contract
                    .all_link_references()
                    .iter()
                    .flat_map(|(file, link)| {
                        link.keys()
                            .map(|key| link_key_construction(file.to_string(), key.to_string()))
                    })
                    .collect::<Vec<(String, String, String)>>(),
            )
        })
        .collect();

    for fname in contracts.keys() {
        let (abi, maybe_deployment_bytes, maybe_runtime) = if let Some(c) = contracts.get(fname) {
            (c.abi.as_ref(), c.bytecode.as_ref(), c.deployed_bytecode.as_ref())
        } else {
            (None, None, None)
        };
        if let (Some(abi), Some(bytecode), Some(runtime)) =
            (abi, maybe_deployment_bytes, maybe_runtime)
        {
            // we are going to mutate, but library contract addresses may change based on
            // the test so we clone
            let mut target_bytecode = bytecode.clone();
            let mut rt = runtime.clone();
            let mut target_bytecode_runtime = rt.bytecode.expect("No target runtime").clone();

            // instantiate a vector that gets filled with library deployment bytecode
            let mut dependencies = vec![];

            match bytecode.object {
                BytecodeObject::Unlinked(_) => {
                    // link needed
                    recurse_link(
                        fname.to_string(),
                        (&mut target_bytecode, &mut target_bytecode_runtime),
                        contracts,
                        &link_tree,
                        &mut dependencies,
                        nonce,
                        sender,
                    );
                }
                BytecodeObject::Bytecode(ref bytes) => {
                    if bytes.as_ref().is_empty() {
                        // abstract, skip
                        continue
                    }
                }
            }

            rt.bytecode = Some(target_bytecode_runtime);
            let tc = CompactContractBytecode {
                abi: Some(abi.clone()),
                bytecode: Some(target_bytecode),
                deployed_bytecode: Some(rt),
            };

            let post_link_input = PostLinkInput {
                contract: tc,
                known_contracts,
                fname: fname.to_string(),
                extra,
                dependencies,
            };

            post_link(post_link_input)?;
        }
    }
    Ok(())
}

/// Enables tracing
#[cfg(any(feature = "test"))]
pub fn init_tracing_subscriber() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::{
        abi::Abi,
        solc::{artifacts::CompactContractBytecode, Project, ProjectPathsConfig},
        types::{Address, Bytes},
    };
    use std::path::PathBuf;

    #[test]
    #[ignore] // TODO: This needs to be re-enabled one it's fixed in ethers-solc.
    fn test_linking() {
        let lib_test_json_lib_test = "6101d1610053600b82828239805160001a607314610046577f4e487b7100000000000000000000000000000000000000000000000000000000600052600060045260246000fd5b30600052607381538281f3fe73000000000000000000000000000000000000000030146080604052600436106100355760003560e01c806314ba3f121461003a575b600080fd5b610054600480360381019061004f91906100bb565b61006a565b60405161006191906100f7565b60405180910390f35b60006064826100799190610141565b9050919050565b600080fd5b6000819050919050565b61009881610085565b81146100a357600080fd5b50565b6000813590506100b58161008f565b92915050565b6000602082840312156100d1576100d0610080565b5b60006100df848285016100a6565b91505092915050565b6100f181610085565b82525050565b600060208201905061010c60008301846100e8565b92915050565b7f4e487b7100000000000000000000000000000000000000000000000000000000600052601160045260246000fd5b600061014c82610085565b915061015783610085565b9250817fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff04831182151516156101905761018f610112565b5b82820290509291505056fea264697066735822122089bbb5614fb9e62f207b40682b397b25f2000c514857bf7959055b0d9b5dcfbf64736f6c634300080b0033";
        let lib_test_nested_json_lib_test_nested = "610266610053600b82828239805160001a607314610046577f4e487b7100000000000000000000000000000000000000000000000000000000600052600060045260246000fd5b30600052607381538281f3fe73000000000000000000000000000000000000000030146080604052600436106100355760003560e01c80639acc23361461003a575b600080fd5b610054600480360381019061004f9190610116565b61006a565b604051610061919061015c565b60405180910390f35b60007347e9fbef8c83a1714f1951f142132e6e90f5fa5d6314ba3f1260656040518263ffffffff1660e01b81526004016100a491906101bc565b602060405180830381865af41580156100c1573d6000803e3d6000fd5b505050506040513d601f19601f820116820180604052508101906100e59190610203565b9050919050565b600080fd5b600381106100fe57600080fd5b50565b600081359050610110816100f1565b92915050565b60006020828403121561012c5761012b6100ec565b5b600061013a84828501610101565b91505092915050565b6000819050919050565b61015681610143565b82525050565b6000602082019050610171600083018461014d565b92915050565b6000819050919050565b6000819050919050565b60006101a66101a161019c84610177565b610181565b610143565b9050919050565b6101b68161018b565b82525050565b60006020820190506101d160008301846101ad565b92915050565b6101e081610143565b81146101eb57600080fd5b50565b6000815190506101fd816101d7565b92915050565b600060208284031215610219576102186100ec565b5b6000610227848285016101ee565b9150509291505056fea26469706673582212204d96467c5d42f97ecaa460cca5137364d61ae850ee0be7c2d9c5ffb045bf8dc364736f6c634300080b0033";
        let contract_names = [
            "DsTestMini.json:DsTestMini",
            "LibLinkingTest.json:LibLinkingTest",
            "LibTest.json:LibTest",
            "LibTestNested.json:LibTestNested",
            "Main.json:Main",
        ];

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/linking");
        let paths = ProjectPathsConfig::builder().root(&root).sources(&root).build().unwrap();

        let project = Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();

        let output = project.compile().unwrap();
        let contracts = output
            .into_artifacts()
            .map(|(i, c)| (i.slug(), c.into_contract_bytecode()))
            .collect::<BTreeMap<String, CompactContractBytecode>>();

        let mut known_contracts: BTreeMap<String, (Abi, Vec<u8>)> = Default::default();
        let mut deployable_contracts: BTreeMap<String, (Abi, Bytes, Vec<Bytes>)> =
            Default::default();

        assert_eq!(&contracts.keys().collect::<Vec<&String>>()[..], &contract_names[..]);

        link(
            &contracts,
            &mut known_contracts,
            Address::default(),
            &mut deployable_contracts,
            |file, key| (format!("{}.json:{}", key, key), file, key),
            |post_link_input| {
                match post_link_input.fname.as_str() {
                    "DsTestMini.json:DsTestMini" => {
                        assert_eq!(post_link_input.dependencies.len(), 0);
                    }
                    "LibLinkingTest.json:LibLinkingTest" => {
                        assert_eq!(post_link_input.dependencies.len(), 3);
                        assert_eq!(
                            hex::encode(post_link_input.dependencies[0].clone()),
                            lib_test_json_lib_test
                        );
                        assert_eq!(
                            hex::encode(post_link_input.dependencies[1].clone()),
                            lib_test_json_lib_test
                        );
                        assert_eq!(
                            hex::encode(post_link_input.dependencies[2].clone()),
                            lib_test_nested_json_lib_test_nested
                        );
                    }
                    "LibTest.json:LibTest" => {
                        assert_eq!(post_link_input.dependencies.len(), 0);
                    }
                    "LibTestNested.json:LibTestNested" => {
                        assert_eq!(post_link_input.dependencies.len(), 1);
                        assert_eq!(
                            hex::encode(post_link_input.dependencies[0].clone()),
                            lib_test_json_lib_test
                        );
                    }
                    "Main.json:Main" => {
                        assert_eq!(post_link_input.dependencies.len(), 3);
                        assert_eq!(
                            hex::encode(post_link_input.dependencies[0].clone()),
                            lib_test_json_lib_test
                        );
                        assert_eq!(
                            hex::encode(post_link_input.dependencies[1].clone()),
                            lib_test_json_lib_test
                        );
                        assert_eq!(
                            hex::encode(post_link_input.dependencies[2].clone()),
                            lib_test_nested_json_lib_test_nested
                        );
                    }
                    _ => assert!(false),
                }
                Ok(())
            },
        )
        .unwrap();
    }

    #[test]
    fn test_resolve_addr() {
        use std::str::FromStr;

        // DAI:mainnet exists in ethers-addressbook (0x6b175474e89094c44da98b954eedeac495271d0f)
        assert_eq!(
            resolve_addr(NameOrAddress::Name("dai".to_string()), Chain::Mainnet).ok(),
            Some(NameOrAddress::Address(
                Address::from_str("0x6b175474e89094c44da98b954eedeac495271d0f").unwrap()
            ))
        );

        // DAI:goerli exists in ethers-adddressbook (0x11fE4B6AE13d2a6055C8D9cF65c55bac32B5d844)
        assert_eq!(
            resolve_addr(NameOrAddress::Name("dai".to_string()), Chain::Goerli).ok(),
            Some(NameOrAddress::Address(
                Address::from_str("0x11fE4B6AE13d2a6055C8D9cF65c55bac32B5d844").unwrap()
            ))
        );

        // DAI:moonbean does not exist in addressbook
        assert!(resolve_addr(NameOrAddress::Name("dai".to_string()), Chain::MoonbeamDev).is_err());

        // If not present in addressbook, gets resolved to an ENS name.
        assert_eq!(
            resolve_addr(NameOrAddress::Name("contractnotpresent".to_string()), Chain::Mainnet)
                .ok(),
            Some(NameOrAddress::Name("contractnotpresent".to_string())),
        );

        // Nothing to resolve for an address.
        assert_eq!(
            resolve_addr(NameOrAddress::Address(Address::zero()), Chain::Mainnet).ok(),
            Some(NameOrAddress::Address(Address::zero())),
        );
    }

    #[tokio::test]
    async fn test_fourbyte() {
        let sigs = fourbyte("0xa9059cbb").await.unwrap();
        assert_eq!(sigs[0].0, "func_2093253501(bytes)".to_string());
        assert_eq!(sigs[0].1, 313067);
    }

    #[tokio::test]
    async fn test_fourbyte_possible_sigs() {
        let sigs = fourbyte_possible_sigs("0xa9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79", None).await.unwrap();
        assert_eq!(sigs[0], "many_msg_babbage(bytes1)".to_string());
        assert_eq!(sigs[1], "transfer(address,uint256)".to_string());

        let sigs = fourbyte_possible_sigs("0xa9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79", Some("earliest".to_string())).await.unwrap();
        assert_eq!(sigs[0], "transfer(address,uint256)".to_string());

        let sigs = fourbyte_possible_sigs("0xa9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79", Some("latest".to_string())).await.unwrap();
        assert_eq!(sigs[0], "func_2093253501(bytes)".to_string());

        let sigs = fourbyte_possible_sigs("0xa9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79", Some("145".to_string())).await.unwrap();
        assert_eq!(sigs[0], "transfer(address,uint256)".to_string());
    }

    #[tokio::test]
    async fn test_fourbyte_event() {
        let sigs =
            fourbyte_event("0x7e1db2a1cd12f0506ecd806dba508035b290666b84b096a87af2fd2a1516ede6")
                .await
                .unwrap();
        assert_eq!(sigs[0].0, "updateAuthority(address,uint8)".to_string());
        assert_eq!(sigs[0].1, 79573);

        let sigs =
            fourbyte_event("0xb7009613e63fb13fd59a2fa4c206a992c1f090a44e5d530be255aa17fed0b3dd")
                .await
                .unwrap();
        assert_eq!(sigs[0].0, "canCall(address,address,bytes4)".to_string());
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn abi2solidity() {
        let contract_abi: Abi =
            serde_json::from_slice(&std::fs::read("testdata/interfaceTestABI.json").unwrap())
                .unwrap();
        assert_eq!(
            std::str::from_utf8(&std::fs::read("testdata/interfaceTest.sol").unwrap())
                .unwrap()
                .to_string(),
            abi_to_solidity(&contract_abi, "test").unwrap()
        );
        assert_eq!(
            std::str::from_utf8(&std::fs::read("testdata/interfaceTestNoName.sol").unwrap())
                .unwrap()
                .to_string(),
            abi_to_solidity(&contract_abi, "").unwrap()
        );
    }
}
