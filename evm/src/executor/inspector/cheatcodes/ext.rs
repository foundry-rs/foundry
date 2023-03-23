use crate::{
    abi::HEVMCalls,
    error,
    executor::inspector::{
        cheatcodes::{util, util::parse},
        Cheatcodes,
    },
};
use bytes::Bytes;
use ethers::{
    abi::{self, AbiEncode, JsonAbi, ParamType, Token},
    prelude::artifacts::CompactContractBytecode,
    types::*,
};
use foundry_common::{fmt::*, fs, get_artifact_path};
use foundry_config::fs_permissions::FsAccessKind;
use hex::FromHex;
use jsonpath_lib;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    env,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::Command,
    str::FromStr,
    time::UNIX_EPOCH,
};
use tracing::{error, trace};
/// Invokes a `Command` with the given args and returns the abi encoded response
///
/// If the output of the command is valid hex, it returns the hex decoded value
fn ffi(state: &Cheatcodes, args: &[String]) -> Result<Bytes, Bytes> {
    if args.is_empty() || args[0].is_empty() {
        return Err(error::encode_error("Can't execute empty command"))
    }
    let mut cmd = Command::new(&args[0]);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }

    trace!(?args, "invoking ffi");

    let output = cmd
        .current_dir(&state.config.root)
        .output()
        .map_err(|err| error::encode_error(format!("Failed to execute command: {err}")))?;

    if !output.stderr.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr);
        error!(?err, "stderr");
    }

    let output = String::from_utf8(output.stdout)
        .map_err(|err| error::encode_error(format!("Failed to decode non utf-8 output: {err}")))?;

    let trim_out = output.trim();
    if let Ok(hex_decoded) = hex::decode(trim_out.strip_prefix("0x").unwrap_or(trim_out)) {
        return Ok(abi::encode(&[Token::Bytes(hex_decoded.to_vec())]).into())
    }

    Ok(trim_out.to_string().encode().into())
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
    fn into_bytecode(self) -> Option<ethers::types::Bytes> {
        match self {
            ArtifactBytecode::Hardhat(inner) => Some(inner.bytecode),
            ArtifactBytecode::Forge(inner) => {
                inner.bytecode.and_then(|bytecode| bytecode.object.into_bytes())
            }
            ArtifactBytecode::Solc(inner) => inner.bytecode(),
            ArtifactBytecode::Huff(inner) => Some(inner.bytecode),
        }
    }

    fn into_deployed_bytecode(self) -> Option<ethers::types::Bytes> {
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
    #[serde(deserialize_with = "ethers::solc::artifacts::deserialize_bytes")]
    bytecode: ethers::types::Bytes,
    #[serde(deserialize_with = "ethers::solc::artifacts::deserialize_bytes")]
    deployed_bytecode: ethers::types::Bytes,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HuffArtifact {
    #[serde(deserialize_with = "ethers::solc::artifacts::deserialize_bytes")]
    bytecode: ethers::types::Bytes,
    #[serde(deserialize_with = "ethers::solc::artifacts::deserialize_bytes")]
    runtime: ethers::types::Bytes,
}

/// Returns the _deployed_ bytecode (`bytecode`) of the matching artifact
fn get_code(state: &Cheatcodes, path: &str) -> Result<Bytes, Bytes> {
    let bytecode = read_bytecode(state, path)?;
    if let Some(bin) = bytecode.into_bytecode() {
        Ok(abi::encode(&[Token::Bytes(bin.to_vec())]).into())
    } else {
        Err(error::encode_error("No bytecode for contract. Is it abstract or unlinked?"))
    }
}

/// Returns the _deployed_ bytecode (`bytecode`) of the matching artifact
fn get_deployed_code(state: &Cheatcodes, path: &str) -> Result<Bytes, Bytes> {
    let bytecode = read_bytecode(state, path)?;
    if let Some(bin) = bytecode.into_deployed_bytecode() {
        Ok(abi::encode(&[Token::Bytes(bin.to_vec())]).into())
    } else {
        Err(error::encode_error("No bytecode for contract. Is it abstract or unlinked?"))
    }
}

/// Reads the bytecode object(s) from the matching artifact
fn read_bytecode(state: &Cheatcodes, path: &str) -> Result<ArtifactBytecode, Bytes> {
    let path = get_artifact_path(&state.config.paths, path);
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Read).map_err(error::encode_error)?;

    let data = fs::read_to_string(path).map_err(error::encode_error)?;
    serde_json::from_str::<ArtifactBytecode>(&data).map_err(error::encode_error)
}

