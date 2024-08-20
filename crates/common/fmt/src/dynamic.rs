use super::{format_int_exp, format_uint_exp};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::hex;
use std::fmt;

/// [`DynSolValue`] formatter.
struct DynValueFormatter {
    raw: bool,
}

impl DynValueFormatter {
    /// Recursively formats a [`DynSolValue`].
    fn value(&self, value: &DynSolValue, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match value {
            DynSolValue::Address(inner) => write!(f, "{inner}"),
            DynSolValue::Function(inner) => write!(f, "{inner}"),
            DynSolValue::Bytes(inner) => f.write_str(&hex::encode_prefixed(inner)),
            DynSolValue::FixedBytes(word, size) => {
                f.write_str(&hex::encode_prefixed(&word[..*size]))
            }
            DynSolValue::Uint(inner, _) => {
                if self.raw {
                    write!(f, "{inner}")
                } else {
                    f.write_str(&format_uint_exp(*inner))
                }
            }
            DynSolValue::Int(inner, _) => {
                if self.raw {
                    write!(f, "{inner}")
                } else {
                    f.write_str(&format_int_exp(*inner))
                }
            }
            DynSolValue::Array(values) | DynSolValue::FixedArray(values) => {
                f.write_str("[")?;
                self.list(values, f)?;
                f.write_str("]")
            }
            DynSolValue::Tuple(values) => self.tuple(values, f),
            DynSolValue::String(inner) => write!(f, "{inner:?}"), // escape strings
            DynSolValue::Bool(inner) => write!(f, "{inner}"),
            DynSolValue::CustomStruct { name, prop_names, tuple } => {
                if self.raw {
                    return self.tuple(tuple, f);
                }

                f.write_str(name)?;

                if prop_names.len() == tuple.len() {
                    f.write_str("({ ")?;

                    for (i, (prop_name, value)) in std::iter::zip(prop_names, tuple).enumerate() {
                        if i > 0 {
                            f.write_str(", ")?;
                        }
                        f.write_str(prop_name)?;
                        f.write_str(": ")?;
                        self.value(value, f)?;
                    }

                    f.write_str(" })")
                } else {
                    self.tuple(tuple, f)
                }
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

/// Wrapper that implements [`Display`](fmt::Display) for a [`DynSolValue`].
struct DynValueDisplay<'a> {
    /// The value to display.
    value: &'a DynSolValue,
    /// The formatter.
    formatter: DynValueFormatter,
}

impl<'a> DynValueDisplay<'a> {
    /// Creates a new [`Display`](fmt::Display) wrapper for the given value.
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
) -> alloy_dyn_abi::Result<Vec<DynSolValue>> {
    params.into_iter().map(|(param, value)| DynSolType::coerce_str(param, value)).collect()
}

/// Pretty-prints a slice of tokens using [`format_token`].
pub fn format_tokens(tokens: &[DynSolValue]) -> impl Iterator<Item = String> + '_ {
    tokens.iter().map(format_token)
}

/// Pretty-prints a slice of tokens using [`format_token_raw`].
pub fn format_tokens_raw(tokens: &[DynSolValue]) -> impl Iterator<Item = String> + '_ {
    tokens.iter().map(format_token_raw)
}

/// Prints slice of tokens using [`format_tokens`] or [`format_tokens_raw`] depending on `json`
/// parameter.
pub fn print_tokens(tokens: &[DynSolValue], json: bool) {
    if json {
        let tokens: Vec<String> = format_tokens_raw(tokens).collect();
        println!("{}", serde_json::to_string_pretty(&tokens).unwrap());
    } else {
        let tokens = format_tokens(tokens);
        tokens.for_each(|t| println!("{t}"));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, U256};

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
