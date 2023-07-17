use super::{bail, ensure, fmt_err, Cheatcodes, Result};
use crate::{abi::HEVMCalls, executor::inspector::cheatcodes::util};
use ethers::{
    abi::{self, AbiEncode, JsonAbi, ParamType, Token},
    prelude::artifacts::CompactContractBytecode,
    types::*,
};
use foundry_common::{fmt::*, fs, get_artifact_path};
use foundry_config::fs_permissions::FsAccessKind;
use hex::FromHex;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, env, path::Path, process::Command, str::FromStr};

/// Invokes a `Command` with the given args and returns the abi encoded response
///
/// If the output of the command is valid hex, it returns the hex decoded value
fn ffi(state: &Cheatcodes, args: &[String]) -> Result {
    if args.is_empty() || args[0].is_empty() {
        bail!("Can't execute empty command");
    }
    let name = &args[0];
    let mut cmd = Command::new(name);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }

    debug!(target: "evm::cheatcodes", ?args, "invoking ffi");

    let output = cmd
        .current_dir(&state.config.root)
        .output()
        .map_err(|err| fmt_err!("Failed to execute command: {err}"))?;

    if !output.stderr.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(target: "evm::cheatcodes", ?args, ?stderr, "non-empty stderr");
    }

    let output = String::from_utf8(output.stdout)?;
    let trimmed = output.trim();
    if let Ok(hex) = hex::decode(trimmed.strip_prefix("0x").unwrap_or(trimmed)) {
        Ok(abi::encode(&[Token::Bytes(hex)]).into())
    } else {
        Ok(trimmed.encode().into())
    }
}

/// An enum which unifies the deserialization of Hardhat-style artifacts with Forge-style artifacts
/// to get their bytecode.
#[derive(Deserialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
enum ArtifactBytecode {
    Hardhat(HardhatArtifact),
    Solc(JsonAbi),
    Forge(CompactContractBytecode),
    Huff(HuffArtifact),
}

impl ArtifactBytecode {
    fn into_bytecode(self) -> Option<Bytes> {
        match self {
            ArtifactBytecode::Hardhat(inner) => Some(inner.bytecode),
            ArtifactBytecode::Forge(inner) => {
                inner.bytecode.and_then(|bytecode| bytecode.object.into_bytes())
            }
            ArtifactBytecode::Solc(inner) => inner.bytecode(),
            ArtifactBytecode::Huff(inner) => Some(inner.bytecode),
        }
    }

    fn into_deployed_bytecode(self) -> Option<Bytes> {
        match self {
            ArtifactBytecode::Hardhat(inner) => Some(inner.deployed_bytecode),
            ArtifactBytecode::Forge(inner) => inner.deployed_bytecode.and_then(|bytecode| {
                bytecode.bytecode.and_then(|bytecode| bytecode.object.into_bytes())
            }),
            ArtifactBytecode::Solc(inner) => inner.deployed_bytecode(),
            ArtifactBytecode::Huff(inner) => Some(inner.runtime),
        }
    }
}

/// A thin wrapper around a Hardhat-style artifact that only extracts the bytecode.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HardhatArtifact {
    bytecode: Bytes,
    deployed_bytecode: Bytes,
}

#[derive(Deserialize)]
struct HuffArtifact {
    bytecode: Bytes,
    runtime: Bytes,
}

/// Returns the _deployed_ bytecode (`bytecode`) of the matching artifact
fn get_code(state: &Cheatcodes, path: &str) -> Result {
    let bytecode = read_bytecode(state, path)?;
    if let Some(bin) = bytecode.into_bytecode() {
        Ok(bin.encode().into())
    } else {
        Err(fmt_err!("No bytecode for contract. Is it abstract or unlinked?"))
    }
}

/// Returns the _deployed_ bytecode (`bytecode`) of the matching artifact
fn get_deployed_code(state: &Cheatcodes, path: &str) -> Result {
    let bytecode = read_bytecode(state, path)?;
    if let Some(bin) = bytecode.into_deployed_bytecode() {
        Ok(bin.encode().into())
    } else {
        Err(fmt_err!("No deployed bytecode for contract. Is it abstract or unlinked?"))
    }
}

