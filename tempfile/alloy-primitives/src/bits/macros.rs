/// Wrap a fixed-size byte array in a newtype, delegating all methods to the
/// underlying [`crate::FixedBytes`].
///
/// This functionally creates a new named `FixedBytes` that cannot be
/// type-confused for another named `FixedBytes`.
///
/// # Examples
///
/// ```
/// use alloy_primitives::wrap_fixed_bytes;
///
/// // These hashes are the same length, and have the same functionality, but
/// // are distinct types
/// wrap_fixed_bytes!(pub struct KeccakOutput<32>;);
/// wrap_fixed_bytes!(pub struct MerkleTreeItem<32>;);
/// ```
#[macro_export]
macro_rules! wrap_fixed_bytes {
    (
        $(#[$attrs:meta])*
        $vis:vis struct $name:ident<$n:literal>;
    ) => {
        $crate::wrap_fixed_bytes!(
            extra_derives: [$crate::private::derive_more::Display],
            $(#[$attrs])*
            $vis struct $name<$n>;
        );
    };

    (
        extra_derives: [$($extra_derives:path),* $(,)?],
        $(#[$attrs:meta])*
        $vis:vis struct $name:ident<$n:literal>;
    ) => {
        $(#[$attrs])*
        #[derive(
            Clone,
            Copy,
            Default,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            $crate::private::derive_more::AsMut,
            $crate::private::derive_more::AsRef,
            $crate::private::derive_more::BitAnd,
            $crate::private::derive_more::BitAndAssign,
            $crate::private::derive_more::BitOr,
            $crate::private::derive_more::BitOrAssign,
            $crate::private::derive_more::BitXor,
            $crate::private::derive_more::BitXorAssign,
            $crate::private::derive_more::Not,
            $crate::private::derive_more::Deref,
            $crate::private::derive_more::DerefMut,
            $crate::private::derive_more::From,
            $crate::private::derive_more::FromStr,
            $crate::private::derive_more::Index,
            $crate::private::derive_more::IndexMut,
            $crate::private::derive_more::Into,
            $crate::private::derive_more::IntoIterator,
            $crate::private::derive_more::LowerHex,
            $crate::private::derive_more::UpperHex,
            $(
                $extra_derives,
            )*
        )]
        #[repr(transparent)]
        $vis struct $name(#[into_iterator(owned, ref, ref_mut)] pub $crate::FixedBytes<$n>);

        impl $crate::private::From<[u8; $n]> for $name {
            #[inline]
            fn from(value: [u8; $n]) -> Self {
                Self($crate::FixedBytes(value))
            }
        }

        impl $crate::private::From<$name> for [u8; $n] {
            #[inline]
            fn from(value: $name) -> Self {
                value.0 .0
            }
        }

        impl<'a> $crate::private::From<&'a [u8; $n]> for $name {
            #[inline]
            fn from(value: &'a [u8; $n]) -> Self {
                Self($crate::FixedBytes(*value))
            }
        }

        impl<'a> $crate::private::From<&'a mut [u8; $n]> for $name {
            #[inline]
            fn from(value: &'a mut [u8; $n]) -> Self {
                Self($crate::FixedBytes(*value))
            }
        }

        impl $crate::private::TryFrom<&[u8]> for $name {
            type Error = $crate::private::core::array::TryFromSliceError;

            #[inline]
            fn try_from(slice: &[u8]) -> Result<Self, Self::Error> {
                <&Self as $crate::private::TryFrom<&[u8]>>::try_from(slice).copied()
            }
        }

        impl $crate::private::TryFrom<&mut [u8]> for $name {
            type Error = $crate::private::core::array::TryFromSliceError;

            #[inline]
            fn try_from(slice: &mut [u8]) -> Result<Self, Self::Error> {
                <Self as $crate::private::TryFrom<&[u8]>>::try_from(&*slice)
            }
        }

        impl<'a> $crate::private::TryFrom<&'a [u8]> for &'a $name {
            type Error = $crate::private::core::array::TryFromSliceError;

            #[inline]
            #[allow(unsafe_code)]
            fn try_from(slice: &'a [u8]) -> Result<&'a $name, Self::Error> {
                // SAFETY: `$name` is `repr(transparent)` for `FixedBytes<$n>`
                // and consequently `[u8; $n]`
                <&[u8; $n] as $crate::private::TryFrom<&[u8]>>::try_from(slice)
                    .map(|array_ref| unsafe { $crate::private::core::mem::transmute(array_ref) })
            }
        }

        impl<'a> $crate::private::TryFrom<&'a mut [u8]> for &'a mut $name {
            type Error = $crate::private::core::array::TryFromSliceError;

            #[inline]
            #[allow(unsafe_code)]
            fn try_from(slice: &'a mut [u8]) -> Result<&'a mut $name, Self::Error> {
                // SAFETY: `$name` is `repr(transparent)` for `FixedBytes<$n>`
                // and consequently `[u8; $n]`
                <&mut [u8; $n] as $crate::private::TryFrom<&mut [u8]>>::try_from(slice)
                    .map(|array_ref| unsafe { $crate::private::core::mem::transmute(array_ref) })
            }
        }

        impl $crate::private::AsRef<[u8; $n]> for $name {
            #[inline]
            fn as_ref(&self) -> &[u8; $n] {
                &self.0 .0
            }
        }

        impl $crate::private::AsMut<[u8; $n]> for $name {
            #[inline]
            fn as_mut(&mut self) -> &mut [u8; $n] {
                &mut self.0 .0
            }
        }

        impl $crate::private::AsRef<[u8]> for $name {
            #[inline]
            fn as_ref(&self) -> &[u8] {
                &self.0 .0
            }
        }

        impl $crate::private::AsMut<[u8]> for $name {
            #[inline]
            fn as_mut(&mut self) -> &mut [u8] {
                &mut self.0 .0
            }
        }

        impl $crate::private::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                $crate::private::core::fmt::Debug::fmt(&self.0, f)
            }
        }

        $crate::impl_fb_traits!($name, $n);
        $crate::impl_rlp!($name, $n);
        $crate::impl_serde!($name);
        $crate::impl_allocative!($name);
        $crate::impl_arbitrary!($name, $n);
        $crate::impl_rand!($name);

        impl $name {
            /// Array of Zero bytes.
            pub const ZERO: Self = Self($crate::FixedBytes::ZERO);

            /// Wraps the given byte array in this type.
            #[inline]
            pub const fn new(bytes: [u8; $n]) -> Self {
                Self($crate::FixedBytes(bytes))
            }

            /// Creates a new byte array with the last byte set to `x`.
            #[inline]
            pub const fn with_last_byte(x: u8) -> Self {
                Self($crate::FixedBytes::with_last_byte(x))
            }

            /// Creates a new byte array where all bytes are set to `byte`.
            #[inline]
            pub const fn repeat_byte(byte: u8) -> Self {
                Self($crate::FixedBytes::repeat_byte(byte))
            }

            /// Returns the size of this array in bytes.
            #[inline]
            pub const fn len_bytes() -> usize {
                $n
            }

            $crate::impl_getrandom!();
            $crate::impl_rand!();

            /// Create a new byte array from the given slice `src`.
            ///
            /// For a fallible version, use the `TryFrom<&[u8]>` implementation.
            ///
            /// # Note
            ///
            /// The given bytes are interpreted in big endian order.
            ///
            /// # Panics
            ///
            /// If the length of `src` and the number of bytes in `Self` do not match.
            #[inline]
            #[track_caller]
            pub fn from_slice(src: &[u8]) -> Self {
                match Self::try_from(src) {
                    Ok(x) => x,
                    Err(_) => panic!("cannot convert a slice of length {} to {}", src.len(), stringify!($name)),
                }
            }

            /// Create a new byte array from the given slice `src`, left-padding it
            /// with zeroes if necessary.
            ///
            /// # Note
            ///
            /// The given bytes are interpreted in big endian order.
            ///
            /// # Panics
            ///
            /// Panics if `src.len() > N`.
            #[inline]
            #[track_caller]
            pub fn left_padding_from(value: &[u8]) -> Self {
                Self($crate::FixedBytes::left_padding_from(value))
            }

            /// Create a new byte array from the given slice `src`, right-padding it
            /// with zeroes if necessary.
            ///
            /// # Note
            ///
            /// The given bytes are interpreted in big endian order.
            ///
            /// # Panics
            ///
            /// Panics if `src.len() > N`.
            #[inline]
            #[track_caller]
            pub fn right_padding_from(value: &[u8]) -> Self {
                Self($crate::FixedBytes::right_padding_from(value))
            }

            /// Returns the inner bytes array.
            #[inline]
            pub const fn into_array(self) -> [u8; $n] {
                self.0 .0
            }

            /// Returns `true` if all bits set in `b` are also set in `self`.
            #[inline]
            pub fn covers(&self, b: &Self) -> bool {
                &(*b & *self) == b
            }

            /// Compile-time equality. NOT constant-time equality.
            pub const fn const_eq(&self, other: &Self) -> bool {
                self.0.const_eq(&other.0)
            }

            /// Computes the bitwise AND of two `FixedBytes`.
            pub const fn bit_and(self, rhs: Self) -> Self {
                Self(self.0.bit_and(rhs.0))
            }

            /// Computes the bitwise OR of two `FixedBytes`.
            pub const fn bit_or(self, rhs: Self) -> Self {
                Self(self.0.bit_or(rhs.0))
            }

            /// Computes the bitwise XOR of two `FixedBytes`.
            pub const fn bit_xor(self, rhs: Self) -> Self {
                Self(self.0.bit_xor(rhs.0))
            }
        }
    };
}

// Extra traits that cannot be derived automatically
#[doc(hidden)]
#[macro_export]
macro_rules! impl_fb_traits {
    (impl<$($const:ident)?> Borrow<$t:ty> for $b:ty) => {
        impl<$($const N: usize)?> $crate::private::Borrow<$t> for $b {
            #[inline]
            fn borrow(&self) -> &$t {
                $crate::private::Borrow::borrow(&self.0)
            }
        }
    };

    (impl<$($const:ident)?> BorrowMut<$t:ty> for $b:ty) => {
        impl<$($const N: usize)?> $crate::private::BorrowMut<$t> for $b {
            #[inline]
            fn borrow_mut(&mut self) -> &mut $t {
                $crate::private::BorrowMut::borrow_mut(&mut self.0)
            }
        }
    };

    (unsafe impl<$lt:lifetime, $($const:ident)?> From<$a:ty> for $b:ty) => {
        impl<$lt, $($const N: usize)?> $crate::private::From<$a> for $b {
            #[inline]
            #[allow(unsafe_code)]
            fn from(value: $a) -> $b {
                // SAFETY: guaranteed by caller
                unsafe { $crate::private::core::mem::transmute::<$a, $b>(value) }
            }
        }
    };

    (impl<$($const:ident)?> cmp::$tr:ident<$a:ty> for $b:ty where fn $fn:ident -> $ret:ty $(, [$e:expr])?) => {
        impl<$($const N: usize)?> $crate::private::$tr<$a> for $b {
            #[inline]
            fn $fn(&self, other: &$a) -> $ret {
                $crate::private::$tr::$fn(&self.0 $([$e])?, other)
            }
        }

        impl<$($const N: usize)?> $crate::private::$tr<$b> for $a {
            #[inline]
            fn $fn(&self, other: &$b) -> $ret {
                $crate::private::$tr::$fn(self, &other.0 $([$e])?)
            }
        }

        impl<$($const N: usize)?> $crate::private::$tr<&$a> for $b {
            #[inline]
            fn $fn(&self, other: &&$a) -> $ret {
                $crate::private::$tr::$fn(&self.0 $([$e])?, *other)
            }
        }

        impl<$($const N: usize)?> $crate::private::$tr<$b> for &$a {
            #[inline]
            fn $fn(&self, other: &$b) -> $ret {
                $crate::private::$tr::$fn(*self, &other.0 $([$e])?)
            }
        }

        impl<$($const N: usize)?> $crate::private::$tr<$a> for &$b {
            #[inline]
            fn $fn(&self, other: &$a) -> $ret {
                $crate::private::$tr::$fn(&self.0 $([$e])?, other)
            }
        }

        impl<$($const N: usize)?> $crate::private::$tr<&$b> for $a {
            #[inline]
            fn $fn(&self, other: &&$b) -> $ret {
                $crate::private::$tr::$fn(self, &other.0 $([$e])?)
            }
        }
    };

    ($t:ty, $n:tt $(, $const:ident)?) => {
        // Borrow is not automatically implemented for references
        $crate::impl_fb_traits!(impl<$($const)?> Borrow<[u8]>        for $t);
        $crate::impl_fb_traits!(impl<$($const)?> Borrow<[u8]>        for &$t);
        $crate::impl_fb_traits!(impl<$($const)?> Borrow<[u8]>        for &mut $t);
        $crate::impl_fb_traits!(impl<$($const)?> Borrow<[u8; $n]>    for $t);
        $crate::impl_fb_traits!(impl<$($const)?> Borrow<[u8; $n]>    for &$t);
        $crate::impl_fb_traits!(impl<$($const)?> Borrow<[u8; $n]>    for &mut $t);

        $crate::impl_fb_traits!(impl<$($const)?> BorrowMut<[u8]>     for $t);
        $crate::impl_fb_traits!(impl<$($const)?> BorrowMut<[u8]>     for &mut $t);
        $crate::impl_fb_traits!(impl<$($const)?> BorrowMut<[u8; $n]> for $t);
        $crate::impl_fb_traits!(impl<$($const)?> BorrowMut<[u8; $n]> for &mut $t);

        // Implement conversion traits for references with `mem::transmute`
        // SAFETY: `repr(transparent)`
        $crate::impl_fb_traits!(unsafe impl<'a, $($const)?> From<&'a [u8; $n]>     for &'a $t);
        $crate::impl_fb_traits!(unsafe impl<'a, $($const)?> From<&'a mut [u8; $n]> for &'a $t);
        $crate::impl_fb_traits!(unsafe impl<'a, $($const)?> From<&'a mut [u8; $n]> for &'a mut $t);

        $crate::impl_fb_traits!(unsafe impl<'a, $($const)?> From<&'a $t>           for &'a [u8; $n]);
        $crate::impl_fb_traits!(unsafe impl<'a, $($const)?> From<&'a mut $t>       for &'a [u8; $n]);
        $crate::impl_fb_traits!(unsafe impl<'a, $($const)?> From<&'a mut $t>       for &'a mut [u8; $n]);

        // Implement PartialEq, PartialOrd, with slice and array
        $crate::impl_fb_traits!(impl<$($const)?> cmp::PartialEq<[u8]> for $t where fn eq -> bool);
        $crate::impl_fb_traits!(impl<$($const)?> cmp::PartialEq<[u8; $n]> for $t where fn eq -> bool);
        $crate::impl_fb_traits!(
            impl<$($const)?> cmp::PartialOrd<[u8]> for $t
            where
                fn partial_cmp -> $crate::private::Option<$crate::private::Ordering>,
                [..] // slices $t
        );

        impl<$($const N: usize)?> $crate::hex::FromHex for $t {
            type Error = $crate::hex::FromHexError;

            #[inline]
            fn from_hex<T: $crate::private::AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> {
                $crate::hex::decode_to_array(hex).map(Self::new)
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
#[cfg(feature = "getrandom")]
macro_rules! impl_getrandom {
    () => {
        /// Instantiates a new fixed byte array with cryptographically random
        /// content.
        ///
        /// # Panics
        ///
        /// Panics if the underlying call to `getrandom_uninit`
        /// fails.
        #[inline]
        #[track_caller]
        #[cfg_attr(docsrs, doc(cfg(feature = "getrandom")))]
        pub fn random() -> Self {
            Self($crate::FixedBytes::random())
        }

        /// Tries to create a new fixed byte array with cryptographically random
        /// content.
        ///
        /// # Errors
        ///
        /// This function only propagates the error from the underlying call to
        /// `getrandom_uninit`.
        #[inline]
        #[cfg_attr(docsrs, doc(cfg(feature = "getrandom")))]
        pub fn try_random() -> $crate::private::Result<Self, $crate::private::getrandom::Error> {
            $crate::FixedBytes::try_random().map(Self)
        }

        /// Fills this fixed byte array with cryptographically random content.
        ///
        /// # Panics
        ///
        /// Panics if the underlying call to `getrandom_uninit` fails.
        #[inline]
        #[track_caller]
        #[cfg_attr(docsrs, doc(cfg(feature = "getrandom")))]
        pub fn randomize(&mut self) {
            self.try_randomize().unwrap()
        }

        /// Tries to fill this fixed byte array with cryptographically random content.
        ///
        /// # Errors
        ///
        /// This function only propagates the error from the underlying call to
        /// `getrandom_uninit`.
        #[inline]
        #[cfg_attr(docsrs, doc(cfg(feature = "getrandom")))]
        pub fn try_randomize(
            &mut self,
        ) -> $crate::private::Result<(), $crate::private::getrandom::Error> {
            self.0.try_randomize()
        }
    };
}

#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "getrandom"))]
macro_rules! impl_getrandom {
    () => {};
}

#[doc(hidden)]
#[macro_export]
#[cfg(feature = "rand")]
macro_rules! impl_rand {
    () => {
        /// Creates a new fixed byte array with the given random number generator.
        #[inline]
        #[doc(alias = "random_using")]
        #[cfg_attr(docsrs, doc(cfg(feature = "rand")))]
        pub fn random_with<R: $crate::private::rand::Rng + ?Sized>(rng: &mut R) -> Self {
            Self($crate::FixedBytes::random_with(rng))
        }

        /// Fills this fixed byte array with the given random number generator.
        #[inline]
        #[doc(alias = "randomize_using")]
        #[cfg_attr(docsrs, doc(cfg(feature = "rand")))]
        pub fn randomize_with<R: $crate::private::rand::Rng + ?Sized>(&mut self, rng: &mut R) {
            self.0.randomize_with(rng);
        }
    };

    ($t:ty) => {
        #[cfg_attr(docsrs, doc(cfg(feature = "rand")))]
        impl $crate::private::rand::distributions::Distribution<$t>
            for $crate::private::rand::distributions::Standard
        {
            #[inline]
            fn sample<R: $crate::private::rand::Rng + ?Sized>(&self, rng: &mut R) -> $t {
                <$t>::random_with(rng)
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "rand"))]
macro_rules! impl_rand {
    ($($t:tt)*) => {};
}

#[doc(hidden)]
#[macro_export]
#[cfg(feature = "rlp")]
macro_rules! impl_rlp {
    ($t:ty, $n:literal) => {
        #[cfg_attr(docsrs, doc(cfg(feature = "rlp")))]
        impl $crate::private::alloy_rlp::Decodable for $t {
            #[inline]
            fn decode(buf: &mut &[u8]) -> $crate::private::alloy_rlp::Result<Self> {
                $crate::private::alloy_rlp::Decodable::decode(buf).map(Self)
            }
        }

        #[cfg_attr(docsrs, doc(cfg(feature = "rlp")))]
        impl $crate::private::alloy_rlp::Encodable for $t {
            #[inline]
            fn length(&self) -> usize {
                $crate::private::alloy_rlp::Encodable::length(&self.0)
            }

            #[inline]
            fn encode(&self, out: &mut dyn bytes::BufMut) {
                $crate::private::alloy_rlp::Encodable::encode(&self.0, out)
            }
        }

        $crate::private::alloy_rlp::impl_max_encoded_len!($t, {
            $n + $crate::private::alloy_rlp::length_of_length($n)
        });
    };
}

#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "rlp"))]
macro_rules! impl_rlp {
    ($t:ty, $n:literal) => {};
}

#[doc(hidden)]
#[macro_export]
#[cfg(feature = "allocative")]
macro_rules! impl_allocative {
    ($t:ty) => {
        #[cfg_attr(docsrs, doc(cfg(feature = "allocative")))]
        impl $crate::private::allocative::Allocative for $t {
            #[inline]
            fn visit<'a, 'b: 'a>(&self, visitor: &'a mut $crate::private::allocative::Visitor<'b>) {
                $crate::private::allocative::Allocative::visit(&self.0, visitor)
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "allocative"))]
macro_rules! impl_allocative {
    ($t:ty) => {};
}

#[doc(hidden)]
#[macro_export]
#[cfg(feature = "serde")]
macro_rules! impl_serde {
    ($t:ty) => {
        #[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
        impl $crate::private::serde::Serialize for $t {
            #[inline]
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                $crate::private::serde::Serialize::serialize(&self.0, serializer)
            }
        }

        #[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
        impl<'de> $crate::private::serde::Deserialize<'de> for $t {
            #[inline]
            fn deserialize<D: $crate::private::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                $crate::private::serde::Deserialize::deserialize(deserializer).map(Self)
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "serde"))]
macro_rules! impl_serde {
    ($t:ty) => {};
}

#[doc(hidden)]
#[macro_export]
#[cfg(feature = "arbitrary")]
macro_rules! impl_arbitrary {
    ($t:ty, $n:literal) => {
        #[cfg_attr(docsrs, doc(cfg(feature = "arbitrary")))]
        impl<'a> $crate::private::arbitrary::Arbitrary<'a> for $t {
            #[inline]
            fn arbitrary(u: &mut $crate::private::arbitrary::Unstructured<'a>) -> $crate::private::arbitrary::Result<Self> {
                <$crate::FixedBytes<$n> as $crate::private::arbitrary::Arbitrary>::arbitrary(u).map(Self)
            }

            #[inline]
            fn arbitrary_take_rest(u: $crate::private::arbitrary::Unstructured<'a>) -> $crate::private::arbitrary::Result<Self> {
                <$crate::FixedBytes<$n> as $crate::private::arbitrary::Arbitrary>::arbitrary_take_rest(u).map(Self)
            }

            #[inline]
            fn size_hint(depth: usize) -> (usize, Option<usize>) {
                <$crate::FixedBytes<$n> as $crate::private::arbitrary::Arbitrary>::size_hint(depth)
            }
        }

        #[cfg_attr(docsrs, doc(cfg(feature = "arbitrary")))]
        impl $crate::private::proptest::arbitrary::Arbitrary for $t {
            type Parameters = <$crate::FixedBytes<$n> as $crate::private::proptest::arbitrary::Arbitrary>::Parameters;
            type Strategy = $crate::private::proptest::strategy::Map<
                <$crate::FixedBytes<$n> as $crate::private::proptest::arbitrary::Arbitrary>::Strategy,
                fn($crate::FixedBytes<$n>) -> Self,
            >;

            #[inline]
            fn arbitrary() -> Self::Strategy {
                use $crate::private::proptest::strategy::Strategy;
                <$crate::FixedBytes<$n> as $crate::private::proptest::arbitrary::Arbitrary>::arbitrary()
                    .prop_map(Self)
            }

            #[inline]
            fn arbitrary_with(args: Self::Parameters) -> Self::Strategy {
                use $crate::private::proptest::strategy::Strategy;
                <$crate::FixedBytes<$n> as $crate::private::proptest::arbitrary::Arbitrary>::arbitrary_with(args)
                    .prop_map(Self)
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "arbitrary"))]
macro_rules! impl_arbitrary {
    ($t:ty, $n:literal) => {};
}

macro_rules! fixed_bytes_macros {
    ($d:tt $($(#[$attr:meta])* macro $name:ident($ty:ident $($rest:tt)*);)*) => {$(
        /// Converts a sequence of string literals containing hex-encoded data
        #[doc = concat!(
            "into a new [`", stringify!($ty), "`][crate::", stringify!($ty), "] at compile time.\n",
        )]
        ///
        /// If the input is empty, a zero-initialized array is returned.
        ///
        /// See [`hex!`](crate::hex!) for more information.
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use alloy_primitives::{", stringify!($name), ", ", stringify!($ty), "};")]
        ///
        #[doc = concat!("const ZERO: ", stringify!($ty $($rest)*), " = ", stringify!($name), "!();")]
        #[doc = concat!("assert_eq!(ZERO, ", stringify!($ty), "::ZERO);")]
        ///
        /// # stringify!(
        #[doc = concat!("let byte_array: ", stringify!($ty), " = ", stringify!($name), "!(\"0x0123abcd…\");")]
        /// # );
        /// ```
        $(#[$attr])*
        #[macro_export]
        macro_rules! $name {
            () => {
                $crate::$ty::ZERO
            };

            ($d ($d t:tt)+) => {
                $crate::$ty::new($crate::hex!($d ($d t)+))
            };
        }
    )*};
}

fixed_bytes_macros! { $
    macro address(Address);

    macro b64(B64);

    macro b128(B128);

    macro b256(B256);

    macro b512(B512);

    macro bloom(Bloom);

    macro fixed_bytes(FixedBytes<0>); // <0> is just for the doctest
}

/// Converts a sequence of string literals containing hex-encoded data into a
/// new [`Bytes`][crate::Bytes] at compile time.
///
/// If the input is empty, an empty instance is returned.
///
/// See [`hex!`](crate::hex!) for more information.
///
/// # Examples
///
/// ```
/// use alloy_primitives::{bytes, Bytes};
///
/// static MY_BYTES: Bytes = bytes!("0x0123" "0xabcd");
/// assert_eq!(MY_BYTES, Bytes::from(&[0x01, 0x23, 0xab, 0xcd]));
/// ```
#[macro_export]
macro_rules! bytes {
    () => {
        $crate::Bytes::new()
    };

    ($($s:literal)+) => {const {
        $crate::Bytes::from_static(&$crate::hex!($($s)+))
    }};

    [$($inner:expr),+ $(,)?] => {const {
        $crate::Bytes::from_static(&[$($inner),+])
    }};

    [$inner:expr; $size:literal] => {const {
        $crate::Bytes::from_static(&[$inner; $size])
    }};
}

#[cfg(test)]
mod tests {
    use crate::{hex, Address, Bytes, FixedBytes};

    #[test]
    fn bytes_macros() {
        static B1: Bytes = bytes!("010203040506070809");
        static B2: Bytes = bytes![1, 2, 3, 4, 5, 6, 7, 8, 9];
        static B3: Bytes = bytes![1, 2, 3, 4, 5, 6, 7, 8, 9,];

        assert_eq!(B1, B2);
        assert_eq!(B1, B3);

        static B4: Bytes = bytes!("0000");
        static B5: Bytes = bytes![0; 2];
        static B6: Bytes = bytes![0, 0];
        assert_eq!(B4, B5);
        assert_eq!(B4, B6);
    }

    #[test]
    fn fixed_byte_macros() {
        const A0: Address = address!();
        assert_eq!(A0, Address::ZERO);

        const A1: Address = address!("0x0102030405060708090a0b0c0d0e0f1011121314");
        const A2: Address = Address(fixed_bytes!("0x0102030405060708090a0b0c0d0e0f1011121314"));
        const A3: Address = Address(FixedBytes(hex!("0x0102030405060708090a0b0c0d0e0f1011121314")));
        assert_eq!(A1, A2);
        assert_eq!(A1, A3);
        assert_eq!(A1, hex!("0x0102030405060708090a0b0c0d0e0f1011121314"));

        static B: Bytes = bytes!("0x112233");
        assert_eq!(B[..], [0x11, 0x22, 0x33]);

        static EMPTY_BYTES1: Bytes = bytes!();
        static EMPTY_BYTES2: Bytes = bytes!("");
        assert!(EMPTY_BYTES1.is_empty());
        assert_eq!(EMPTY_BYTES1, Bytes::new());
        assert_eq!(EMPTY_BYTES1, EMPTY_BYTES2);
    }
}
