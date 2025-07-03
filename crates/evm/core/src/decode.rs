//! Various utilities to decode test results.

use crate::abi::{Vm, console};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::{Error, JsonAbi};
use alloy_primitives::{Log, Selector, hex, map::HashMap};
use alloy_sol_types::{
    ContractError::Revert, RevertReason, RevertReason::ContractError, SolEventInterface,
    SolInterface, SolValue,
};
use foundry_common::SELECTOR_LEN;
use itertools::Itertools;
use revm::interpreter::InstructionResult;
use std::{fmt, sync::OnceLock};

/// A skip reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkipReason(pub Option<String>);

impl SkipReason {
    /// Decodes a skip reason, if any.
    pub fn decode(raw_result: &[u8]) -> Option<Self> {
        raw_result.strip_prefix(crate::constants::MAGIC_SKIP).map(|reason| {
            let reason = String::from_utf8_lossy(reason).into_owned();
            Self((!reason.is_empty()).then_some(reason))
        })
    }

    /// Decodes a skip reason from a string that was obtained by formatting `Self`.
    ///
    /// This is a hack to support re-decoding a skip reason in proptest.
    pub fn decode_self(s: &str) -> Option<Self> {
        s.strip_prefix("skipped").map(|rest| Self(rest.strip_prefix(": ").map(ToString::to_string)))
    }
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("skipped")?;
        if let Some(reason) = &self.0 {
            f.write_str(": ")?;
            f.write_str(reason)?;
        }
        Ok(())
    }
}

/// Decode a set of logs, only returning logs from DSTest logging events and Hardhat's `console.log`
pub fn decode_console_logs(logs: &[Log]) -> Vec<String> {
    logs.iter().filter_map(decode_console_log).collect()
}

/// Decode a single log.
///
/// This function returns [None] if it is not a DSTest log or the result of a Hardhat
/// `console.log`.
pub fn decode_console_log(log: &Log) -> Option<String> {
    console::ds::ConsoleEvents::decode_log(log).ok().map(|decoded| decoded.to_string())
}

/// Decodes revert data.
#[derive(Clone, Debug, Default)]
pub struct RevertDecoder {
    /// The custom errors to use for decoding.
    pub errors: HashMap<Selector, Vec<Error>>,
}

impl Default for &RevertDecoder {
    fn default() -> Self {
        static EMPTY: OnceLock<RevertDecoder> = OnceLock::new();
        EMPTY.get_or_init(RevertDecoder::new)
    }
}