/// Reads the bytecode object(s) from the matching artifact
fn read_bytecode(state: &Cheatcodes, path: &str) -> Result<ArtifactBytecode> {
    let path = get_artifact_path(&state.config.paths, path);
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
    let data = fs::read_to_string(path)?;
    serde_json::from_str::<ArtifactBytecode>(&data).map_err(Into::into)
}

fn set_env(key: &str, val: &str) -> Result {
    // `std::env::set_var` may panic in the following situations
    // ref: https://doc.rust-lang.org/std/env/fn.set_var.html
    if key.is_empty() {
        Err(fmt_err!("Environment variable key can't be empty"))
    } else if key.contains('=') {
        Err(fmt_err!("Environment variable key can't contain equal sign `=`"))
    } else if key.contains('\0') {
        Err(fmt_err!("Environment variable key can't contain NUL character `\\0`"))
    } else if val.contains('\0') {
        Err(fmt_err!("Environment variable value can't contain NUL character `\\0`"))
    } else {
        env::set_var(key, val);
        Ok(Bytes::new())
    }
}

fn get_env(key: &str, ty: ParamType, delim: Option<&str>, default: Option<String>) -> Result {
    let val = env::var(key).or_else(|e| {
        default.ok_or_else(|| {
            fmt_err!("Failed to get environment variable `{key}` as type `{ty}`: {e}")
        })
    })?;
    if let Some(d) = delim {
        util::parse_array(val.split(d).map(str::trim), &ty)
    } else {
        util::parse(&val, &ty)
    }
}

/// Converts a JSON [`Value`] to a [`Token`].
///
/// The function is designed to run recursively, so that in case of an object
/// it will call itself to convert each of it's value and encode the whole as a
/// Tuple
fn value_to_token(value: &Value) -> Result<Token> {
    match value {
        Value::Null => Ok(Token::FixedBytes(vec![0; 32])),
        Value::Bool(boolean) => Ok(Token::Bool(*boolean)),
        Value::Array(array) => {
            let values = array.iter().map(value_to_token).collect::<Result<Vec<_>>>()?;
            Ok(Token::Array(values))
        }
        value @ Value::Object(_) => {
            // See: [#3647](https://github.com/foundry-rs/foundry/pull/3647)
            let ordered_object: BTreeMap<String, Value> =
                serde_json::from_value(value.clone()).unwrap();
            let values = ordered_object.values().map(value_to_token).collect::<Result<Vec<_>>>()?;
            Ok(Token::Tuple(values))
        }
        Value::Number(number) => {
            if let Some(f) = number.as_f64() {
                // Check if the number has decimal digits because the EVM does not support floating
                // point math
                if f.fract() == 0.0 {
                    // Use the string representation of the `serde_json` Number type instead of
                    // calling f.to_string(), because some numbers are wrongly rounded up after
                    // being convented to f64.
                    // Example: 18446744073709551615 becomes 18446744073709552000 after parsing it
                    // to f64.
                    let s = number.to_string();

                    // Calling Number::to_string with powers of ten formats the number using
                    // scientific notation and causes from_dec_str to fail. Using format! with f64
                    // keeps the full number representation.
                    // Example: 100000000000000000000 becomes 1e20 when Number::to_string is
                    // used.
                    let fallback_s = format!("{f}");

                    if let Ok(n) = U256::from_dec_str(&s) {
                        return Ok(Token::Uint(n))
                    }
                    if let Ok(n) = I256::from_dec_str(&s) {
                        return Ok(Token::Int(n.into_raw()))
                    }
                    if let Ok(n) = U256::from_dec_str(&fallback_s) {
                        return Ok(Token::Uint(n))
                    }
                    if let Ok(n) = I256::from_dec_str(&fallback_s) {
                        return Ok(Token::Int(n.into_raw()))
                    }
                }
            }

            Err(fmt_err!("Unsupported value: {number:?}"))
        }
        Value::String(string) => {
            if let Some(val) = string.strip_prefix("0x") {
                // If it can decoded as an address, it's an address
                if let Ok(addr) = H160::from_str(string) {
                    Ok(Token::Address(addr))
                } else if hex::decode(val).is_ok() {
                    // if length == 32 bytes, then encode as Bytes32, else Bytes
                    Ok(if val.len() == 64 {
                        Token::FixedBytes(Vec::from_hex(val).unwrap())
                    } else {
                        Token::Bytes(Vec::from_hex(val).unwrap())
                    })
                } else {
                    // If incorrect length, pad 0 at the beginning
                    let arr = format!("0{val}");
                    Ok(Token::Bytes(Vec::from_hex(arr).unwrap()))
                }
            } else {
                Ok(Token::String(string.to_owned()))
            }
        }
    }
}

