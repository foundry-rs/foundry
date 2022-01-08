#![doc = include_str!("../README.md")]
use ethers_addressbook::contract;
use ethers_core::{
    abi::{
        self, parse_abi,
        token::{LenientTokenizer, StrictTokenizer, Tokenizer},
        AbiParser, Function, ParamType, Token,
    },
    types::*,
};
use ethers_etherscan::Client;
use eyre::{Result, WrapErr};
use serde::Deserialize;

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
            calldata_cost += 16
        } else {
            calldata_cost += 8;
        }
    }
    gas - calldata_cost - BASE_TX_COST
}

/// Given an ABI encoded error string with the function signature `Error(string)`, it decodes
/// it and returns the revert error message.
pub fn decode_revert(error: &[u8]) -> Result<String> {
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
                        if let Ok(decoded) = decode_revert(actual_err) {
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

pub async fn get_func_etherscan(
    function_name: &str,
    contract: Address,
    args: Vec<String>,
    chain: Chain,
    etherscan_api_key: String,
) -> Result<Function> {
    let client = Client::new(chain, etherscan_api_key)?;
    let abi = client.contract_abi(contract).await?;
    let funcs = abi.functions.get(function_name).unwrap();

    for func in funcs {
        let res = encode_args(func, &args);
        if res.is_ok() {
            return Ok(func.clone())
        }
    }

    Err(eyre::eyre!("Function not found"))
}

/// Parses string input as Token against the expected ParamType
pub fn parse_tokens<'a, I: IntoIterator<Item = (&'a ParamType, &'a str)>>(
    params: I,
    lenient: bool,
) -> eyre::Result<Vec<Token>> {
    params
        .into_iter()
        .map(|(param, value)| {
            let value = match param {
                // allow addresses and bytes to be passed with "0x"
                ParamType::Address => value.strip_prefix("0x").unwrap_or(value),
                ParamType::Bytes => value.strip_prefix("0x").unwrap_or(value),
                ParamType::FixedBytes(_size) => value.strip_prefix("0x").unwrap_or(value),
                _ => value,
            };
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
    let api_response = res.json::<ApiResponse>().await?;

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

#[cfg(test)]
mod tests {
    use super::*;

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

        // DAI:rinkeby exists in ethers-adddressbook (0x8ad3aa5d5ff084307d28c8f514d7a193b2bfe725)
        assert_eq!(
            resolve_addr(NameOrAddress::Name("dai".to_string()), Chain::Rinkeby).ok(),
            Some(NameOrAddress::Address(
                Address::from_str("0x8ad3aa5d5ff084307d28c8f514d7a193b2bfe725").unwrap()
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
}
