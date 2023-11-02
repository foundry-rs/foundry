//! Helpers for formatting ethereum types

use crate::{calc::to_exp_notation, TransactionReceiptWithRevertReason};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{hex, U256};
use eyre::Result;
use std::fmt::{self, Debug, Display};
use yansi::Paint;

pub use foundry_macros::fmt::*;

/// [`DynSolValue`] formatter.
struct DynValueFormatter {
    raw: bool,
}

impl DynValueFormatter {
    /// Recursively formats a [`DynSolValue`].
    fn value(&self, value: &DynSolValue, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match value {
            DynSolValue::Address(inner) => Display::fmt(inner, f),
            DynSolValue::Function(inner) => Display::fmt(inner, f),
            DynSolValue::Bytes(inner) => f.write_str(&hex::encode_prefixed(inner)),
            DynSolValue::FixedBytes(inner, _) => f.write_str(&hex::encode_prefixed(inner)),
            DynSolValue::Uint(inner, _) => {
                if self.raw {
                    write!(f, "{inner}")
                } else {
                    f.write_str(&format_uint_exp(*inner))
                }
            }
            DynSolValue::Int(inner, _) => write!(f, "{inner}"),
            DynSolValue::Array(values) | DynSolValue::FixedArray(values) => {
                f.write_str("[")?;
                self.list(values, f)?;
                f.write_str("]")
            }
            DynSolValue::Tuple(values) => self.tuple(values, f),
            DynSolValue::String(inner) => Debug::fmt(inner, f),
            DynSolValue::Bool(inner) => Display::fmt(inner, f),
            DynSolValue::CustomStruct { name, prop_names, tuple } => {
                if self.raw {
                    return self.tuple(tuple, f);
                }

                f.write_str(name)?;
                f.write_str(" { ")?;

                for (i, (prop_name, value)) in std::iter::zip(prop_names, tuple).enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    f.write_str(prop_name)?;
                    f.write_str(": ")?;
                    self.value(value, f)?;
                }

                f.write_str(" }")
            }
        }
    }

    /// Recursively formats a comma-separated list of [`DynSolValue`]s.
    fn list(&self, values: &[DynSolValue], f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, value) in values.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            self.value(value, f)?;
        }
        Ok(())
    }

    /// Formats the given values as a tuple.
    fn tuple(&self, values: &[DynSolValue], f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(")?;
        self.list(values, f)?;
        f.write_str(")")
    }
}

/// Wrapper that implements [`Display`] for a [`DynSolValue`].
struct DynValueDisplay<'a> {
    /// The value to display.
    value: &'a DynSolValue,
    /// The formatter.
    formatter: DynValueFormatter,
}

impl<'a> DynValueDisplay<'a> {
    /// Creates a new [`Display`] wrapper for the given value.
    #[inline]
    fn new(value: &'a DynSolValue, raw: bool) -> Self {
        Self { value, formatter: DynValueFormatter { raw } }
    }
}

impl fmt::Display for DynValueDisplay<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.formatter.value(self.value, f)
    }
}

/// Parses string input as Token against the expected ParamType
pub fn parse_tokens<'a, I: IntoIterator<Item = (&'a DynSolType, &'a str)>>(
    params: I,
) -> Result<Vec<DynSolValue>> {
    let mut tokens = Vec::new();
    for (param, value) in params {
        let token = DynSolType::coerce_str(param, value)?;
        tokens.push(token);
    }
    Ok(tokens)
}

/// Pretty-prints a slice of tokens using [`format_token`].
pub fn format_tokens(tokens: &[DynSolValue]) -> impl Iterator<Item = String> + '_ {
    tokens.iter().map(format_token)
}

/// Pretty-prints the given value into a string suitable for user output.
pub fn format_token(value: &DynSolValue) -> String {
    DynValueDisplay::new(value, false).to_string()
}

/// Pretty-prints the given value into a string suitable for re-parsing as values later.
///
/// This means:
/// - integers are not formatted with exponential notation hints
/// - structs are formatted as tuples, losing the struct and property names
pub fn format_token_raw(value: &DynSolValue) -> String {
    DynValueDisplay::new(value, true).to_string()
}

