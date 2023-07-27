//! utilities used within tracing

use crate::{debug::DebugArena, decode};
use ethers::{
    abi::{Abi, Address, Function, ParamType, Token},
    core::utils::to_checksum,
    solc::{artifacts::ContractBytecodeSome, ArtifactId},
};
use foundry_common::{abi::format_token, SELECTOR_LEN};
use foundry_config::{Chain, Config};
use std::{
    collections::{BTreeMap, HashMap},
    str::FromStr,
};
use yansi::Paint;

use super::{
    identifier::{EtherscanIdentifier, SignaturesIdentifier},
    CallTraceDecoder, CallTraceDecoderBuilder, Traces,
};

/// Returns the label for the given `token`
///
/// If the `token` is an `Address` then we look abel the label map.
/// by default the token is formatted using standard formatting
pub fn label(token: &Token, labels: &HashMap<Address, String>) -> String {
    match token {
        Token::Address(addr) => {
            if let Some(label) = labels.get(addr) {
                format!("{label}: [{}]", to_checksum(addr, None))
            } else {
                format_token(token)
            }
        }
        _ => format_token(token),
    }
}

/// Custom decoding of cheatcode calls
pub(crate) fn decode_cheatcode_inputs(
    func: &Function,
    data: &[u8],
    errors: &Abi,
    verbosity: u8,
) -> Option<Vec<String>> {
    match func.name.as_str() {
        "expectRevert" => {
            decode::decode_revert(data, Some(errors), None).ok().map(|decoded| vec![decoded])
        }
        "rememberKey" | "addr" | "startBroadcast" | "broadcast" => {
            // these functions accept a private key as uint256, which should not be
            // converted to plain text
            if !func.inputs.is_empty() && matches!(&func.inputs[0].kind, ParamType::Uint(_)) {
                // redact private key input
                Some(vec!["<pk>".to_string()])
            } else {
                None
            }
        }
        "sign" => {
            // sign(uint256,bytes32)
            let mut decoded = func.decode_input(&data[SELECTOR_LEN..]).ok()?;
            if !decoded.is_empty() && matches!(&func.inputs[0].kind, ParamType::Uint(_)) {
                decoded[0] = Token::String("<pk>".to_string());
            }
            Some(decoded.iter().map(format_token).collect())
        }
        "deriveKey" => Some(vec!["<pk>".to_string()]),
        "parseJson"
        | "parseJsonUint"
        | "parseJsonUintArray"
        | "parseJsonInt"
        | "parseJsonIntArray"
        | "parseJsonString"
        | "parseJsonStringArray"
        | "parseJsonAddress"
        | "parseJsonAddressArray"
        | "parseJsonBool"
        | "parseJsonBoolArray"
        | "parseJsonBytes"
        | "parseJsonBytesArray"
        | "parseJsonBytes32"
        | "parseJsonBytes32Array"
        | "writeJson"
        | "keyExists"
        | "serializeBool"
        | "serializeUint"
        | "serializeInt"
        | "serializeAddress"
        | "serializeBytes32"
        | "serializeString"
        | "serializeBytes" => {
            if verbosity == 5 {
                None
            } else {
                let mut decoded = func.decode_input(&data[SELECTOR_LEN..]).ok()?;
                let token =
                    if func.name.as_str() == "parseJson" || func.name.as_str() == "keyExists" {
                        "<JSON file>"
                    } else {
                        "<stringified JSON>"
                    };
                decoded[0] = Token::String(token.to_string());
                Some(decoded.iter().map(format_token).collect())
            }
        }
        _ => None,
    }
}

/// Custom decoding of cheatcode return values
pub(crate) fn decode_cheatcode_outputs(
    func: &Function,
    _data: &[u8],
    verbosity: u8,
) -> Option<String> {
    if func.name.starts_with("env") {
        // redacts the value stored in the env var
        return Some("<env var value>".to_string());
    }
    if func.name == "deriveKey" {
        // redacts derived private key
        return Some("<pk>".to_string());
    }
    if func.name == "parseJson" && verbosity != 5 {
        return Some("<encoded JSON value>".to_string());
    }
    if func.name == "readFile" && verbosity != 5 {
        return Some("<file>".to_string());
    }
    None
}

pub async fn print_traces(
    result: &mut TraceResult,
    decoder: CallTraceDecoder,
    verbose: bool,
) -> eyre::Result<()> {
    if result.traces.is_empty() {
        eyre::bail!("Unexpected error: No traces. Please report this as a bug: https://github.com/foundry-rs/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
    }

    println!("Traces:");
    for (_, trace) in &mut result.traces {
        decoder.decode(trace).await;
        if !verbose {
            println!("{trace}");
        } else {
            println!("{trace:#}");
        }
    }
    println!();

    if result.success {
        println!("{}", Paint::green("Transaction successfully executed."));
    } else {
        println!("{}", Paint::red("Transaction failed."));
    }

    println!("Gas used: {}", result.gas_used);
    Ok(())
}

pub struct TraceResult {
    pub success: bool,
    pub traces: Traces,
    pub debug: DebugArena,
    pub gas_used: u64,
}
