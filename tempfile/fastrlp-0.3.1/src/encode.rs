use crate::types::*;
use arrayvec::ArrayVec;
use auto_impl::auto_impl;
use bytes::{BufMut, Bytes, BytesMut};
use core::{any::Any, borrow::Borrow, mem::size_of};

pub fn zeroless_view(v: &impl AsRef<[u8]>) -> &[u8] {
    let v = v.as_ref();
    &v[v.iter().take_while(|&&b| b == 0).count()..]
}

impl Header {
    pub fn encode(&self, out: &mut dyn BufMut) {
        if self.payload_length < 56 {
            let code = if self.list {
                EMPTY_LIST_CODE
            } else {
                EMPTY_STRING_CODE
            };
            out.put_u8(code + self.payload_length as u8);
        } else {
            let len_be = self.payload_length.to_be_bytes();
            let len_be = zeroless_view(&len_be);
            let code = if self.list { 0xF7 } else { 0xB7 };
            out.put_u8(code + len_be.len() as u8);
            out.put_slice(len_be);
        }
    }
}

pub const fn length_of_length(payload_length: usize) -> usize {
    if payload_length < 56 {
        1
    } else {
        1 + size_of::<usize>() - payload_length.leading_zeros() as usize / 8
    }
}

#[doc(hidden)]
pub const fn const_add(a: usize, b: usize) -> usize {
    a + b
}

#[doc(hidden)]
pub unsafe trait MaxEncodedLen<const LEN: usize>: Encodable {}

#[doc(hidden)]
pub unsafe trait MaxEncodedLenAssoc: Encodable {
    const LEN: usize;
}

/// Use this to define length of an encoded entity
///
/// # Safety
/// Invalid value can cause the encoder to crash.
#[macro_export]
macro_rules! impl_max_encoded_len {
    ($t:ty, $len:block) => {
        unsafe impl MaxEncodedLen<{ $len }> for $t {}
        unsafe impl MaxEncodedLenAssoc for $t {
            const LEN: usize = $len;
        }
    };
}

#[auto_impl(&)]
#[cfg_attr(feature = "alloc", auto_impl(Box, Arc))]
pub trait Encodable {
    fn encode(&self, out: &mut dyn BufMut);
    fn length(&self) -> usize {
        let mut out = BytesMut::new();
        self.encode(&mut out);
        out.len()
    }
}

impl<'a> Encodable for &'a [u8] {
    fn length(&self) -> usize {
        let mut len = self.len();
        if self.len() != 1 || self[0] >= EMPTY_STRING_CODE {
            len += length_of_length(self.len());
        }
        len
    }

    fn encode(&self, out: &mut dyn BufMut) {
        if self.len() != 1 || self[0] >= EMPTY_STRING_CODE {
            Header {
                list: false,
                payload_length: self.len(),
            }
            .encode(out);
        }
        out.put_slice(self);
    }
}

impl<T, const LEN: usize> Encodable for [T; LEN]
where
    T: Encodable + 'static,
{
    fn length(&self) -> usize {
        if let Some(s) = <dyn Any>::downcast_ref::<[u8; LEN]>(self) {
            (s as &[u8]).length()
        } else {
            list_length(self)
        }
    }

    fn encode(&self, out: &mut dyn BufMut) {
        if let Some(s) = <dyn Any>::downcast_ref::<[u8; LEN]>(self) {
            (s as &[u8]).encode(out)
        } else {
            encode_list(self, out)
        }
    }
}

unsafe impl<const LEN: usize> MaxEncodedLenAssoc for [u8; LEN] {
    const LEN: usize = LEN + length_of_length(LEN);
}

macro_rules! encodable_uint {
    ($t:ty) => {
        #[allow(clippy::cmp_owned)]
        impl Encodable for $t {
            fn length(&self) -> usize {
                if *self < <$t>::from(EMPTY_STRING_CODE) {
                    1
                } else {
                    1 + (<$t>::BITS as usize / 8) - (self.leading_zeros() as usize / 8)
                }
            }

            fn encode(&self, out: &mut dyn BufMut) {
                if *self == 0 {
                    out.put_u8(EMPTY_STRING_CODE);
                } else if *self < <$t>::from(EMPTY_STRING_CODE) {
                    out.put_u8(u8::try_from(*self).unwrap());
                } else {
                    let be = self.to_be_bytes();
                    let be = zeroless_view(&be);
                    out.put_u8(EMPTY_STRING_CODE + be.len() as u8);
                    out.put_slice(be);
                }
            }
        }
    };
}

macro_rules! max_encoded_len_uint {
    ($t:ty) => {
        impl_max_encoded_len!($t, {
            length_of_length(<$t>::MAX.to_be_bytes().len()) + <$t>::MAX.to_be_bytes().len()
        });
    };
}