/// Formats a U256 number to string, adding an exponential notation _hint_ if it
/// is larger than `10_000`, with a precision of `4` figures, and trimming the
/// trailing zeros.
///
/// # Examples
///
/// ```
/// use alloy_primitives::U256;
/// use foundry_common::fmt::format_uint_exp as f;
///
/// yansi::Paint::disable();
/// assert_eq!(f(U256::from(0)), "0");
/// assert_eq!(f(U256::from(1234)), "1234");
/// assert_eq!(f(U256::from(1234567890)), "1234567890 [1.234e9]");
/// assert_eq!(f(U256::from(1000000000000000000_u128)), "1000000000000000000 [1e18]");
/// assert_eq!(f(U256::from(10000000000000000000000_u128)), "10000000000000000000000 [1e22]");
/// ```
pub fn format_uint_exp(num: U256) -> String {
    if num < U256::from(10_000) {
        return num.to_string()
    }

    let exp = to_exp_notation(num, 4, true);
    format!("{} {}", num, Paint::default(format!("[{exp}]")).dimmed())
}

impl UIfmt for TransactionReceiptWithRevertReason {
    fn pretty(&self) -> String {
        if let Some(revert_reason) = &self.revert_reason {
            format!(
                "{}
revertReason            {}",
                self.receipt.pretty(),
                revert_reason
            )
        } else {
            self.receipt.pretty()
        }
    }
}

/// Returns the ``UiFmt::pretty()` formatted attribute of the transaction receipt
pub fn get_pretty_tx_receipt_attr(
    receipt: &TransactionReceiptWithRevertReason,
    attr: &str,
) -> Option<String> {
    match attr {
        "blockHash" | "block_hash" => Some(receipt.receipt.block_hash.pretty()),
        "blockNumber" | "block_number" => Some(receipt.receipt.block_number.pretty()),
        "contractAddress" | "contract_address" => Some(receipt.receipt.contract_address.pretty()),
        "cumulativeGasUsed" | "cumulative_gas_used" => {
            Some(receipt.receipt.cumulative_gas_used.pretty())
        }
        "effectiveGasPrice" | "effective_gas_price" => {
            Some(receipt.receipt.effective_gas_price.pretty())
        }
        "gasUsed" | "gas_used" => Some(receipt.receipt.gas_used.pretty()),
        "logs" => Some(receipt.receipt.logs.pretty()),
        "logsBloom" | "logs_bloom" => Some(receipt.receipt.logs_bloom.pretty()),
        "root" => Some(receipt.receipt.root.pretty()),
        "status" => Some(receipt.receipt.status.pretty()),
        "transactionHash" | "transaction_hash" => Some(receipt.receipt.transaction_hash.pretty()),
        "transactionIndex" | "transaction_index" => {
            Some(receipt.receipt.transaction_index.pretty())
        }
        "type" | "transaction_type" => Some(receipt.receipt.transaction_type.pretty()),
        "revertReason" | "revert_reason" => Some(receipt.revert_reason.pretty()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn parse_hex_uint() {
        let ty = DynSolType::Uint(256);

        let values = parse_tokens(std::iter::once((&ty, "100"))).unwrap();
        assert_eq!(values, [DynSolValue::Uint(U256::from(100), 256)]);

        let val: U256 = U256::from(100u64);
        let hex_val = format!("0x{val:x}");
        let values = parse_tokens(std::iter::once((&ty, hex_val.as_str()))).unwrap();
        assert_eq!(values, [DynSolValue::Uint(U256::from(100), 256)]);
    }

    #[test]
    fn format_addr() {
        // copied from testcases in https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md
        assert_eq!(
            format_token(&DynSolValue::Address(address!(
                "5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"
            ))),
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
        );

        // copied from testcases in https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1191.md
        assert_ne!(
            format_token(&DynSolValue::Address(address!(
                "Fb6916095cA1Df60bb79ce92cE3EA74c37c5d359"
            ))),
            "0xFb6916095cA1Df60bb79ce92cE3EA74c37c5d359"
        );
    }
}
