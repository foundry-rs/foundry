use super::{bail, ensure, fmt_err, util::MAGIC_SKIP_BYTES, Cheatcodes, Error, Result};
use crate::{abi::HEVMCalls, executor::inspector::cheatcodes::parse};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, Bytes, B256, I256, U256};
use ethers::{abi::JsonAbi, prelude::artifacts::CompactContractBytecode};
use foundry_common::{fmt::*, fs, get_artifact_path};
use foundry_config::fs_permissions::FsAccessKind;
use foundry_utils::types::ToAlloy;
use revm::{Database, EVMData};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    env,
    path::Path,
    process::Command,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

/// Invokes a `Command` with the given args and returns the exit code, stdout, and stderr.
///
/// If stdout or stderr are valid hex, it returns the hex decoded value.
fn try_ffi(state: &Cheatcodes, args: &[String]) -> Result {
    if args.is_empty() || args[0].is_empty() {
        bail!("Can't execute empty command");
    }
    let name = &args[0];
    let mut cmd = Command::new(name);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }

    trace!(?args, "invoking try_ffi");

    let output = cmd
        .current_dir(&state.config.root)
        .output()
        .map_err(|err| fmt_err!("Failed to execute command: {err}"))?;

    let exit_code = output.status.code().unwrap_or(1);

    let trimmed_stdout = String::from_utf8(output.stdout)?;
    let trimmed_stdout = trimmed_stdout.trim();

    // The stdout might be encoded on valid hex, or it might just be a string,
    // so we need to determine which it is to avoid improperly encoding later.
    let encoded_stdout: DynSolValue = if let Ok(hex) = hex::decode(trimmed_stdout) {
        DynSolValue::Bytes(hex)
    } else {
        DynSolValue::Bytes(trimmed_stdout.into())
    };
    let exit_code = I256::from_dec_str(&exit_code.to_string())
        .map_err(|err| fmt_err!("Could not convert exit code: {err}"))?;
    let res = DynSolValue::Tuple(vec![
        DynSolValue::Int(exit_code, 256),
        encoded_stdout,
        // We can grab the stderr output as-is.
        DynSolValue::Bytes(output.stderr),
    ]);

    Ok(res.abi_encode().into())
}

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
    if let Ok(hex) = hex::decode(trimmed) {
        Ok(DynSolValue::Bytes(hex).abi_encode().into())
    } else {
        Ok(DynSolValue::String(trimmed.to_owned()).abi_encode().into())
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
                inner.bytecode.and_then(|bytecode| bytecode.object.into_bytes()).map(|b| b.0.into())
            }
            ArtifactBytecode::Solc(inner) => inner.bytecode().map(|b| b.0.into()),
            ArtifactBytecode::Huff(inner) => Some(inner.bytecode),
        }
    }

    fn into_deployed_bytecode(self) -> Option<Bytes> {
        match self {
            ArtifactBytecode::Hardhat(inner) => Some(inner.deployed_bytecode),
            ArtifactBytecode::Forge(inner) => inner.deployed_bytecode.and_then(|bytecode| {
                bytecode
                    .bytecode
                    .and_then(|bytecode| bytecode.object.into_bytes())
                    .map(|b| b.0.into())
            }),
            ArtifactBytecode::Solc(inner) => inner.deployed_bytecode().map(|b| b.0.into()),
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
        Ok(DynSolValue::Bytes(bin.to_vec()).abi_encode().into())
    } else {
        Err(fmt_err!("No bytecode for contract. Is it abstract or unlinked?"))
    }
}

