use crate::{Address, FixedBytes, Selector};
use core::borrow::Borrow;

wrap_fixed_bytes! {
    /// An Ethereum ABI function pointer, 24 bytes in length.
    ///
    /// An address (20 bytes), followed by a function selector (4 bytes).
    /// Encoded identical to `bytes24`.
    pub struct Function<24>;
}

impl<A, S> From<(A, S)> for Function
where
    A: Borrow<[u8; 20]>,
    S: Borrow<[u8; 4]>,
{
    #[inline]
    fn from((address, selector): (A, S)) -> Self {
        Self::from_address_and_selector(address, selector)
    }
}

impl Function {
    /// Creates an Ethereum function from an EVM word's lower 24 bytes
    /// (`word[..24]`).
    ///
    /// Note that this is different from `Address::from_word`, which uses the
    /// upper 20 bytes.
    #[inline]
    #[must_use]
    pub fn from_word(word: FixedBytes<32>) -> Self {
        Self(FixedBytes(word[..24].try_into().unwrap()))
    }

    /// Right-pads the function to 32 bytes (EVM word size).
    ///
    /// Note that this is different from `Address::into_word`, which left-pads
    /// the address.
    #[inline]
    #[must_use]
    pub fn into_word(&self) -> FixedBytes<32> {
        let mut word = [0; 32];
        word[..24].copy_from_slice(self.as_slice());
        FixedBytes(word)
    }

    /// Creates an Ethereum function from an address and selector.
    #[inline]
    pub fn from_address_and_selector<A, S>(address: A, selector: S) -> Self
    where
        A: Borrow<[u8; 20]>,
        S: Borrow<[u8; 4]>,
    {
        let mut bytes = [0; 24];
        bytes[..20].copy_from_slice(address.borrow());
        bytes[20..].copy_from_slice(selector.borrow());
        Self(FixedBytes(bytes))
    }

    /// Returns references to the address and selector of the function.
    #[inline]
    pub fn as_address_and_selector(&self) -> (&Address, &Selector) {
        // SAFETY: Function (24) = Address (20) + Selector (4)
        unsafe { (&*self.as_ptr().cast(), &*self.as_ptr().add(20).cast()) }
    }

    /// Returns the address and selector of the function.
    #[inline]
    pub fn to_address_and_selector(&self) -> (Address, Selector) {
        let (a, s) = self.as_address_and_selector();
        (*a, *s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex;

    #[test]
    fn function_parts() {
        let f = Function::new(hex!(
            "
            ffffffffffffffffffffffffffffffffffffffff
            12345678
        "
        ));

        let (a1, s1) = f.as_address_and_selector();
        assert_eq!(a1, hex!("ffffffffffffffffffffffffffffffffffffffff"));
        assert_eq!(s1, &hex!("12345678"));

        let (a2, s2) = f.to_address_and_selector();
        assert_eq!(a2, *a1);
        assert_eq!(s2, *s1);
    }
}
