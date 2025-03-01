use crate::{DynSolError, Specifier};
use alloc::vec::Vec;
use alloy_json_abi::Error;
use alloy_primitives::{keccak256, Selector};

#[allow(unknown_lints, unnameable_types)]
mod sealed {
    pub trait Sealed {}
    impl Sealed for alloy_json_abi::Error {}
}
use sealed::Sealed;

impl Specifier<DynSolError> for Error {
    fn resolve(&self) -> crate::Result<DynSolError> {
        let signature = self.signature();
        let selector = Selector::from_slice(&keccak256(signature)[0..4]);

        let mut body = Vec::with_capacity(self.inputs.len());
        for param in &self.inputs {
            body.push(param.resolve()?);
        }

        Ok(DynSolError::new_unchecked(selector, crate::DynSolType::Tuple(body)))
    }
}

/// Provides error encoding and decoding for the [`Error`] type.
pub trait ErrorExt: Sealed {
    /// Decode the error from the given data.
    fn decode_error(&self, data: &[u8]) -> crate::Result<crate::DecodedError>;
}

impl ErrorExt for alloy_json_abi::Error {
    fn decode_error(&self, data: &[u8]) -> crate::Result<crate::DecodedError> {
        self.resolve()?.decode_error(data)
    }
}