fn set_env(key: &str, val: &str) -> Result<Bytes, Bytes> {
    // `std::env::set_var` may panic in the following situations
    // ref: https://doc.rust-lang.org/std/env/fn.set_var.html
    if key.is_empty() {
        Err(error::encode_error("Environment variable key can't be empty"))
    } else if key.contains('=') {
        Err(error::encode_error("Environment variable key can't contain equal sign `=`"))
    } else if key.contains('\0') {
        Err(error::encode_error("Environment variable key can't contain NUL character `\\0`"))
    } else if val.contains('\0') {
        Err(error::encode_error("Environment variable value can't contain NUL character `\\0`"))
    } else {
        env::set_var(key, val);
        Ok(Bytes::new())
    }
}

fn get_env(
    key: &str,
    r#type: ParamType,
    delim: Option<&str>,
    default: Option<String>,
) -> Result<Bytes, Bytes> {
    let msg = format!("Failed to get environment variable `{key}` as type `{}`", &r#type);
    let val = if let Some(value) = default {
        env::var(key).unwrap_or(value)
    } else {
        env::var(key).map_err::<Bytes, _>(|e| error::encode_error(format!("{msg}: {e}")))?
    };
    let val = if let Some(d) = delim {
        val.split(d).map(|v| v.trim().to_string()).collect()
    } else {
        vec![val]
    };
    let is_array: bool = delim.is_some();
    util::value_to_abi(val, r#type, is_array)
        .map_err(|e| error::encode_error(format!("{msg}: {e}")))
}

fn project_root(state: &Cheatcodes) -> Result<Bytes, Bytes> {
    let root = state.config.root.display().to_string();

    Ok(abi::encode(&[Token::String(root)]).into())
}

fn read_file(state: &Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(&path, FsAccessKind::Read).map_err(error::encode_error)?;

    let data = fs::read_to_string(path).map_err(error::encode_error)?;

    Ok(abi::encode(&[Token::String(data)]).into())
}

fn read_file_binary(state: &Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(&path, FsAccessKind::Read).map_err(error::encode_error)?;

    let data = fs::read(path).map_err(error::encode_error)?;

    Ok(abi::encode(&[Token::Bytes(data)]).into())
}

fn read_line(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(&path, FsAccessKind::Read).map_err(error::encode_error)?;

    // Get reader for previously opened file to continue reading OR initialize new reader
    let reader = state
        .context
        .opened_read_files
        .entry(path.clone())
        .or_insert(BufReader::new(fs::open(path).map_err(error::encode_error)?));

    let mut line: String = String::new();
    reader.read_line(&mut line).map_err(error::encode_error)?;

    // Remove trailing newline character, preserving others for cases where it may be important
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }

    Ok(abi::encode(&[Token::String(line)]).into())
}

/// Writes the content to the file
///
/// This function will create a file if it does not exist, and will entirely replace its contents if
/// it does.
///
/// Caution: writing files is only allowed if the targeted path is allowed, (inside `<root>/` by
/// default)
fn write_file(
    state: &Cheatcodes,
    path: impl AsRef<Path>,
    content: impl AsRef<[u8]>,
) -> Result<Bytes, Bytes> {
    let path = state
        .config
        .ensure_path_allowed(&path, FsAccessKind::Write)
        .map_err(error::encode_error)?;
    // write access to foundry.toml is not allowed
    state.config.ensure_not_foundry_toml(&path).map_err(error::encode_error)?;

    if state.fs_commit {
        fs::write(path, content.as_ref()).map_err(error::encode_error)?;
    }

    Ok(Bytes::new())
}

/// Writes a single line to the file
///
/// This will create a file if it does not exist but append the `line` if it does
fn write_line(state: &Cheatcodes, path: impl AsRef<Path>, line: &str) -> Result<Bytes, Bytes> {
    let path = state
        .config
        .ensure_path_allowed(&path, FsAccessKind::Write)
        .map_err(error::encode_error)?;
    state.config.ensure_not_foundry_toml(&path).map_err(error::encode_error)?;

    if state.fs_commit {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .map_err(error::encode_error)?;

        writeln!(file, "{line}").map_err(error::encode_error)?;
    }

    Ok(Bytes::new())
}

fn close_file(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(&path, FsAccessKind::Read).map_err(error::encode_error)?;

    state.context.opened_read_files.remove(&path);

    Ok(Bytes::new())
}

/// Removes a file from the filesystem.
///
/// Only files inside `<root>/` can be removed, `foundry.toml` excluded.
///
/// This will return an error if the path points to a directory, or the file does not exist
fn remove_file(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path = state
        .config
        .ensure_path_allowed(&path, FsAccessKind::Write)
        .map_err(error::encode_error)?;
    state.config.ensure_not_foundry_toml(&path).map_err(error::encode_error)?;

    // also remove from the set if opened previously
    state.context.opened_read_files.remove(&path);

    if state.fs_commit {
        fs::remove_file(&path).map_err(error::encode_error)?;
    }

    Ok(Bytes::new())
}

/// Gets the metadata of a file/directory
///
/// This will return an error if no file/directory is found, or if the target path isn't allowed
fn fs_metadata(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(&path, FsAccessKind::Read).map_err(error::encode_error)?;

    let metadata = path.metadata().map_err(error::encode_error)?;

    // These fields not available on all platforms; default to 0
    let [modified, accessed, created] =
        [metadata.modified(), metadata.accessed(), metadata.created()].map(|time| {
            time.unwrap_or(UNIX_EPOCH).duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
        });

    let metadata = (
        metadata.is_dir(),
        metadata.is_symlink(),
        metadata.len(),
        metadata.permissions().readonly(),
        modified,
        accessed,
        created,
    );
    Ok(metadata.encode().into())
}

/// Converts a serde_json::Value to an abi::Token
/// The function is designed to run recursively, so that in case of an object
/// it will call itself to convert each of it's value and encode the whole as a
/// Tuple
fn value_to_token(value: &Value) -> eyre::Result<Token> {
    if let Some(boolean) = value.as_bool() {
        Ok(Token::Bool(boolean))
    } else if let Some(string) = value.as_str() {
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
    } else if let Ok(number) = U256::from_dec_str(&value.to_string()) {
        Ok(Token::Uint(number))
    } else if let Ok(number) = I256::from_dec_str(&value.to_string()) {
        Ok(Token::Int(number.into_raw()))
    } else if let Some(array) = value.as_array() {
        Ok(Token::Array(array.iter().map(value_to_token).collect::<eyre::Result<Vec<_>>>()?))
    } else if value.as_object().is_some() {
        let ordered_object: BTreeMap<String, Value> =
            serde_json::from_value(value.clone()).unwrap();
        let values =
            ordered_object.values().map(value_to_token).collect::<eyre::Result<Vec<_>>>()?;
        Ok(Token::Tuple(values))
    } else if value.is_null() {
        Ok(Token::FixedBytes(vec![0; 32]))
    } else {
        eyre::bail!("Unexpected json value: {}", value)
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
fn parse_json(
    _state: &mut Cheatcodes,
    json_str: &str,
    key: &str,
    coerce: Option<ParamType>,
) -> Result<Bytes, Bytes> {
    let json = serde_json::from_str(json_str).map_err(error::encode_error)?;
    let values: Vec<&Value> =
        jsonpath_lib::select(&json, &canonicalize_json_key(key)).map_err(error::encode_error)?;
    // values is an array of items. Depending on the JsonPath key, they
    // can be many or a single item. An item can be a single value or
    // an entire JSON object.
    if let Some(coercion_type) = coerce {
        if values.iter().any(|value| value.is_object()) {
            return Err(error::encode_error(format!(
                "You can only coerce values or arrays, not JSON objects. The key '{key}' returns an object",
            )))
        }

        let final_val = if let Some(array) = values[0].as_array() {
            array.iter().map(|v| v.to_string().replace('\"', "")).collect::<Vec<String>>()
        } else {
            vec![values[0].to_string().replace('\"', "")]
        };
        let bytes = parse(final_val, coercion_type, values[0].is_array());
        return bytes
    }
    let res = values
        .iter()
        .map(|inner| {
            value_to_token(inner).map_err(|err| {
                error::encode_error(err.wrap_err(format!("Failed to parse key {key}")))
            })
        })
        .collect::<Result<Vec<Token>, Bytes>>()?;
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
) -> Result<Bytes, Bytes> {
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
        .map_err(|err| error::encode_error(format!("Failed to stringify hashmap: {err}")))?;
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
    _state: &mut Cheatcodes,
    object: &str,
    path: impl AsRef<Path>,
    json_path_or_none: Option<&str>,
) -> Result<Bytes, Bytes> {
    let json: Value =
        serde_json::from_str(object).unwrap_or_else(|_| Value::String(object.to_owned()));
    let json_string = serde_json::to_string_pretty(&if let Some(json_path) = json_path_or_none {
        let path = _state
            .config
            .ensure_path_allowed(&path, FsAccessKind::Read)
            .map_err(error::encode_error)?;
        let data = serde_json::from_str(&fs::read_to_string(path).map_err(error::encode_error)?)
            .map_err(error::encode_error)?;
        jsonpath_lib::replace_with(data, &canonicalize_json_key(json_path), &mut |_| {
            Some(json.clone())
        })
        .map_err(error::encode_error)?
    } else {
        json
    })
    .map_err(error::encode_error)?;
    write_file(_state, path, json_string)?;
    Ok(Bytes::new())
}

pub fn apply(
    state: &mut Cheatcodes,
    ffi_enabled: bool,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::Ffi(inner) => {
            if !ffi_enabled {
                Err(error::encode_error("FFI disabled: run again with `--ffi` if you want to allow tests to call external scripts."))
            } else {
                ffi(state, &inner.0)
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

        HEVMCalls::ProjectRoot(_) => project_root(state),
        HEVMCalls::ReadFile(inner) => read_file(state, &inner.0),
        HEVMCalls::ReadFileBinary(inner) => read_file_binary(state, &inner.0),
        HEVMCalls::ReadLine(inner) => read_line(state, &inner.0),
        HEVMCalls::WriteFile(inner) => write_file(state, &inner.0, &inner.1),
        HEVMCalls::WriteFileBinary(inner) => write_file(state, &inner.0, &inner.1),
        HEVMCalls::WriteLine(inner) => write_line(state, &inner.0, &inner.1),
        HEVMCalls::CloseFile(inner) => close_file(state, &inner.0),
        HEVMCalls::RemoveFile(inner) => remove_file(state, &inner.0),
        HEVMCalls::FsMetadata(inner) => fs_metadata(state, &inner.0),
        // If no key argument is passed, return the whole JSON object.
        // "$" is the JSONPath key for the root of the object
        HEVMCalls::ParseJson0(inner) => parse_json(state, &inner.0, "$", None),
        HEVMCalls::ParseJson1(inner) => parse_json(state, &inner.0, &inner.1, None),
        HEVMCalls::ParseJsonBool(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::Bool))
        }
        HEVMCalls::ParseJsonBoolArray(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::Bool))
        }
        HEVMCalls::ParseJsonUint(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::Uint(256)))
        }
        HEVMCalls::ParseJsonUintArray(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::Uint(256)))
        }
        HEVMCalls::ParseJsonInt(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::Int(256)))
        }
        HEVMCalls::ParseJsonIntArray(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::Int(256)))
        }
        HEVMCalls::ParseJsonString(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::String))
        }
        HEVMCalls::ParseJsonStringArray(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::String))
        }
        HEVMCalls::ParseJsonAddress(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::Address))
        }
        HEVMCalls::ParseJsonAddressArray(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::Address))
        }
        HEVMCalls::ParseJsonBytes(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::Bytes))
        }
        HEVMCalls::ParseJsonBytesArray(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::Bytes))
        }
        HEVMCalls::ParseJsonBytes32(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::FixedBytes(32)))
        }
        HEVMCalls::ParseJsonBytes32Array(inner) => {
            parse_json(state, &inner.0, &inner.1, Some(ParamType::FixedBytes(32)))
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
