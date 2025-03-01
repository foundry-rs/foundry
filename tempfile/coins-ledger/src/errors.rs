use thiserror::Error;

use crate::common::APDUResponseCodes;

/// APDU-related errors
#[derive(Debug, Error)]
pub enum LedgerError {
    /// APDU Response was too short
    #[error("Response too short. Expected at least 2 bytes. Got {0:?}")]
    ResponseTooShort(Vec<u8>),

    /// APDU error
    #[error("Ledger device: APDU Response error `{0}`")]
    BadRetcode(APDUResponseCodes),

    /// Ledger returned an unknown APDU
    #[error("Ledger returned an unknown response status code {0:x}. This is a bug. Please file an issue at https://github.com/summa-tx/coins-rs/issues")]
    UnknownAPDUCode(u16),

    /// The backend has been disconnected.
    #[error("The backend has been disconnected.")]
    BackendGone,

    /// JsValue Error
    #[error("JsValue Error: {0}")]
    #[cfg(target_arch = "wasm32")]
    JsError(String),

    /// Native transport error type.
    #[error(transparent)]
    #[cfg(not(target_arch = "wasm32"))]
    NativeTransportError(#[from] crate::transports::native::NativeTransportError),
}

#[cfg(target_arch = "wasm32")]
impl From<wasm_bindgen::prelude::JsValue> for LedgerError {
    fn from(r: wasm_bindgen::prelude::JsValue) -> Self {
        LedgerError::JsError(format!("{:#?}", &r))
    }
}

impl From<APDUResponseCodes> for LedgerError {
    fn from(r: APDUResponseCodes) -> Self {
        LedgerError::BadRetcode(r)
    }
}