impl RevertDecoder {
    /// Creates a new, empty revert decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the ABIs to use for error decoding.
    ///
    /// Note that this is decently expensive as it will hash all errors for faster indexing.
    pub fn with_abis<'a>(mut self, abi: impl IntoIterator<Item = &'a JsonAbi>) -> Self {
        self.extend_from_abis(abi);
        self
    }

    /// Sets the ABI to use for error decoding.
    ///
    /// Note that this is decently expensive as it will hash all errors for faster indexing.
    pub fn with_abi(mut self, abi: &JsonAbi) -> Self {
        self.extend_from_abi(abi);
        self
    }

    /// Sets the ABI to use for error decoding, if it is present.
    ///
    /// Note that this is decently expensive as it will hash all errors for faster indexing.
    pub fn with_abi_opt(mut self, abi: Option<&JsonAbi>) -> Self {
        if let Some(abi) = abi {
            self.extend_from_abi(abi);
        }
        self
    }

    /// Extends the decoder with the given ABI's custom errors.
    pub fn extend_from_abis<'a>(&mut self, abi: impl IntoIterator<Item = &'a JsonAbi>) {
        for abi in abi {
            self.extend_from_abi(abi);
        }
    }

    /// Extends the decoder with the given ABI's custom errors.
    pub fn extend_from_abi(&mut self, abi: &JsonAbi) {
        for error in abi.errors() {
            self.push_error(error.clone());
        }
    }

    /// Adds a custom error to use for decoding.
    pub fn push_error(&mut self, error: Error) {
        self.errors.entry(error.selector()).or_default().push(error);
    }

    /// Tries to decode an error message from the given revert bytes.
    ///
    /// Note that this is just a best-effort guess, and should not be relied upon for anything other
    /// than user output.
    pub fn decode(&self, err: &[u8], status: Option<InstructionResult>) -> String {
        self.maybe_decode(err, status).unwrap_or_else(|| {
            if err.is_empty() { "<empty revert data>".to_string() } else { trimmed_hex(err) }
        })
    }

    /// Tries to decode an error message from the given revert bytes.
    ///
    /// See [`decode`](Self::decode) for more information.
    pub fn maybe_decode(&self, err: &[u8], status: Option<InstructionResult>) -> Option<String> {
        if let Some(reason) = SkipReason::decode(err) {
            return Some(reason.to_string());
        }

        // Solidity's `Error(string)` (handled separately in order to strip revert: prefix)
        if let Some(ContractError(Revert(revert))) = RevertReason::decode(err) {
            return Some(revert.reason);
        }

        // Solidity's `Panic(uint256)` and `Vm`'s custom errors.
        if let Ok(e) = alloy_sol_types::ContractError::<Vm::VmErrors>::abi_decode(err) {
            return Some(e.to_string());
        }

        let string_decoded = decode_as_non_empty_string(err);

        if let Some((selector, data)) = err.split_first_chunk::<SELECTOR_LEN>() {
            // Custom errors.
            if let Some(errors) = self.errors.get(selector) {
                for error in errors {
                    // If we don't decode, don't return an error, try to decode as a string
                    // later.
                    if let Ok(decoded) = error.abi_decode_input(data) {
                        return Some(format!(
                            "{}({})",
                            error.name,
                            decoded.iter().map(foundry_common::fmt::format_token).format(", ")
                        ));
                    }
                }
            }

            if string_decoded.is_some() {
                return string_decoded;
            }

            // Generic custom error.
            return Some({
                let mut s = format!("custom error {}", hex::encode_prefixed(selector));
                if !data.is_empty() {
                    s.push_str(": ");
                    match std::str::from_utf8(data) {
                        Ok(data) => s.push_str(data),
                        Err(_) => s.push_str(&hex::encode(data)),
                    }
                }
                s
            });
        }

        if string_decoded.is_some() {
            return string_decoded;
        }

        if let Some(status) = status
            && !status.is_ok()
        {
            return Some(format!("EvmError: {status:?}"));
        }
        if err.is_empty() {
            None
        } else {
            Some(format!("custom error bytes {}", hex::encode_prefixed(err)))
        }
    }
}

/// Helper function that decodes provided error as an ABI encoded or an ASCII string (if not empty).
fn decode_as_non_empty_string(err: &[u8]) -> Option<String> {
    // ABI-encoded `string`.
    if let Ok(s) = String::abi_decode(err)
        && !s.is_empty()
    {
        return Some(s);
    }

    // ASCII string.
    if err.is_ascii() {
        let msg = std::str::from_utf8(err).unwrap().to_string();
        if !msg.is_empty() {
            return Some(msg);
        }
    }

    None
}

fn trimmed_hex(s: &[u8]) -> String {
    let n = 32;
    if s.len() <= n {
        hex::encode(s)
    } else {
        format!(
            "{}…{} ({} bytes)",
            &hex::encode(&s[..n / 2]),
            &hex::encode(&s[s.len() - n / 2..]),
            s.len(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trimmed_hex() {
        assert_eq!(trimmed_hex(&hex::decode("1234567890").unwrap()), "1234567890");
        assert_eq!(
            trimmed_hex(&hex::decode("492077697368207275737420737570706F72746564206869676865722D6B696E646564207479706573").unwrap()),
            "49207769736820727573742073757070…6865722d6b696e646564207479706573 (41 bytes)"
        );
    }

    // https://github.com/foundry-rs/foundry/issues/10162
    #[test]
    fn partial_decode() {
        /*
        error ValidationFailed(bytes);
        error InvalidNonce();
        */
        let mut decoder = RevertDecoder::default();
        decoder.push_error("ValidationFailed(bytes)".parse().unwrap());

        /*
        abi.encodeWithSelector(ValidationFailed.selector, InvalidNonce.selector)
        */
        let data = &hex!(
            "0xe17594de"
            "756688fe00000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(
            decoder.decode(data, None),
            "custom error 0xe17594de: 756688fe00000000000000000000000000000000000000000000000000000000"
        );

        /*
        abi.encodeWithSelector(ValidationFailed.selector, abi.encodeWithSelector(InvalidNonce.selector))
        */
        let data = &hex!(
            "0xe17594de"
            "0000000000000000000000000000000000000000000000000000000000000020"
            "0000000000000000000000000000000000000000000000000000000000000004"
            "756688fe00000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(decoder.decode(data, None), "ValidationFailed(0x756688fe)");
    }
}
