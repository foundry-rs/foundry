#![cfg_attr(not(feature = "std"), no_std)]
// All `as` conversions in this code base have been carefully reviewed
// and are safe.
#![allow(
    clippy::as_conversions,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::many_single_char_names
)]

mod arith;
mod maybe_std;
mod mpnat;

use maybe_std::Vec;

/// Trait providing the interface for the modexp function.
/// The implementation provided by this crate is `AuroraModExp` below,
/// but other users of Aurora Engine may wish to select a different implementation.
pub trait ModExpAlgorithm: 'static {
    /// Computes `(base ^ exp) % modulus`, where all values are given as big-endian encoded bytes.
    fn modexp(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8>;
}

pub struct AuroraModExp;

impl ModExpAlgorithm for AuroraModExp {
    fn modexp(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8> {
        modexp(base, exp, modulus)
    }
}

/// Computes `(base ^ exp) % modulus`, where all values are given as big-endian
/// encoded bytes.
#[must_use]
pub fn modexp(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8> {
    let mut x = mpnat::MPNat::from_big_endian(base);
    let m = mpnat::MPNat::from_big_endian(modulus);
    if m.digits.len() == 1 && m.digits[0] == 0 {
        return Vec::new();
    }
    let result = x.modpow(exp, &m);
    result.to_big_endian()
}

#[cfg(feature = "bench")]
pub fn modexp_ibig(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8> {
    use num::Zero;

    let base = ibig::UBig::from_be_bytes(base);
    let modulus = ibig::UBig::from_be_bytes(modulus);
    if modulus.is_zero() {
        return Vec::new();
    }
    let exponent = ibig::UBig::from_be_bytes(exp);
    let ring = ibig::modular::ModuloRing::new(&modulus);
    let result = ring.from(base).pow(&exponent);
    result.residue().to_be_bytes()
}

#[cfg(feature = "bench")]
pub fn modexp_num(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8> {
    use num::Zero;

    let base = num::BigUint::from_bytes_be(base);
    let modulus = num::BigUint::from_bytes_be(modulus);
    if modulus.is_zero() {
        return Vec::new();
    }
    let exponent = num::BigUint::from_bytes_be(exp);
    base.modpow(&exponent, &modulus).to_bytes_be()
}
