//! Secret key tests

#![cfg(feature = "dev")]

use elliptic_curve::dev::SecretKey;

#[test]
fn from_empty_slice() {
    assert!(SecretKey::from_slice(&[]).is_err());
}

#[test]
fn from_slice_expected_size() {
    let bytes = [1u8; 32];
    assert!(SecretKey::from_slice(&bytes).is_ok());
}

#[test]
fn from_slice_allowed_short() {
    let bytes = [1u8; 24];
    assert!(SecretKey::from_slice(&bytes).is_ok());
}

#[test]
fn from_slice_too_short() {
    let bytes = [1u8; 23]; // min 24-bytes
    assert!(SecretKey::from_slice(&bytes).is_err());
}
