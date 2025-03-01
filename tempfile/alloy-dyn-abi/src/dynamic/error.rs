use crate::{DynSolType, DynSolValue, Error, Result};
use alloc::vec::Vec;
use alloy_primitives::Selector;
use alloy_sol_types::SolError;

/// See [alloy_sol_types::Panic] for signature details.
const PANIC_SELECTOR: Selector = Selector::new(alloy_sol_types::Panic::SELECTOR);
/// See [alloy_sol_types::Revert] for signature details.
const REVERT_SELECTOR: Selector = Selector::new(alloy_sol_types::Revert::SELECTOR);

/// A dynamic ABI error.
///
/// This is a representation of a Solidity error, which can be used to decode
/// error events.
#[derive(Clone, Debug, PartialEq)]
pub struct DynSolError {
    /// Error selector.
    pub(crate) selector: Selector,
    /// Error body types. MUST be a tuple.
    pub(crate) body: DynSolType,
}

impl DynSolError {
    /// Represents a standard Solidity revert. These are thrown by
    /// `revert(reason)` or `require(condition, reason)` statements in Solidity.
    ///
    /// **Note**: Usage of this instantiator is not recommended. It is better to
    /// use [alloy_sol_types::Revert] in almost all cases.
    pub fn revert() -> Self {
        Self { selector: REVERT_SELECTOR, body: DynSolType::Tuple(vec![DynSolType::String]) }
    }

    /// A [Solidity panic].
    ///
    /// **Note**: Usage of this instantiator is not recommended. It is better to
    /// use [alloy_sol_types::Panic] in almost all cases.
    ///
    /// These are thrown by `assert(condition)` and by internal Solidity checks,
    /// such as arithmetic overflow or array bounds checks.
    ///
    /// The list of all known panic codes can be found in the [PanicKind] enum.
    ///
    /// [Solidity panic]: https://docs.soliditylang.org/en/latest/control-structures.html#panic-via-assert-and-error-via-require
    /// [PanicKind]: alloy_sol_types::PanicKind
    pub fn panic() -> Self {
        Self { selector: PANIC_SELECTOR, body: DynSolType::Tuple(vec![DynSolType::Uint(256)]) }
    }

    /// Creates a new error, without length-checking the body. This allows
    /// creation of invalid errors.
    pub const fn new_unchecked(selector: Selector, body: DynSolType) -> Self {
        Self { selector, body }
    }

    /// Creates a new error from a selector.
    pub fn new(selector: Selector, body: DynSolType) -> Option<Self> {
        let _ = body.as_tuple()?;
        Some(Self::new_unchecked(selector, body))
    }

    /// Error selector is the first 4 bytes of the keccak256 hash of the error
    /// declaration.
    pub const fn selector(&self) -> Selector {
        self.selector
    }

    /// Error body types.
    pub fn body(&self) -> &[DynSolType] {
        self.body.as_tuple().expect("body is a tuple")
    }

    /// Decode the error from the given data, which must already be stripped of
    /// its selector.
    fn decode_error_body(&self, data: &[u8]) -> Result<DecodedError> {
        let body = self.body.abi_decode_sequence(data)?.into_fixed_seq().expect("body is a tuple");
        Ok(DecodedError { body })
    }

    /// Decode the error from the given data.
    pub fn decode_error(&self, data: &[u8]) -> Result<DecodedError> {
        // Check selector validity.
        if !data.starts_with(self.selector.as_slice()) {
            return Err(Error::SelectorMismatch {
                expected: self.selector,
                actual: Selector::from_slice(&data[0..4]),
            });
        }

        // will not panic, as we've already checked the length with starts_with
        let data = data.split_at(4).1;
        self.decode_error_body(data)
    }
}

/// A decoded dynamic ABI error.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedError {
    /// The decoded error body.
    pub body: Vec<DynSolValue>,
}

#[cfg(test)]
mod test {
    use super::DynSolError;
    use crate::DynSolValue;
    use alloy_primitives::hex;

    #[test]
    fn decode_revert_message() {
        let error = DynSolError::revert();
        let data = hex!("08c379a0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000042020202000000000000000000000000000000000000000000000000000000000");

        let decoded = error.decode_error(&data).unwrap();
        assert_eq!(decoded.body, vec!(DynSolValue::String("    ".into())));
    }

    #[test]
    fn decode_panic() {
        let error = DynSolError::panic();
        let data = hex!("4e487b710000000000000000000000000000000000000000000000000000000000000001");

        let decoded = error.decode_error(&data).unwrap();
        assert_eq!(decoded.body, vec![DynSolValue::Uint(alloy_primitives::Uint::from(1), 256)]);
    }
}