/// Canonicalize a json path key to always start from the root of the document.
/// Read more about json path syntax: https://goessner.net/articles/JsonPath/
fn canonicalize_json_key(key: &str) -> String {
    if !key.starts_with('$') {
        format!("${key}")
    } else {
        key.to_owned()
    }
}

/// Parses a JSON and returns a single value, an array or an entire JSON object encoded as tuple.
/// As the JSON object is parsed serially, with the keys ordered alphabetically, they must be
/// deserialized in the same order. That means that the solidity `struct` should order it's fields
/// alphabetically and not by efficient packing or some other taxonomy.
fn parse_json(json_str: &str, key: &str, coerce: Option<ParamType>) -> Result {
    let json = serde_json::from_str(json_str)?;
    let values = jsonpath_lib::select(&json, &canonicalize_json_key(key))?;

    // values is an array of items. Depending on the JsonPath key, they
    // can be many or a single item. An item can be a single value or
    // an entire JSON object.
    if let Some(coercion_type) = coerce {
        ensure!(
            values.iter().all(|value| !value.is_object()),
            "You can only coerce values or arrays, not JSON objects. The key '{key}' returns an object",
        );

        ensure!(!values.is_empty(), "No matching value or array found for key {key}");

        let to_string = |v: &Value| {
            let mut s = v.to_string();
            s.retain(|c: char| c != '"');
            s
        };
        return if let Some(array) = values[0].as_array() {
            util::parse_array(array.iter().map(to_string), &coercion_type)
        } else {
            util::parse(&to_string(values[0]), &coercion_type)
        }
    } else {
        // If the user did not specify a coercion type, we should ensure it exists as sanity check.
        ensure!(!values.is_empty(), "No matching value or array found for key {key}");
    }

    let res = values
        .iter()
        .map(|inner| {
            value_to_token(inner).map_err(|err| fmt_err!("Failed to parse key \"{key}\": {err}"))
        })
        .collect::<Result<Vec<Token>>>()?;

    // encode the bytes as the 'bytes' solidity type
    let abi_encoded = if res.len() == 1 {
        abi::encode(&[Token::Bytes(abi::encode(&res))])
    } else {
        abi::encode(&[Token::Bytes(abi::encode(&[Token::Array(res)]))])
    };
    Ok(abi_encoded.into())
}

/// Serializes a key:value pair to a specific object. By calling this function multiple times,
/// the user can serialize multiple KV pairs to the same object. The value can be of any type, even
/// a new object in itself. The function will return
/// a stringified version of the object, so that the user can use that as a value to a new
/// invocation of the same function with a new object key. This enables the user to reuse the same
/// function to crate arbitrarily complex object structures (JSON).
fn serialize_json(
    state: &mut Cheatcodes,
    object_key: &str,
    value_key: &str,
    value: &str,
) -> Result {
    let parsed_value =
        serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()));
    let json = if let Some(serialization) = state.serialized_jsons.get_mut(object_key) {
        serialization.insert(value_key.to_string(), parsed_value);
        serialization.clone()
    } else {
        let mut serialization = BTreeMap::new();
        serialization.insert(value_key.to_string(), parsed_value);
        state.serialized_jsons.insert(object_key.to_string(), serialization.clone());
        serialization.clone()
    };
    let stringified = serde_json::to_string(&json)
        .map_err(|err| fmt_err!(format!("Failed to stringify hashmap: {err}")))?;
    Ok(abi::encode(&[Token::String(stringified)]).into())
}