encodable_uint!(usize);
max_encoded_len_uint!(usize);

encodable_uint!(u8);
max_encoded_len_uint!(u8);

encodable_uint!(u16);
max_encoded_len_uint!(u16);

encodable_uint!(u32);
max_encoded_len_uint!(u32);

encodable_uint!(u64);
max_encoded_len_uint!(u64);

encodable_uint!(u128);
max_encoded_len_uint!(u128);

impl Encodable for bool {
    fn length(&self) -> usize {
        (*self as u8).length()
    }

    fn encode(&self, out: &mut dyn BufMut) {
        (*self as u8).encode(out)
    }
}

impl_max_encoded_len!(bool, { <u8 as MaxEncodedLenAssoc>::LEN });

#[cfg(feature = "ethnum")]
mod ethnum_support {
    use super::*;

    encodable_uint!(ethnum::U256);
    impl_max_encoded_len!(ethnum::U256, { length_of_length(32) + 32 });
}

#[cfg(feature = "ethereum-types")]
mod ethereum_types_support {
    use super::*;
    use ethereum_types::*;

    macro_rules! fixed_hash_impl {
        ($t:ty) => {
            impl Encodable for $t {
                fn length(&self) -> usize {
                    self.0.length()
                }

                fn encode(&self, out: &mut dyn bytes::BufMut) {
                    self.0.encode(out)
                }
            }
            impl_max_encoded_len!($t, {
                length_of_length(<$t>::len_bytes()) + <$t>::len_bytes()
            });
        };
    }

    fixed_hash_impl!(H64);
    fixed_hash_impl!(H128);
    fixed_hash_impl!(H160);
    fixed_hash_impl!(H256);
    fixed_hash_impl!(H512);
    fixed_hash_impl!(H520);
    #[cfg(feature = "ethbloom")]
    fixed_hash_impl!(Bloom);

    macro_rules! fixed_uint_impl {
        ($t:ty, $n_bytes:tt) => {
            impl Encodable for $t {
                fn length(&self) -> usize {
                    if *self < <$t>::from(EMPTY_STRING_CODE) {
                        1
                    } else {
                        1 + $n_bytes - (self.leading_zeros() as usize / 8)
                    }
                }

                fn encode(&self, out: &mut dyn bytes::BufMut) {
                    let mut temp_arr = [0u8; $n_bytes];
                    self.to_big_endian(&mut temp_arr[..]);
                    // cut the leading zeros after converting to big endian
                    let sliced = &temp_arr[(self.leading_zeros() / 8) as usize..];
                    sliced.encode(out);
                }
            }
        };
    }

    fixed_uint_impl!(U64, 8);
    fixed_uint_impl!(U128, 16);
    fixed_uint_impl!(U256, 32);
    fixed_uint_impl!(U512, 64);
}

macro_rules! slice_impl {
    ($t:ty) => {
        impl $crate::Encodable for $t {
            fn length(&self) -> usize {
                (&self[..]).length()
            }

            fn encode(&self, out: &mut dyn bytes::BufMut) {
                (&self[..]).encode(out)
            }
        }
    };
}

#[cfg(feature = "alloc")]
mod alloc_support {
    use super::*;

    extern crate alloc;

    impl<T> Encodable for ::alloc::vec::Vec<T>
    where
        T: Encodable + 'static,
    {
        fn length(&self) -> usize {
            if let Some(s) = <dyn Any>::downcast_ref::<::alloc::vec::Vec<u8>>(self) {
                (s as &[u8]).length()
            } else {
                list_length(self)
            }
        }

        fn encode(&self, out: &mut dyn BufMut) {
            if let Some(s) = <dyn Any>::downcast_ref::<::alloc::vec::Vec<u8>>(self) {
                (s as &[u8]).encode(out)
            } else {
                encode_list(self, out)
            }
        }
    }

    impl Encodable for ::alloc::string::String {
        fn encode(&self, out: &mut dyn BufMut) {
            self.as_bytes().encode(out);
        }
        fn length(&self) -> usize {
            self.as_bytes().length()
        }
    }
}

impl<T, const LEN: usize> Encodable for ArrayVec<T, LEN>
where
    T: Encodable + 'static,
{
    fn length(&self) -> usize {
        if let Some(s) = <dyn Any>::downcast_ref::<ArrayVec<u8, LEN>>(self) {
            (s as &[u8]).length()
        } else {
            list_length(self)
        }
    }

    fn encode(&self, out: &mut dyn BufMut) {
        if let Some(s) = <dyn Any>::downcast_ref::<ArrayVec<u8, LEN>>(self) {
            (s as &[u8]).encode(out)
        } else {
            encode_list(self, out)
        }
    }
}
slice_impl!(Bytes);
slice_impl!(BytesMut);

