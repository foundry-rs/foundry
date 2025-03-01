//! Utilities for working with EIP-4844 field elements and implementing
//! [`SidecarCoder`].
//!
//! [`SidecarCoder`]: crate::eip4844::builder::SidecarCoder

use crate::eip4844::{FIELD_ELEMENT_BYTES_USIZE, USABLE_BITS_PER_FIELD_ELEMENT};

/// Determine whether a slice of bytes can be contained in a field element.
pub const fn fits_in_fe(data: &[u8]) -> bool {
    const FIELD_ELEMENT_BYTES_USIZE_PLUS_ONE: usize = FIELD_ELEMENT_BYTES_USIZE + 1;

    match data.len() {
        FIELD_ELEMENT_BYTES_USIZE_PLUS_ONE.. => false,
        FIELD_ELEMENT_BYTES_USIZE => data[0] & 0b1100_0000 == 0, // first two bits must be zero
        _ => true,
    }
}

/// Calculate the number of field elements required to store the given
/// number of bytes.
pub const fn minimum_fe_for_bytes(bytes: usize) -> usize {
    (bytes * 8).div_ceil(USABLE_BITS_PER_FIELD_ELEMENT)
}

/// Calculate the number of field elements required to store the given data.
pub const fn minimum_fe(data: &[u8]) -> usize {
    minimum_fe_for_bytes(data.len())
}

/// A wrapper for a slice of bytes that is a whole, valid field element.
#[derive(Clone, Copy, Debug)]
pub struct WholeFe<'a>(&'a [u8]);

impl<'a> WholeFe<'a> {
    pub(crate) const fn new_unchecked(data: &'a [u8]) -> Self {
        Self(data)
    }

    /// Instantiate a new `WholeFe` from a slice of bytes, if it is a valid
    /// field element.
    pub const fn new(data: &'a [u8]) -> Option<Self> {
        if data.len() == FIELD_ELEMENT_BYTES_USIZE && fits_in_fe(data) {
            Some(Self::new_unchecked(data))
        } else {
            None
        }
    }
}

impl AsRef<[u8]> for WholeFe<'_> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

#[cfg(test)]
mod test {
    use crate::eip4844::{FIELD_ELEMENTS_PER_BLOB, USABLE_BYTES_PER_BLOB};

    use super::*;
    #[test]
    fn calc_required_fe() {
        assert_eq!(minimum_fe(&[0u8; 32]), 2);
        assert_eq!(minimum_fe(&[0u8; 31]), 1);
        assert_eq!(minimum_fe(&[0u8; 33]), 2);
        assert_eq!(minimum_fe(&[0u8; 64]), 3);
        assert_eq!(minimum_fe(&[0u8; 65]), 3);
        assert_eq!(minimum_fe_for_bytes(USABLE_BYTES_PER_BLOB), FIELD_ELEMENTS_PER_BLOB as usize);
    }

    #[test]
    fn calc_is_valid_field_element() {
        assert!(fits_in_fe(&[0u8; 32]));
        assert!(!fits_in_fe(&[0u8; 33]));

        assert!(WholeFe::new(&[0u8; 32]).is_some());
        assert!(WholeFe::new(&[0u8; 33]).is_none());
        assert!(WholeFe::new(&[0u8; 31]).is_none());
    }
}
