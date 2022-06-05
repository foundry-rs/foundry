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
use ethers_providers::{Middleware, Provider, ProviderError};
use ethers_solc::{
    artifacts::{BytecodeObject, CompactBytecode, CompactContractBytecode, Libraries},
    ArtifactId,
};
use eyre::{Result, WrapErr};
use futures::future::BoxFuture;
use std::{
    collections::{BTreeMap, HashSet},
    env::VarError,
    fmt::Write,
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

pub mod rpc;
pub mod selectors;

/// Very simple fuzzy matching of contract bytecode.
///
/// Will fail for small contracts that are essentially all immutable variables.
pub fn diff_score(a: &[u8], b: &[u8]) -> f64 {
    let cutoff_len = usize::min(a.len(), b.len());
    if cutoff_len == 0 {
        return 1.0
    }

    let a = &a[..cutoff_len];
    let b = &b[..cutoff_len];
    let mut diff_chars = 0;
    for i in 0..cutoff_len {
        if a[i] != b[i] {
            diff_chars += 1;
        }
    }
    diff_chars as f64 / cutoff_len as f64
}

#[derive(Debug)]
pub struct PostLinkInput<'a, T, U> {
    pub contract: CompactContractBytecode,
    pub known_contracts: &'a mut BTreeMap<ArtifactId, T>,
    pub id: ArtifactId,
    pub extra: &'a mut U,
    pub dependencies: Vec<(String, ethers_core::types::Bytes)>,
}

#[allow(clippy::too_many_arguments)]
pub fn link_with_nonce_or_address<T, U>(
    contracts: BTreeMap<ArtifactId, CompactContractBytecode>,
    known_contracts: &mut BTreeMap<ArtifactId, T>,
    deployed_library_addresses: Libraries,
    sender: Address,
    nonce: U256,
    extra: &mut U,
    link_key_construction: impl Fn(String, String) -> (String, String, String),
    post_link: impl Fn(PostLinkInput<T, U>) -> eyre::Result<()>,
) -> eyre::Result<()> {
    // create a mapping of fname => Vec<(fname, file, key)>,
    let link_tree: BTreeMap<String, Vec<(String, String, String)>> = contracts
        .iter()
        .map(|(id, contract)| {
            (
                id.slug(),
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

    let contracts_by_slug = contracts
        .iter()
        .map(|(i, c)| (i.slug(), c.clone()))
        .collect::<BTreeMap<String, CompactContractBytecode>>();

    for (id, contract) in contracts.into_iter() {
        let (abi, maybe_deployment_bytes, maybe_runtime) = (
            contract.abi.as_ref(),
            contract.bytecode.as_ref(),
            contract.deployed_bytecode.as_ref(),
        );

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
                        id.slug(),
                        (&mut target_bytecode, &mut target_bytecode_runtime),
                        &contracts_by_slug,
                        &link_tree,
                        &mut dependencies,
                        &deployed_library_addresses,
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

            let post_link_input =
                PostLinkInput { contract: tc, known_contracts, id, extra, dependencies };

            post_link(post_link_input)?;
        }
    }
    Ok(())
}

/// Recursively links bytecode given a target contract artifact name, the bytecode(s) to be linked,
/// a mapping of contract artifact name to bytecode, a dependency mapping, a mutable list that
/// will be filled with the predeploy libraries, initial nonce, and the sender.
#[allow(clippy::too_many_arguments)]
pub fn recurse_link<'a>(
    // target name
    target: String,
    // to-be-modified/linked bytecode
    target_bytecode: (&'a mut CompactBytecode, &'a mut CompactBytecode),
    // Contracts
    contracts: &'a BTreeMap<String, CompactContractBytecode>,
    // fname => Vec<(fname, file, key)>
    dependency_tree: &'a BTreeMap<String, Vec<(String, String, String)>>,
    // library deployment vector (file:contract:address, bytecode)
    deployment: &'a mut Vec<(String, ethers_core::types::Bytes)>,
    // deployed library addresses fname => adddress
    deployed_library_addresses: &'a Libraries,
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
            if let Some(deps) = dependency_tree.get(next_target) {
                if !deps.is_empty() {
                    // actually link the nested dependencies to this dependency
                    recurse_link(
                        next_target.to_string(),
                        (&mut next_target_bytecode, &mut next_target_runtime_bytecode),
                        contracts,
                        dependency_tree,
                        deployment,
                        deployed_library_addresses,
                        init_nonce,
                        sender,
                    );
                }
            }

            let mut deployed_address = None;

            if let Some(library_file) = deployed_library_addresses
                .libs
                .get(&PathBuf::from_str(file).expect("Invalid library path."))
            {
                if let Some(address) = library_file.get(key) {
                    deployed_address =
                        Some(Address::from_str(address).expect("Invalid library address passed."));
                }
            }

            let address = deployed_address.unwrap_or_else(|| {
                ethers_core::utils::get_contract_address(sender, init_nonce + deployment.len())
            });

            // link the dependency to the target
            target_bytecode.0.link(file.clone(), key.clone(), address);
            target_bytecode.1.link(file.clone(), key.clone(), address);

            if deployed_address.is_none() {
                let library = format!("{}:{}:0x{}", file, key, hex::encode(address));

                // push the dependency into the library deployment vector
                deployment.push((
                    library,
                    next_target_bytecode.object.into_bytes().expect("Bytecode should be linked"),
                ));
            }
        });
    }
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
        AbiParser::default()
            .parse_function(self)
            .unwrap_or_else(|_| panic!("could not convert {self} to function"))
    }
}