/// Returns the _deployed_ bytecode (`bytecode`) of the matching artifact
fn get_deployed_code(state: &Cheatcodes, path: &str) -> Result {
    let bytecode = read_bytecode(state, path)?;
    if let Some(bin) = bytecode.into_deployed_bytecode() {
        Ok(DynSolValue::Bytes(bin.to_vec()).abi_encode().into())
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

fn get_env(key: &str, ty: DynSolType, delim: Option<&str>, default: Option<String>) -> Result {
    let val = env::var(key).or_else(|e| {
        default.ok_or_else(|| {
            fmt_err!("Failed to get environment variable `{key}` as type `{ty}`: {e}")
        })
    })?;
    if let Some(d) = delim {
        parse::parse_array(val.split(d).map(str::trim), &ty)
    } else {
        parse::parse(&val, &ty)
    }
}

/// Converts a JSON [`Value`] to a [`DynSolValue`].
///
/// The function is designed to run recursively, so that in case of an object
/// it will call itself to convert each of its values and encode the whole as a
/// Tuple
pub fn value_to_token(value: &Value) -> Result<DynSolValue> {
    match value {
        Value::Null => Ok(DynSolValue::FixedBytes(B256::ZERO, 32)),
        Value::Bool(boolean) => Ok(DynSolValue::Bool(*boolean)),
        Value::Array(array) => {
            let values = array.iter().map(value_to_token).collect::<Result<Vec<_>>>()?;
            Ok(DynSolValue::Array(values))
        }
        value @ Value::Object(_) => {
            // See: [#3647](https://github.com/foundry-rs/foundry/pull/3647)
            let ordered_object: BTreeMap<String, Value> =
                serde_json::from_value(value.clone()).unwrap();
            let values = ordered_object.values().map(value_to_token).collect::<Result<Vec<_>>>()?;
            Ok(DynSolValue::Tuple(values))
        }
        Value::Number(number) => {
            if let Some(f) = number.as_f64() {
                // Check if the number has decimal digits because the EVM does not support floating
                // point math
                if f.fract() == 0.0 {
                    // Use the string representation of the `serde_json` Number type instead of
                    // calling f.to_string(), because some numbers are wrongly rounded up after
                    // being converted to f64.
                    // Example: 18446744073709551615 becomes 18446744073709552000 after parsing it
                    // to f64.
                    let s = number.to_string();
                    // Coerced to scientific notation, so short-ciruit to using fallback.
                    // This will not have a problem with hex numbers, as for parsing these
                    // We'd need to prefix this with 0x.
                    // See also <https://docs.soliditylang.org/en/latest/types.html#rational-and-integer-literals>
                    if s.contains('e') {
                        // Calling Number::to_string with powers of ten formats the number using
                        // scientific notation and causes from_dec_str to fail. Using format! with
                        // f64 keeps the full number representation.
                        // Example: 100000000000000000000 becomes 1e20 when Number::to_string is
                        // used.
                        let fallback_s = format!("{f}");
                        if let Ok(n) = U256::from_str(&fallback_s) {
                            return Ok(DynSolValue::Uint(n, 256))
                        }
                        if let Ok(n) = I256::from_dec_str(&fallback_s) {
                            return Ok(DynSolValue::Int(n, 256))
                        }
                    }

                    if let Ok(n) = U256::from_str(&s) {
                        return Ok(DynSolValue::Uint(n, 256))
                    }
                    if let Ok(n) = I256::from_str(&s) {
                        return Ok(DynSolValue::Int(n, 256))
                    }
                }
            }

            Err(fmt_err!("Unsupported value: {number:?}"))
        }
        Value::String(string) => {
            if let Some(mut val) = string.strip_prefix("0x") {
                let s;
                if val.len() % 2 != 0 {
                    s = format!("0{}", val);
                    val = &s[..];
                }
                let bytes = hex::decode(val)?;
                Ok(match bytes.len() {
                    20 => DynSolValue::Address(Address::from_slice(&bytes)),
                    32 => DynSolValue::FixedBytes(B256::from_slice(&bytes), 32),
                    _ => DynSolValue::Bytes(bytes),
                })
            } else {
                Ok(DynSolValue::String(string.to_owned()))
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

/// Encodes a vector of [`DynSolValue`] into a vector of bytes.
fn encode_abi_values(values: Vec<DynSolValue>) -> Vec<u8> {
    if values.is_empty() {
        DynSolValue::Bytes(Vec::new()).abi_encode()
    } else if values.len() == 1 {
        DynSolValue::Bytes(values[0].abi_encode()).abi_encode()
    } else {
        DynSolValue::Bytes(DynSolValue::Array(values).abi_encode()).abi_encode()
    }
}

/// Parses a vector of [`Value`]s into a vector of [`DynSolValue`]s.
fn parse_json_values(values: Vec<&Value>, key: &str) -> Result<Vec<DynSolValue>> {
    trace!(?values, %key, "parseing json values");
    values
        .iter()
        .map(|inner| {
            value_to_token(inner).map_err(|err| fmt_err!("Failed to parse key \"{key}\": {err}"))
        })
        .collect::<Result<Vec<DynSolValue>>>()
}

/// Parses a JSON and returns a single value, an array or an entire JSON object encoded as tuple.
/// As the JSON object is parsed serially, with the keys ordered alphabetically according to the
/// Rust BTreeMap crate serialization, they must be deserialized in the same order. That means that
/// the solidity `struct` should order its fields not by efficient packing or some other taxonomy
/// but instead alphabetically, with attention to upper/lower casing since uppercase precedes
/// lowercase in BTreeMap lexicographical ordering.
fn parse_json(json_str: &str, key: &str, coerce: Option<DynSolType>) -> Result {
    trace!(%json_str, %key, ?coerce, "parsing json");
    let json =
        serde_json::from_str(json_str).map_err(|err| fmt_err!("Failed to parse JSON: {err}"))?;
    match key {
        // Handle the special case of the root key. We want to return the entire JSON object
        // in this case.
        "." => {
            let values = jsonpath_lib::select(&json, "$")?;
            let res = parse_json_values(values, key)?;

            // encode the bytes as the 'bytes' solidity type
            let abi_encoded = encode_abi_values(res);
            Ok(abi_encoded.into())
        }
        _ => {
            let values = jsonpath_lib::select(&json, &canonicalize_json_key(key))?;
            trace!(?values, "selected json values");

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
                trace!(target : "forge::evm", ?values, "parsing values");
                return if let Some(array) = values[0].as_array() {
                    parse::parse_array(array.iter().map(to_string), &coercion_type)
                } else {
                    parse::parse(&to_string(values[0]), &coercion_type)
                }
            }

            let res = parse_json_values(values, key)?;
            // encode the bytes as the 'bytes' solidity type
            let abi_encoded = encode_abi_values(res);
            Ok(abi_encoded.into())
        }
    }
}

// returns JSON keys of given object as a string array
fn parse_json_keys(json_str: &str, key: &str) -> Result {
    let json = serde_json::from_str(json_str)?;
    let values = jsonpath_lib::select(&json, &canonicalize_json_key(key))?;

    // We need to check that values contains just one JSON-object and not an array of objects
    ensure!(
        values.len() == 1,
        "You can only get keys for a single JSON-object. The key '{key}' returns a value or an array of JSON-objects",
    );

    let value = values[0];

    ensure!(
        value.is_object(),
        "You can only get keys for JSON-object. The key '{key}' does not return an object",
    );

    let res = value
        .as_object()
        .ok_or(eyre::eyre!("Unexpected error while extracting JSON-object"))?
        .keys()
        .map(|key| DynSolValue::String(key.to_owned()))
        .collect::<Vec<DynSolValue>>();

    // encode the bytes as the 'bytes' solidity type
    let abi_encoded = DynSolValue::Array(res).abi_encode();
    Ok(abi_encoded.into())
}

/// Serializes a key:value pair to a specific object. If the key is None, the value is expected to
/// be an object, which will be set as the root object for the provided object key, overriding
/// the whole root object if the object key already exists. By calling this function multiple times,
/// the user can serialize multiple KV pairs to the same object. The value can be of any type, even
/// a new object in itself. The function will return a stringified version of the object, so that
/// the user can use that as a value to a new invocation of the same function with a new object key.
/// This enables the user to reuse the same function to crate arbitrarily complex object structures
/// (JSON). Note that the `BTreeMap` is used to serialize in lexicographical order, meaning
/// uppercase precedes lowercase. More: <https://doc.rust-lang.org/std/collections/struct.BTreeMap.html>
fn serialize_json(
    state: &mut Cheatcodes,
    object_key: &str,
    value_key: Option<&str>,
    value: &str,
) -> Result {
    let json = if let Some(key) = value_key {
        let parsed_value =
            serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()));
        if let Some(serialization) = state.serialized_jsons.get_mut(object_key) {
            serialization.insert(key.to_string(), parsed_value);
            serialization.clone()
        } else {
            let mut serialization = BTreeMap::new();
            serialization.insert(key.to_string(), parsed_value);
            state.serialized_jsons.insert(object_key.to_string(), serialization.clone());
            serialization.clone()
        }
    } else {
        // value must be a JSON object
        let parsed_value: BTreeMap<String, Value> = serde_json::from_str(value)
            .map_err(|err| fmt_err!("Failed to parse JSON object: {err}"))?;
        let serialization = parsed_value;
        state.serialized_jsons.insert(object_key.to_string(), serialization.clone());
        serialization.clone()
    };

    let stringified = serde_json::to_string(&json)
        .map_err(|err| fmt_err!("Failed to stringify hashmap: {err}"))?;
    Ok(DynSolValue::String(stringified).abi_encode().into())
}

/// Converts an array to its stringified version, adding the appropriate quotes around its
/// elements. This is to signify that the elements of the array are strings themselves.
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

/// Converts an array to its stringified version. It will not add quotes around the values of the
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
    state: &Cheatcodes,
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

/// Checks if a key exists in a JSON object.
fn key_exists(json_str: &str, key: &str) -> Result {
    let json: Value =
        serde_json::from_str(json_str).map_err(|e| format!("Could not convert to JSON: {e}"))?;
    let values = jsonpath_lib::select(&json, &canonicalize_json_key(key))?;
    let exists = parse::parse(&(!values.is_empty()).to_string(), &DynSolType::Bool)?;
    Ok(exists)
}

/// Sleeps for a given amount of milliseconds.
fn sleep(milliseconds: &U256) -> Result {
    let sleep_duration = std::time::Duration::from_millis(milliseconds.to::<u64>());
    std::thread::sleep(sleep_duration);

    Ok(Default::default())
}

/// Returns the time since unix epoch in milliseconds
fn duration_since_epoch() -> Result {
    let sys_time = SystemTime::now();
    let difference = sys_time
        .duration_since(UNIX_EPOCH)
        .expect("Failed getting timestamp in unixTime cheatcode");
    let millis = difference.as_millis();
    Ok(DynSolValue::Uint(U256::from(millis), 256).abi_encode().into())
}

/// Skip the current test, by returning a magic value that will be checked by the test runner.
pub fn skip(state: &mut Cheatcodes, depth: u64, skip: bool) -> Result {
    if !skip {
        return Ok(b"".into())
    }

    // Skip should not work if called deeper than at test level.
    // As we're not returning the magic skip bytes, this will cause a test failure.
    if depth > 1 {
        return Err(Error::custom("The skip cheatcode can only be used at test level"))
    }

    state.skip = true;
    Err(Error::custom_bytes(MAGIC_SKIP_BYTES))
}

#[instrument(level = "error", name = "ext", target = "evm::cheatcodes", skip_all)]
pub fn apply<DB: Database>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result> {
    Some(match call {
        HEVMCalls::Ffi(inner) => {
            if state.config.ffi {
                ffi(state, &inner.0)
            } else {
                Err(fmt_err!("FFI disabled: run again with `--ffi` if you want to allow tests to call external scripts."))
            }
        }
        HEVMCalls::TryFfi(inner) => {
            if state.config.ffi {
                try_ffi(state, &inner.0)
            } else {
                Err(fmt_err!("FFI disabled: run again with `--ffi` if you want to allow tests to call external scripts."))
            }
        }
        HEVMCalls::GetCode(inner) => get_code(state, &inner.0),
        HEVMCalls::GetDeployedCode(inner) => get_deployed_code(state, &inner.0),
        HEVMCalls::SetEnv(inner) => set_env(&inner.0, &inner.1),
        HEVMCalls::EnvBool0(inner) => get_env(&inner.0, DynSolType::Bool, None, None),
        HEVMCalls::EnvUint0(inner) => get_env(&inner.0, DynSolType::Uint(256), None, None),
        HEVMCalls::EnvInt0(inner) => get_env(&inner.0, DynSolType::Int(256), None, None),
        HEVMCalls::EnvAddress0(inner) => get_env(&inner.0, DynSolType::Address, None, None),
        HEVMCalls::EnvBytes320(inner) => get_env(&inner.0, DynSolType::FixedBytes(32), None, None),
        HEVMCalls::EnvString0(inner) => get_env(&inner.0, DynSolType::String, None, None),
        HEVMCalls::EnvBytes0(inner) => get_env(&inner.0, DynSolType::Bytes, None, None),
        HEVMCalls::EnvBool1(inner) => get_env(&inner.0, DynSolType::Bool, Some(&inner.1), None),
        HEVMCalls::EnvUint1(inner) => {
            get_env(&inner.0, DynSolType::Uint(256), Some(&inner.1), None)
        }
        HEVMCalls::EnvInt1(inner) => get_env(&inner.0, DynSolType::Int(256), Some(&inner.1), None),
        HEVMCalls::EnvAddress1(inner) => {
            get_env(&inner.0, DynSolType::Address, Some(&inner.1), None)
        }
        HEVMCalls::EnvBytes321(inner) => {
            get_env(&inner.0, DynSolType::FixedBytes(32), Some(&inner.1), None)
        }
        HEVMCalls::EnvString1(inner) => get_env(&inner.0, DynSolType::String, Some(&inner.1), None),
        HEVMCalls::EnvBytes1(inner) => get_env(&inner.0, DynSolType::Bytes, Some(&inner.1), None),
        HEVMCalls::EnvOr0(inner) => {
            get_env(&inner.0, DynSolType::Bool, None, Some(inner.1.to_string()))
        }
        HEVMCalls::EnvOr1(inner) => {
            get_env(&inner.0, DynSolType::Uint(256), None, Some(inner.1.to_string()))
        }
        HEVMCalls::EnvOr2(inner) => {
            get_env(&inner.0, DynSolType::Int(256), None, Some(inner.1.to_string()))
        }
        HEVMCalls::EnvOr3(inner) => {
            get_env(&inner.0, DynSolType::Address, None, Some(hex::encode(inner.1)))
        }
        HEVMCalls::EnvOr4(inner) => {
            get_env(&inner.0, DynSolType::FixedBytes(32), None, Some(hex::encode(inner.1)))
        }
        HEVMCalls::EnvOr5(inner) => {
            get_env(&inner.0, DynSolType::String, None, Some(inner.1.to_string()))
        }
        HEVMCalls::EnvOr6(inner) => {
            get_env(&inner.0, DynSolType::Bytes, None, Some(hex::encode(&inner.1)))
        }
        HEVMCalls::EnvOr7(inner) => get_env(
            &inner.0,
            DynSolType::Bool,
            Some(&inner.1),
            Some(inner.2.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&inner.1)),
        ),
        HEVMCalls::EnvOr8(inner) => get_env(
            &inner.0,
            DynSolType::Uint(256),
            Some(&inner.1),
            Some(inner.2.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&inner.1)),
        ),
        HEVMCalls::EnvOr9(inner) => get_env(
            &inner.0,
            DynSolType::Int(256),
            Some(&inner.1),
            Some(inner.2.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&inner.1)),
        ),
        HEVMCalls::EnvOr10(inner) => get_env(
            &inner.0,
            DynSolType::Address,
            Some(&inner.1),
            Some(inner.2.iter().map(hex::encode).collect::<Vec<_>>().join(&inner.1)),
        ),
        HEVMCalls::EnvOr11(inner) => get_env(
            &inner.0,
            DynSolType::FixedBytes(32),
            Some(&inner.1),
            Some(inner.2.iter().map(hex::encode).collect::<Vec<_>>().join(&inner.1)),
        ),
        HEVMCalls::EnvOr12(inner) => {
            get_env(&inner.0, DynSolType::String, Some(&inner.1), Some(inner.2.join(&inner.1)))
        }
        HEVMCalls::EnvOr13(inner) => get_env(
            &inner.0,
            DynSolType::Bytes,
            Some(&inner.1),
            Some(inner.2.iter().map(hex::encode).collect::<Vec<_>>().join(&inner.1)),
        ),

        // If no key argument is passed, return the whole JSON object.
        // "$" is the JSONPath key for the root of the object
        HEVMCalls::ParseJson0(inner) => parse_json(&inner.0, "$", None),
        HEVMCalls::ParseJson1(inner) => parse_json(&inner.0, &inner.1, None),
        HEVMCalls::ParseJsonBool(inner) => parse_json(&inner.0, &inner.1, Some(DynSolType::Bool)),
        HEVMCalls::ParseJsonKeys(inner) => parse_json_keys(&inner.0, &inner.1),
        HEVMCalls::ParseJsonBoolArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::Bool))
        }
        HEVMCalls::ParseJsonUint(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::Uint(256)))
        }
        HEVMCalls::ParseJsonUintArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::Uint(256)))
        }
        HEVMCalls::ParseJsonInt(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::Int(256)))
        }
        HEVMCalls::ParseJsonIntArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::Int(256)))
        }
        HEVMCalls::ParseJsonString(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::String))
        }
        HEVMCalls::ParseJsonStringArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::String))
        }
        HEVMCalls::ParseJsonAddress(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::Address))
        }
        HEVMCalls::ParseJsonAddressArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::Address))
        }
        HEVMCalls::ParseJsonBytes(inner) => parse_json(&inner.0, &inner.1, Some(DynSolType::Bytes)),
        HEVMCalls::ParseJsonBytesArray(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::Bytes))
        }
        HEVMCalls::ParseJsonBytes32(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::FixedBytes(32)))
        }
        HEVMCalls::ParseJsonBytes32Array(inner) => {
            parse_json(&inner.0, &inner.1, Some(DynSolType::FixedBytes(32)))
        }
        HEVMCalls::SerializeJson(inner) => serialize_json(state, &inner.0, None, &inner.1.pretty()),
        HEVMCalls::SerializeBool0(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &inner.2.pretty())
        }
        HEVMCalls::SerializeBool1(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &array_eval_to_str(&inner.2))
        }
        HEVMCalls::SerializeUint0(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &inner.2.pretty())
        }
        HEVMCalls::SerializeUint1(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &array_eval_to_str(&inner.2))
        }
        HEVMCalls::SerializeInt0(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &inner.2.pretty())
        }
        HEVMCalls::SerializeInt1(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &array_eval_to_str(&inner.2))
        }
        HEVMCalls::SerializeAddress0(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &inner.2.pretty())
        }
        HEVMCalls::SerializeAddress1(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &array_str_to_str(&inner.2))
        }
        HEVMCalls::SerializeBytes320(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &inner.2.pretty())
        }
        HEVMCalls::SerializeBytes321(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &array_str_to_str(&inner.2))
        }
        HEVMCalls::SerializeString0(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &inner.2.pretty())
        }
        HEVMCalls::SerializeString1(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &array_str_to_str(&inner.2))
        }
        HEVMCalls::SerializeBytes0(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &inner.2.pretty())
        }
        HEVMCalls::SerializeBytes1(inner) => {
            serialize_json(state, &inner.0, Some(&inner.1), &array_str_to_str(&inner.2))
        }
        HEVMCalls::Sleep(inner) => sleep(&inner.0.to_alloy()),
        HEVMCalls::UnixTime(_) => duration_since_epoch(),
        HEVMCalls::WriteJson0(inner) => write_json(state, &inner.0, &inner.1, None),
        HEVMCalls::WriteJson1(inner) => write_json(state, &inner.0, &inner.1, Some(&inner.2)),
        HEVMCalls::KeyExists(inner) => key_exists(&inner.0, &inner.1),
        HEVMCalls::Skip(inner) => skip(state, data.journaled_state.depth(), inner.0),
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
