//! Private-key validation shared by native broadcasting and crypto cheatcodes.

use crate::Result;
use alloy_primitives::U256;
use alloy_signer_local::PrivateKeySigner;
use k256::{ecdsa::SigningKey, elliptic_curve::bigint::ArrayEncoding};

pub(crate) fn validate_private_key<C: ecdsa::PrimeCurve>(private_key: &U256) -> Result<()> {
    ensure!(!private_key.is_zero(), "private key cannot be 0");
    let order = U256::from_be_slice(&C::ORDER.to_be_byte_array());
    ensure!(
        *private_key < order,
        "private key must be less than the {curve:?} curve order ({order})",
        curve = C::default(),
    );

    Ok(())
}

pub(crate) fn parse_wallet(private_key: &U256) -> Result<PrivateKeySigner> {
    validate_private_key::<k256::Secp256k1>(private_key)?;
    let key = SigningKey::from_bytes((&private_key.to_be_bytes()).into())?;
    Ok(PrivateKeySigner::from(key))
}