fn rlp_list_header<E, K>(v: &[K]) -> Header
where
    E: Encodable + ?Sized,
    K: Borrow<E>,
{
    let mut h = Header {
        list: true,
        payload_length: 0,
    };
    for x in v {
        h.payload_length += x.borrow().length();
    }
    h
}

pub fn list_length<E, K>(v: &[K]) -> usize
where
    E: Encodable,
    K: Borrow<E>,
{
    let payload_length = rlp_list_header(v).payload_length;
    length_of_length(payload_length) + payload_length
}

pub fn encode_list<E, K>(v: &[K], out: &mut dyn BufMut)
where
    E: Encodable + ?Sized,
    K: Borrow<E>,
{
    let h = rlp_list_header(v);
    h.encode(out);
    for x in v {
        x.borrow().encode(out);
    }
}

pub fn encode_fixed_size<E: MaxEncodedLen<LEN>, const LEN: usize>(v: &E) -> ArrayVec<u8, LEN> {
    let mut out = ArrayVec::from([0_u8; LEN]);

    let mut s = out.as_mut_slice();

    v.encode(&mut s);

    let final_len = LEN - s.len();
    out.truncate(final_len);

    out
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;
    use alloc::vec;
    use bytes::BytesMut;
    use hex_literal::hex;

    fn encoded<T: Encodable>(t: T) -> BytesMut {
        let mut out = BytesMut::new();
        t.encode(&mut out);
        out
    }

    fn encoded_list<T: Encodable + Clone + 'static>(t: &[T]) -> BytesMut {
        let mut out1 = BytesMut::new();
        encode_list(t, &mut out1);

        let v = t.to_vec();
        assert_eq!(out1.len(), v.length());

        let mut out2 = BytesMut::new();
        v.encode(&mut out2);
        assert_eq!(out1, out2);

        out1
    }

    #[test]
    fn rlp_strings() {
        assert_eq!(encoded(hex!(""))[..], hex!("80")[..]);
        assert_eq!(encoded(hex!("7B"))[..], hex!("7b")[..]);
        assert_eq!(encoded(hex!("80"))[..], hex!("8180")[..]);
        assert_eq!(encoded(hex!("ABBA"))[..], hex!("82abba")[..]);
    }

    fn u8_fixtures() -> impl IntoIterator<Item = (u8, &'static [u8])> {
        vec![
            (0, &hex!("80")[..]),
            (1, &hex!("01")[..]),
            (0x7F, &hex!("7F")[..]),
            (0x80, &hex!("8180")[..]),
        ]
    }

    fn c<T, U: From<T>>(
        it: impl IntoIterator<Item = (T, &'static [u8])>,
    ) -> impl Iterator<Item = (U, &'static [u8])> {
        it.into_iter().map(|(k, v)| (k.into(), v))
    }

    fn u16_fixtures() -> impl IntoIterator<Item = (u16, &'static [u8])> {
        c(u8_fixtures()).chain(vec![(0x400, &hex!("820400")[..])])
    }

    fn u32_fixtures() -> impl IntoIterator<Item = (u32, &'static [u8])> {
        c(u16_fixtures()).chain(vec![
            (0xFFCCB5, &hex!("83ffccb5")[..]),
            (0xFFCCB5DD, &hex!("84ffccb5dd")[..]),
        ])
    }

    fn u64_fixtures() -> impl IntoIterator<Item = (u64, &'static [u8])> {
        c(u32_fixtures()).chain(vec![
            (0xFFCCB5DDFF, &hex!("85ffccb5ddff")[..]),
            (0xFFCCB5DDFFEE, &hex!("86ffccb5ddffee")[..]),
            (0xFFCCB5DDFFEE14, &hex!("87ffccb5ddffee14")[..]),
            (0xFFCCB5DDFFEE1483, &hex!("88ffccb5ddffee1483")[..]),
        ])
    }

    fn u128_fixtures() -> impl IntoIterator<Item = (u128, &'static [u8])> {
        c(u64_fixtures()).chain(vec![(
            0x10203E405060708090A0B0C0D0E0F2,
            &hex!("8f10203e405060708090a0b0c0d0e0f2")[..],
        )])
    }

    #[cfg(feature = "ethnum")]
    fn u256_fixtures() -> impl IntoIterator<Item = (ethnum::U256, &'static [u8])> {
        c(u128_fixtures()).chain(vec![(
            ethnum::U256::from_str_radix(
                "0100020003000400050006000700080009000A0B4B000C000D000E01",
                16,
            )
            .unwrap(),
            &hex!("9c0100020003000400050006000700080009000a0b4b000c000d000e01")[..],
        )])
    }

    #[cfg(feature = "ethereum-types")]
    fn eth_u64_fixtures() -> impl IntoIterator<Item = (ethereum_types::U64, &'static [u8])> {
        c(u64_fixtures()).chain(vec![
            (
                ethereum_types::U64::from_str_radix("FFCCB5DDFF", 16).unwrap(),
                &hex!("85ffccb5ddff")[..],
            ),
            (
                ethereum_types::U64::from_str_radix("FFCCB5DDFFEE", 16).unwrap(),
                &hex!("86ffccb5ddffee")[..],
            ),
            (
                ethereum_types::U64::from_str_radix("FFCCB5DDFFEE14", 16).unwrap(),
                &hex!("87ffccb5ddffee14")[..],
            ),
            (
                ethereum_types::U64::from_str_radix("FFCCB5DDFFEE1483", 16).unwrap(),
                &hex!("88ffccb5ddffee1483")[..],
            ),
        ])
    }

    #[cfg(feature = "ethereum-types")]
    fn eth_u128_fixtures() -> impl IntoIterator<Item = (ethereum_types::U128, &'static [u8])> {
        c(u128_fixtures()).chain(vec![(
            ethereum_types::U128::from_str_radix("10203E405060708090A0B0C0D0E0F2", 16).unwrap(),
            &hex!("8f10203e405060708090a0b0c0d0e0f2")[..],
        )])
    }

    #[cfg(feature = "ethereum-types")]
    fn eth_u256_fixtures() -> impl IntoIterator<Item = (ethereum_types::U256, &'static [u8])> {
        c(u128_fixtures()).chain(vec![(
            ethereum_types::U256::from_str_radix(
                "0100020003000400050006000700080009000A0B4B000C000D000E01",
                16,
            )
            .unwrap(),
            &hex!("9c0100020003000400050006000700080009000a0b4b000c000d000e01")[..],
        )])
    }

    #[cfg(feature = "ethereum-types")]
    fn eth_u512_fixtures() -> impl IntoIterator<Item = (ethereum_types::U512, &'static [u8])> {
        c(eth_u256_fixtures()).chain(vec![(
            ethereum_types::U512::from_str_radix(
                "0100020003000400050006000700080009000A0B4B000C000D000E010100020003000400050006000700080009000A0B4B000C000D000E01",
                16,
            )
            .unwrap(),
            &hex!("b8380100020003000400050006000700080009000A0B4B000C000D000E010100020003000400050006000700080009000A0B4B000C000D000E01")[..],
        )])
    }

    macro_rules! uint_rlp_test {
        ($fixtures:expr) => {
            for (input, output) in $fixtures {
                assert_eq!(encoded(input), output);
            }
        };
    }

    #[test]
    fn rlp_uints() {
        uint_rlp_test!(u8_fixtures());
        uint_rlp_test!(u16_fixtures());
        uint_rlp_test!(u32_fixtures());
        uint_rlp_test!(u64_fixtures());
        uint_rlp_test!(u128_fixtures());
        #[cfg(feature = "ethnum")]
        uint_rlp_test!(u256_fixtures());
    }

    #[cfg(feature = "ethereum-types")]
    #[test]
    fn rlp_eth_uints() {
        uint_rlp_test!(eth_u64_fixtures());
        uint_rlp_test!(eth_u128_fixtures());
        uint_rlp_test!(eth_u256_fixtures());
        uint_rlp_test!(eth_u512_fixtures());
    }

    #[test]
    fn rlp_list() {
        assert_eq!(encoded_list::<u64>(&[]), &hex!("c0")[..]);
        assert_eq!(
            encoded_list(&[0xFFCCB5_u64, 0xFFC0B5_u64]),
            &hex!("c883ffccb583ffc0b5")[..]
        );
    }

    #[test]
    fn vec_specialization() {
        const SPECIALIZED: [u8; 2] = [0x42_u8, 0x43_u8];
        const GENERAL: [u64; 2] = [0xFFCCB5_u64, 0xFFC0B5_u64];

        const SPECIALIZED_EXP: &[u8] = &hex!("824243");
        const GENERAL_EXP: &[u8] = &hex!("C883FFCCB583FFC0B5");

        assert_eq!(&*encoded(SPECIALIZED), SPECIALIZED_EXP);
        assert_eq!(&*encoded(GENERAL), GENERAL_EXP);

        assert_eq!(&*encoded(ArrayVec::from(SPECIALIZED)), SPECIALIZED_EXP);
        assert_eq!(&*encoded(ArrayVec::from(GENERAL)), GENERAL_EXP);

        #[cfg(feature = "alloc")]
        {
            assert_eq!(&*encoded(SPECIALIZED.to_vec()), SPECIALIZED_EXP);
            assert_eq!(&*encoded(GENERAL.to_vec()), GENERAL_EXP);
        }
    }
}