/// Converts an array to it's stringified version, adding the appropriate quotes around it's
/// ellements. This is to signify that the elements of the array are string themselves.
fn array_str_to_str<T: UIfmt>(array: &Vec<T>) -> String {
    format!(
        "[{}]",
        array
            .iter()
            .enumerate()
            .map(|(index, value)| {
                if index == array.len() - 1 {
                    format!("\"{}\"", value.pretty())
                } else {
                    format!("\"{}\",", value.pretty())
                }
            })
            .collect::<String>()
    )
}

/// Converts an array to it's stringified version. It will not add quotes around the values of the
/// array, enabling serde_json to parse the values of the array as types (e.g numbers, booleans,
/// etc.)
fn array_eval_to_str<T: UIfmt>(array: &Vec<T>) -> String {
    format!(
        "[{}]",
        array
            .iter()
            .enumerate()
            .map(|(index, value)| {
                if index == array.len() - 1 {
                    value.pretty()
                } else {
                    format!("{},", value.pretty())
                }
            })
            .collect::<String>()
    )
}

/// Write an object to a new file OR replace the value of an existing JSON file with the supplied
/// object.
fn write_json(
    state: &mut Cheatcodes,
    object: &str,
    path: impl AsRef<Path>,
    json_path_or_none: Option<&str>,
) -> Result {
    let json: Value =
        serde_json::from_str(object).unwrap_or_else(|_| Value::String(object.to_owned()));
    let json_string = serde_json::to_string_pretty(&if let Some(json_path) = json_path_or_none {
        let path = state.config.ensure_path_allowed(&path, FsAccessKind::Read)?;
        let data = serde_json::from_str(&fs::read_to_string(path)?)?;
        jsonpath_lib::replace_with(data, &canonicalize_json_key(json_path), &mut |_| {
            Some(json.clone())
        })?
    } else {
        json
    })?;
    super::fs::write_file(state, path, json_string)?;
    Ok(Bytes::new())
}

