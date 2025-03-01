mod error;
pub use error::SignatureError;

#[allow(deprecated)]
mod parity;
#[allow(deprecated)]
pub use parity::Parity;

#[allow(deprecated)]
mod sig;
#[allow(deprecated)]
pub use sig::Signature;

mod utils;
pub use utils::{normalize_v, to_eip155_v};

mod primitive_sig;
pub use primitive_sig::PrimitiveSignature;
