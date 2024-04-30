use alloy_primitives::Parity;

/// See <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md>
/// > If you do, then the v of the signature MUST be set to {0,1} + CHAIN_ID * 2 + 35 where
/// > {0,1} is the parity of the y value of the curve point for which r is the x-value in the
/// > secp256k1 signing process.
pub fn meets_eip155(chain_id: u64, v: Parity) -> bool {
    let double_chain_id = chain_id.saturating_mul(2);
    match v {
        Parity::Eip155(v) => v == double_chain_id + 35 || v == double_chain_id + 36,
        _ => false,
    }
}