/// Flattens a group of contracts into maps of all events and functions
pub fn flatten_known_contracts(
    contracts: &BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
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
            // keccak(expectRevert(bytes4))
            [195, 30, 176, 224] => {
                let err_data = &error[4..];
                if err_data.len() == 32 {
                    let actual_err = &err_data[..4];
                    if let Ok(decoded) = decode_revert(actual_err, maybe_abi) {
                        // it's a known selector
                        return Ok(decoded)
                    }
                }
                Err(eyre::Error::msg("Unknown error selector"))
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
                writeln!(&mut s, "{: <20} {}\n", k, v).expect("could not write k/v to table");
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
    etherscan_api_key: &str,
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
#[allow(clippy::no_effect)]
pub fn parse_tokens<'a, I: IntoIterator<Item = (&'a ParamType, &'a str)>>(
    params: I,
    lenient: bool,
) -> Result<Vec<Token>> {
    params
        .into_iter()
        .map(|(param, value)| {
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
            token
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
        Token::Int(num) => format!("{}", I256::from_raw(*num)),
        Token::Uint(num) => num.to_string(),
        Token::Bool(b) => format!("{b}"),
        Token::String(s) => format!("{:?}", s),
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
    let kind = if is_memory { format!("{kind} memory") } else { kind };

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

            let v2_struct = format!("struct {name} {{ {args} }}");
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
/// * Kudos to [maxme/abi2solidity](https://github.com/maxme/abi2solidity) for the algorithm
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
            format!("{event_final};")
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
                func = format!("{func} {mutability}");
            }
            func = format!("{func} external");
            if !outputs.is_empty() {
                func = format!("{func} returns ({outputs})");
            }
            format!("{func};")
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

/// A type that keeps track of attempts
#[derive(Debug, Clone)]
pub struct Retry {
    retries: u32,
    delay: Option<u32>,
}

/// Sample retry logic implementation
impl Retry {
    pub fn new(retries: u32, delay: Option<u32>) -> Self {
        Self { retries, delay }
    }

    fn handle_err(&mut self, err: eyre::Report) {
        self.retries -= 1;
        tracing::warn!(
            "erroneous attempt ({} tries remaining): {}",
            self.retries,
            err.root_cause()
        );
        if let Some(delay) = self.delay {
            std::thread::sleep(Duration::from_secs(delay.into()));
        }
    }

    pub fn run<T, F>(mut self, mut callback: F) -> eyre::Result<T>
    where
        F: FnMut() -> eyre::Result<T>,
    {
        loop {
            match callback() {
                Err(e) if self.retries > 0 => self.handle_err(e),
                res => return res,
            }
        }
    }

    pub async fn run_async<'a, T, F>(mut self, mut callback: F) -> eyre::Result<T>
    where
        F: FnMut() -> BoxFuture<'a, eyre::Result<T>>,
    {
        loop {
            match callback().await {
                Err(e) if self.retries > 0 => self.handle_err(e),
                res => return res,
            };
        }
    }
}

/// Enables tracing
#[cfg(any(feature = "test"))]
pub fn init_tracing_subscriber() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();
}

pub async fn next_nonce(
    caller: Address,
    provider_url: &str,
    block: Option<BlockId>,
) -> Result<U256, ProviderError> {
    let provider = Provider::try_from(provider_url).expect("Bad fork_url provider");
    provider.get_transaction_count(caller, block).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::{
        abi::Abi,
        solc::{artifacts::CompactContractBytecode, Project, ProjectPathsConfig},
        types::{Address, Bytes},
    };

    #[test]
    fn parse_hex_uint_tokens() {
        let param = ParamType::Uint(256);

        let tokens = parse_tokens(std::iter::once((&param, "100")), true).unwrap();
        assert_eq!(tokens, vec![Token::Uint(100u64.into())]);

        let val: U256 = 100u64.into();
        let hex_val = format!("0x{:x}", val);
        let tokens = parse_tokens(std::iter::once((&param, hex_val.as_str())), true).unwrap();
        assert_eq!(tokens, vec![Token::Uint(100u64.into())]);
    }

    #[test]
    fn test_linking() {
        let mut contract_names = [
            "DSTest.json:DSTest",
            "Lib.json:Lib",
            "LibraryConsumer.json:LibraryConsumer",
            "LibraryLinkingTest.json:LibraryLinkingTest",
            "NestedLib.json:NestedLib",
        ];
        contract_names.sort_unstable();

        let paths = ProjectPathsConfig::builder()
            .root("../testdata")
            .sources("../testdata/core")
            .build()
            .unwrap();

        let project = Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();

        let output = project.compile().unwrap();
        let contracts = output
            .into_artifacts()
            .filter(|(i, _)| contract_names.contains(&i.slug().as_str()))
            .map(|(id, c)| (id, c.into_contract_bytecode()))
            .collect::<BTreeMap<ArtifactId, CompactContractBytecode>>();

        let mut known_contracts: BTreeMap<ArtifactId, (Abi, Vec<u8>)> = Default::default();
        let mut deployable_contracts: BTreeMap<String, (Abi, Bytes, Vec<Bytes>)> =
            Default::default();

        let mut res = contracts.keys().map(|i| i.slug()).collect::<Vec<String>>();
        res.sort_unstable();
        assert_eq!(&res[..], &contract_names[..]);

        let lib_linked = hex::encode(
            &contracts
                .iter()
                .find(|(i, _)| i.slug() == "Lib.json:Lib")
                .unwrap()
                .1
                .bytecode
                .clone()
                .expect("library had no bytecode")
                .object
                .into_bytes()
                .expect("could not get bytecode as bytes"),
        );
        let nested_lib_unlinked = &contracts
            .iter()
            .find(|(i, _)| i.slug() == "NestedLib.json:NestedLib")
            .unwrap()
            .1
            .bytecode
            .as_ref()
            .expect("nested library had no bytecode")
            .object
            .as_str()
            .expect("could not get bytecode as str")
            .to_string();

        link_with_nonce_or_address(
            contracts,
            &mut known_contracts,
            Default::default(),
            Address::default(),
            U256::one(),
            &mut deployable_contracts,
            |file, key| (format!("{key}.json:{key}"), file, key),
            |post_link_input| {
                match post_link_input.id.slug().as_str() {
                    "DSTest.json:DSTest" => {
                        assert_eq!(post_link_input.dependencies.len(), 0);
                    }
                    "LibraryLinkingTest.json:LibraryLinkingTest" => {
                        assert_eq!(post_link_input.dependencies.len(), 3);
                        assert_eq!(hex::encode(&post_link_input.dependencies[0].1), lib_linked);
                        assert_eq!(hex::encode(&post_link_input.dependencies[1].1), lib_linked);
                        assert_ne!(
                            hex::encode(&post_link_input.dependencies[2].1),
                            *nested_lib_unlinked
                        );
                    }
                    "Lib.json:Lib" => {
                        assert_eq!(post_link_input.dependencies.len(), 0);
                    }
                    "NestedLib.json:NestedLib" => {
                        assert_eq!(post_link_input.dependencies.len(), 1);
                        assert_eq!(hex::encode(&post_link_input.dependencies[0].1), lib_linked);
                    }
                    "LibraryConsumer.json:LibraryConsumer" => {
                        assert_eq!(post_link_input.dependencies.len(), 3);
                        assert_eq!(hex::encode(&post_link_input.dependencies[0].1), lib_linked);
                        assert_eq!(hex::encode(&post_link_input.dependencies[1].1), lib_linked);
                        assert_ne!(
                            hex::encode(&post_link_input.dependencies[2].1),
                            *nested_lib_unlinked
                        );
                    }
                    s => panic!("unexpected slug {s}"),
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

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn abi2solidity() {
        let contract_abi: Abi = serde_json::from_slice(
            &std::fs::read("../testdata/fixtures/SolidityGeneration/InterfaceABI.json").unwrap(),
        )
        .unwrap();
        assert_eq!(
            std::str::from_utf8(
                &std::fs::read(
                    "../testdata/fixtures/SolidityGeneration/GeneratedNamedInterface.sol"
                )
                .unwrap()
            )
            .unwrap()
            .to_string(),
            abi_to_solidity(&contract_abi, "test").unwrap()
        );
        assert_eq!(
            std::str::from_utf8(
                &std::fs::read(
                    "../testdata/fixtures/SolidityGeneration/GeneratedUnnamedInterface.sol"
                )
                .unwrap()
            )
            .unwrap()
            .to_string(),
            abi_to_solidity(&contract_abi, "").unwrap()
        );
    }
}