#[instrument(level = "error", name = "ext", target = "evm::cheatcodes", skip_all)]
pub fn apply(state: &mut Cheatcodes, call: &HEVMCalls) -> Option<Result> {
    Some(match call {
        HEVMCalls::Ffi(inner) => {
            if state.config.ffi {
                ffi(state, &inner.0)
            } else {
                Err(fmt_err!("FFI disabled: run again with `--ffi` if you want to allow tests to call external scripts."))
            }
        }
        HEVMCalls::GetCode(inner) => get_code(state, &inner.0),
        HEVMCalls::GetDeployedCode(inner) => get_deployed_code(state, &inner.0),
        HEVMCalls::SetEnv(inner) => set_env(&inner.0, &inner.1),
        HEVMCalls::EnvBool0(inner) => get_env(&inner.0, ParamType::Bool, None, None),
        HEVMCalls::EnvUint0(inner) => get_env(&inner.0, ParamType::Uint(256), None, None),
        HEVMCalls::EnvInt0(inner) => get_env(&inner.0, ParamType::Int(256), None, None),
        HEVMCalls::EnvAddress0(inner) => get_env(&inner.0, ParamType::Address, None, None),
        HEVMCalls::EnvBytes320(inner) => get_env(&inner.0, ParamType::FixedBytes(32), None, None),
        HEVMCalls::EnvString0(inner) => get_env(&inner.0, ParamType::String, None, None),
        HEVMCalls::EnvBytes0(inner) => get_env(&inner.0, ParamType::Bytes, None, None),
        HEVMCalls::EnvBool1(inner) => get_env(&inner.0, ParamType::Bool, Some(&inner.1), None),
        HEVMCalls::EnvUint1(inner) => get_env(&inner.0, ParamType::Uint(256), Some(&inner.1), None),
        HEVMCalls::EnvInt1(inner) => get_env(&inner.0, ParamType::Int(256), Some(&inner.1), None),
        HEVMCalls::EnvAddress1(inner) => {
            get_env(&inner.0, ParamType::Address, Some(&inner.1), None)
        }
        HEVMCalls::EnvBytes321(inner) => {
            get_env(&inner.0, ParamType::FixedBytes(32), Some(&inner.1), None)
        }
        HEVMCalls::EnvString1(inner) => get_env(&inner.0, ParamType::String, Some(&inner.1), None),
        HEVMCalls::EnvBytes1(inner) => get_env(&inner.0, ParamType::Bytes, Some(&inner.1), None),
        HEVMCalls::EnvOr0(inner) => {
            get_env(&inner.0, ParamType::Bool, None, Some(inner.1.to_string()))
        }
        HEVMCalls::EnvOr1(inner) => {
            get_env(&inner.0, ParamType::Uint(256), None, Some(inner.1.to_string()))
        }
        HEVMCalls::EnvOr2(inner) => {
            get_env(&inner.0, ParamType::Int(256), None, Some(inner.1.to_string()))
        }
        HEVMCalls::EnvOr3(inner) => {
            get_env(&inner.0, ParamType::Address, None, Some(hex::encode(inner.1)))
        }
        HEVMCalls::EnvOr4(inner) => {
            get_env(&inner.0, ParamType::FixedBytes(32), None, Some(hex::encode(inner.1)))
        }
        HEVMCalls::EnvOr5(inner) => {
            get_env(&inner.0, ParamType::String, None, Some(inner.1.to_string()))
        }
        HEVMCalls::EnvOr6(inner) => {
            get_env(&inner.0, ParamType::Bytes, None, Some(hex::encode(&inner.1)))
        }
        HEVMCalls::EnvOr7(inner) => get_env(
            &inner.0,
            ParamType::Bool,
            Some(&inner.1),
            Some(inner.2.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&inner.1)),
        ),
        HEVMCalls::EnvOr8(inner) => get_env(
            &inner.0,
            ParamType::Uint(256),
            Some(&inner.1),
            Some(inner.2.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&inner.1)),
        ),
        HEVMCalls::EnvOr9(inner) => get_env(
            &inner.0,
            ParamType::Int(256),
            Some(&inner.1),
            Some(inner.2.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&inner.1)),
        ),
        HEVMCalls::EnvOr10(inner) => get_env(
            &inner.0,
            ParamType::Address,
            Some(&inner.1),
            Some(inner.2.iter().map(hex::encode).collect::<Vec<_>>().join(&inner.1)),
        ),
        HEVMCalls::EnvOr11(inner) => get_env(
            &inner.0,
            ParamType::FixedBytes(32),
            Some(&inner.1),
            Some(inner.2.iter().map(hex::encode).collect::<Vec<_>>().join(&inner.1)),
        ),
        HEVMCalls::EnvOr12(inner) => {
            get_env(&inner.0, ParamType::String, Some(&inner.1), Some(inner.2.join(&inner.1)))
        }
        HEVMCalls::EnvOr13(inner) => get_env(
            &inner.0,
            ParamType::Bytes,
            Some(&inner.1),
            Some(inner.2.iter().map(hex::encode).collect::<Vec<_>>().join(&inner.1)),
        ),

        // If no key argument is passed, return the whole JSON object.
        // "$" is the JSONPath key for the root of the object
        HEVMCalls::ParseJson0(inner) => parse_json(&inner.0, "$", None),
        HEVMCalls::ParseJson1(inner) => parse_json(&inner.0, &inner.1, None),
        HEVMCalls::ParseJsonBool(inner) => parse_json(&inner.0, &inner.1, Some(ParamType::Bool)),
        HEVMCalls::ParseJsonBoolArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::Bool))
        }
        HEVMCalls::ParseJsonUint(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::Uint(256)))
        }
        HEVMCalls::ParseJsonUintArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::Uint(256)))
        }
        HEVMCalls::ParseJsonInt(inner) => parse_json(&inner.0, &inner.1, Some(ParamType::Int(256))),
        HEVMCalls::ParseJsonIntArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::Int(256)))
        }
        HEVMCalls::ParseJsonString(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::String))
        }
        HEVMCalls::ParseJsonStringArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::String))
        }
        HEVMCalls::ParseJsonAddress(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::Address))
        }
        HEVMCalls::ParseJsonAddressArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::Address))
        }
        HEVMCalls::ParseJsonBytes(inner) => parse_json(&inner.0, &inner.1, Some(ParamType::Bytes)),
        HEVMCalls::ParseJsonBytesArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::Bytes))
        }
        HEVMCalls::ParseJsonBytes32(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::FixedBytes(32)))
        }
        HEVMCalls::ParseJsonBytes32Array(inner) => {
            parse_json(&inner.0, &inner.1, Some(ParamType::FixedBytes(32)))
        }
        HEVMCalls::SerializeBool0(inner) => {
            serialize_json(state, &inner.0, &inner.1, &inner.2.pretty())
        }
        HEVMCalls::SerializeBool1(inner) => {
            serialize_json(state, &inner.0, &inner.1, &array_eval_to_str(&inner.2))
        }
        HEVMCalls::SerializeUint0(inner) => {
            serialize_json(state, &inner.0, &inner.1, &inner.2.pretty())
        }
        HEVMCalls::SerializeUint1(inner) => {
            serialize_json(state, &inner.0, &inner.1, &array_eval_to_str(&inner.2))
        }
        HEVMCalls::SerializeInt0(inner) => {
            serialize_json(state, &inner.0, &inner.1, &inner.2.pretty())
        }
        HEVMCalls::SerializeInt1(inner) => {
            serialize_json(state, &inner.0, &inner.1, &array_eval_to_str(&inner.2))
        }
        HEVMCalls::SerializeAddress0(inner) => {
            serialize_json(state, &inner.0, &inner.1, &inner.2.pretty())
        }
        HEVMCalls::SerializeAddress1(inner) => {
            serialize_json(state, &inner.0, &inner.1, &array_str_to_str(&inner.2))
        }
        HEVMCalls::SerializeBytes320(inner) => {
            serialize_json(state, &inner.0, &inner.1, &inner.2.pretty())
        }
        HEVMCalls::SerializeBytes321(inner) => {
            serialize_json(state, &inner.0, &inner.1, &array_str_to_str(&inner.2))
        }
        HEVMCalls::SerializeString0(inner) => {
            serialize_json(state, &inner.0, &inner.1, &inner.2.pretty())
        }
        HEVMCalls::SerializeString1(inner) => {
            serialize_json(state, &inner.0, &inner.1, &array_str_to_str(&inner.2))
        }
        HEVMCalls::SerializeBytes0(inner) => {
            serialize_json(state, &inner.0, &inner.1, &inner.2.pretty())
        }
        HEVMCalls::SerializeBytes1(inner) => {
            serialize_json(state, &inner.0, &inner.1, &array_str_to_str(&inner.2))
        }
        HEVMCalls::WriteJson0(inner) => write_json(state, &inner.0, &inner.1, None),
        HEVMCalls::WriteJson1(inner) => write_json(state, &inner.0, &inner.1, Some(&inner.2)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::inspector::CheatsConfig;
    use ethers::core::abi::AbiDecode;
    use std::{path::PathBuf, sync::Arc};

    fn cheats() -> Cheatcodes {
        let config =
            CheatsConfig { root: PathBuf::from(&env!("CARGO_MANIFEST_DIR")), ..Default::default() };
        Cheatcodes { config: Arc::new(config), ..Default::default() }
    }

    #[test]
    fn test_ffi_hex() {
        let msg = "gm";
        let cheats = cheats();
        let args = ["echo".to_string(), hex::encode(msg)];
        let output = ffi(&cheats, &args).unwrap();

        let output = String::decode(&output).unwrap();
        assert_eq!(output, msg);
    }

    #[test]
    fn test_ffi_string() {
        let msg = "gm";
        let cheats = cheats();

        let args = ["echo".to_string(), msg.to_string()];
        let output = ffi(&cheats, &args).unwrap();

        let output = String::decode(&output).unwrap();
        assert_eq!(output, msg);
    }

    #[test]
    fn test_artifact_parsing() {
        let s = include_str!("../../../../test-data/solc-obj.json");
        let artifact: ArtifactBytecode = serde_json::from_str(s).unwrap();
        assert!(artifact.into_bytecode().is_some());

        let artifact: ArtifactBytecode = serde_json::from_str(s).unwrap();
        assert!(artifact.into_deployed_bytecode().is_some());
    }
}
