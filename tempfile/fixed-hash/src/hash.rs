// Copyright 2020 Parity Technologies
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

/// Construct a fixed-size hash type.
///
/// # Examples
///
/// Create a public unformatted hash type with 32 bytes size.
///
/// ```
/// use fixed_hash::construct_fixed_hash;
///
/// construct_fixed_hash!{ pub struct H256(32); }
/// assert_eq!(std::mem::size_of::<H256>(), 32);
/// ```
///
/// With additional attributes and doc comments.
///
/// ```
/// use fixed_hash::construct_fixed_hash;
/// construct_fixed_hash!{
///     /// My unformatted 160 bytes sized hash type.
///     #[cfg_attr(feature = "serialize", derive(Serialize, Deserialize))]
///     pub struct H160(20);
/// }
/// assert_eq!(std::mem::size_of::<H160>(), 20);
/// ```
///
/// The visibility modifier is optional and you can create a private hash type.
///
/// ```
/// use fixed_hash::construct_fixed_hash;
/// construct_fixed_hash!{ struct H512(64); }
/// assert_eq!(std::mem::size_of::<H512>(), 64);
/// ```
#[macro_export(local_inner_macros)]
macro_rules! construct_fixed_hash {
	( $(#[$attr:meta])* $visibility:vis struct $name:ident ( $n_bytes:expr ); ) => {
		#[repr(C)]
		$(#[$attr])*
		$visibility struct $name (pub [u8; $n_bytes]);

		impl From<[u8; $n_bytes]> for $name {
			/// Constructs a hash type from the given bytes array of fixed length.
			///
			/// # Note
			///
			/// The given bytes are interpreted in big endian order.
			#[inline]
			fn from(bytes: [u8; $n_bytes]) -> Self {
				$name(bytes)
			}
		}

		impl<'a> From<&'a [u8; $n_bytes]> for $name {
			/// Constructs a hash type from the given reference
			/// to the bytes array of fixed length.
			///
			/// # Note
			///
			/// The given bytes are interpreted in big endian order.
			#[inline]
			fn from(bytes: &'a [u8; $n_bytes]) -> Self {
				$name(*bytes)
			}
		}

		impl<'a> From<&'a mut [u8; $n_bytes]> for $name {
			/// Constructs a hash type from the given reference
			/// to the mutable bytes array of fixed length.
			///
			/// # Note
			///
			/// The given bytes are interpreted in big endian order.
			#[inline]
			fn from(bytes: &'a mut [u8; $n_bytes]) -> Self {
				$name(*bytes)
			}
		}

		impl From<$name> for [u8; $n_bytes] {
			#[inline]
			fn from(s: $name) -> Self {
				s.0
			}
		}

		impl AsRef<[u8]> for $name {
			#[inline]
			fn as_ref(&self) -> &[u8] {
				self.as_bytes()
			}
		}

		impl AsMut<[u8]> for $name {
			#[inline]
			fn as_mut(&mut self) -> &mut [u8] {
				self.as_bytes_mut()
			}
		}

		impl $name {
			/// Returns a new fixed hash where all bits are set to the given byte.
			#[inline]
			pub const fn repeat_byte(byte: u8) -> $name {
				$name([byte; $n_bytes])
			}

			/// Returns a new zero-initialized fixed hash.
			#[inline]
			pub const fn zero() -> $name {
				$name::repeat_byte(0u8)
			}

			/// Returns the size of this hash in bytes.
			#[inline]
			pub const fn len_bytes() -> usize {
				$n_bytes
			}

			/// Extracts a byte slice containing the entire fixed hash.
			#[inline]
			pub fn as_bytes(&self) -> &[u8] {
				&self.0
			}

			/// Extracts a mutable byte slice containing the entire fixed hash.
			#[inline]
			pub fn as_bytes_mut(&mut self) -> &mut [u8] {
				&mut self.0
			}

			/// Extracts a reference to the byte array containing the entire fixed hash.
			#[inline]
			pub const fn as_fixed_bytes(&self) -> &[u8; $n_bytes] {
				&self.0
			}

			/// Extracts a reference to the byte array containing the entire fixed hash.
			#[inline]
			pub fn as_fixed_bytes_mut(&mut self) -> &mut [u8; $n_bytes] {
				&mut self.0
			}

			/// Returns the inner bytes array.
			#[inline]
			pub const fn to_fixed_bytes(self) -> [u8; $n_bytes] {
				self.0
			}

			/// Returns a constant raw pointer to the value.
			#[inline]
			pub fn as_ptr(&self) -> *const u8 {
				self.as_bytes().as_ptr()
			}

			/// Returns a mutable raw pointer to the value.
			#[inline]
			pub fn as_mut_ptr(&mut self) -> *mut u8 {
				self.as_bytes_mut().as_mut_ptr()
			}

			/// Assign the bytes from the byte slice `src` to `self`.
			///
			/// # Note
			///
			/// The given bytes are interpreted in big endian order.
			///
			/// # Panics
			///
			/// If the length of `src` and the number of bytes in `self` do not match.
			pub fn assign_from_slice(&mut self, src: &[u8]) {
				$crate::core_::assert_eq!(src.len(), $n_bytes);
				self.as_bytes_mut().copy_from_slice(src);
			}

			/// Create a new fixed-hash from the given slice `src`.
			///
			/// # Note
			///
			/// The given bytes are interpreted in big endian order.
			///
			/// # Panics
			///
			/// If the length of `src` and the number of bytes in `Self` do not match.
			pub fn from_slice(src: &[u8]) -> Self {
				$crate::core_::assert_eq!(src.len(), $n_bytes);
				let mut ret = Self::zero();
				ret.assign_from_slice(src);
				ret
			}

			/// Returns `true` if all bits set in `b` are also set in `self`.
			#[inline]
			pub fn covers(&self, b: &Self) -> bool {
				&(b & self) == b
			}

			/// Returns `true` if no bits are set.
			#[inline]
			pub fn is_zero(&self) -> bool {
				self.as_bytes().iter().all(|&byte| byte == 0u8)
			}
		}

		impl $crate::core_::fmt::Debug for $name {
			fn fmt(&self, f: &mut $crate::core_::fmt::Formatter) -> $crate::core_::fmt::Result {
				$crate::core_::write!(f, "{:#x}", self)
			}
		}

		impl $crate::core_::fmt::Display for $name {
			fn fmt(&self, f: &mut $crate::core_::fmt::Formatter) -> $crate::core_::fmt::Result {
				$crate::core_::write!(f, "0x")?;
				for i in &self.0[0..2] {
					$crate::core_::write!(f, "{:02x}", i)?;
				}
				$crate::core_::write!(f, "…")?;
				for i in &self.0[$n_bytes - 2..$n_bytes] {
					$crate::core_::write!(f, "{:02x}", i)?;
				}
				Ok(())
			}
		}

		impl $crate::core_::fmt::LowerHex for $name {
			fn fmt(&self, f: &mut $crate::core_::fmt::Formatter) -> $crate::core_::fmt::Result {
				if f.alternate() {
					$crate::core_::write!(f, "0x")?;
				}
				for i in &self.0[..] {
					$crate::core_::write!(f, "{:02x}", i)?;
				}
				Ok(())
			}
		}

		impl $crate::core_::fmt::UpperHex for $name {
			fn fmt(&self, f: &mut $crate::core_::fmt::Formatter) -> $crate::core_::fmt::Result {
				if f.alternate() {
					$crate::core_::write!(f, "0X")?;
				}
				for i in &self.0[..] {
					$crate::core_::write!(f, "{:02X}", i)?;
				}
				Ok(())
			}
		}

		impl $crate::core_::marker::Copy for $name {}

		#[cfg_attr(feature = "dev", allow(expl_impl_clone_on_copy))]
		impl $crate::core_::clone::Clone for $name {
			fn clone(&self) -> $name {
				let mut ret = $name::zero();
				ret.0.copy_from_slice(&self.0);
				ret
			}
		}

		impl $crate::core_::cmp::Eq for $name {}

		impl $crate::core_::cmp::PartialOrd for $name {
			fn partial_cmp(&self, other: &Self) -> Option<$crate::core_::cmp::Ordering> {
				Some(self.cmp(other))
			}
		}

		impl $crate::core_::hash::Hash for $name {
			fn hash<H>(&self, state: &mut H) where H: $crate::core_::hash::Hasher {
				state.write(&self.0);
			}
		}

		impl<I> $crate::core_::ops::Index<I> for $name
		where
			I: $crate::core_::slice::SliceIndex<[u8]>
		{
			type Output = I::Output;

			#[inline]
			fn index(&self, index: I) -> &I::Output {
				&self.as_bytes()[index]
			}
		}

		impl<I> $crate::core_::ops::IndexMut<I> for $name
		where
			I: $crate::core_::slice::SliceIndex<[u8], Output = [u8]>
		{
			#[inline]
			fn index_mut(&mut self, index: I) -> &mut I::Output {
				&mut self.as_bytes_mut()[index]
			}
		}

		impl $crate::core_::default::Default for $name {
			#[inline]
			fn default() -> Self {
				Self::zero()
			}
		}

		impl_ops_for_hash!($name, BitOr, bitor, BitOrAssign, bitor_assign, |, |=);
		impl_ops_for_hash!($name, BitAnd, bitand, BitAndAssign, bitand_assign, &, &=);
		impl_ops_for_hash!($name, BitXor, bitxor, BitXorAssign, bitxor_assign, ^, ^=);

		impl_byteorder_for_fixed_hash!($name);
		impl_rand_for_fixed_hash!($name);
		impl_cmp_for_fixed_hash!($name);
		impl_rustc_hex_for_fixed_hash!($name);
		impl_quickcheck_for_fixed_hash!($name);
		impl_arbitrary_for_fixed_hash!($name);
	}
}

// Implementation for disabled byteorder crate support.
//
// # Note
//
// Feature guarded macro definitions instead of feature guarded impl blocks
// to work around the problems of introducing `byteorder` crate feature in
// a user crate.
#[cfg(not(feature = "byteorder"))]
#[macro_export]
#[doc(hidden)]
macro_rules! impl_byteorder_for_fixed_hash {
	( $name:ident ) => {};
}

// Implementation for enabled byteorder crate support.
//
// # Note
//
// Feature guarded macro definitions instead of feature guarded impl blocks
// to work around the problems of introducing `byteorder` crate feature in
// a user crate.
#[cfg(feature = "byteorder")]
#[macro_export]
#[doc(hidden)]
macro_rules! impl_byteorder_for_fixed_hash {
	( $name:ident ) => {
		/// Utilities using the `byteorder` crate.
		impl $name {
			/// Returns the least significant `n` bytes as slice.
			///
			/// # Panics
			///
			/// If `n` is greater than the number of bytes in `self`.
			#[inline]
			fn least_significant_bytes(&self, n: usize) -> &[u8] {
				$crate::core_::assert_eq!(true, n <= Self::len_bytes());
				&self[(Self::len_bytes() - n)..]
			}

			fn to_low_u64_with_byteorder<B>(&self) -> u64
			where
				B: $crate::byteorder::ByteOrder,
			{
				let mut buf = [0x0; 8];
				let capped = $crate::core_::cmp::min(Self::len_bytes(), 8);
				buf[(8 - capped)..].copy_from_slice(self.least_significant_bytes(capped));
				B::read_u64(&buf)
			}

			/// Returns the lowest 8 bytes interpreted as big-endian.
			///
			/// # Note
			///
			/// For hash type with less than 8 bytes the missing bytes
			/// are interpreted as being zero.
			#[inline]
			pub fn to_low_u64_be(&self) -> u64 {
				self.to_low_u64_with_byteorder::<$crate::byteorder::BigEndian>()
			}

			/// Returns the lowest 8 bytes interpreted as little-endian.
			///
			/// # Note
			///
			/// For hash type with less than 8 bytes the missing bytes
			/// are interpreted as being zero.
			#[inline]
			pub fn to_low_u64_le(&self) -> u64 {
				self.to_low_u64_with_byteorder::<$crate::byteorder::LittleEndian>()
			}

			/// Returns the lowest 8 bytes interpreted as native-endian.
			///
			/// # Note
			///
			/// For hash type with less than 8 bytes the missing bytes
			/// are interpreted as being zero.
			#[inline]
			pub fn to_low_u64_ne(&self) -> u64 {
				self.to_low_u64_with_byteorder::<$crate::byteorder::NativeEndian>()
			}

			fn from_low_u64_with_byteorder<B>(val: u64) -> Self
			where
				B: $crate::byteorder::ByteOrder,
			{
				let mut buf = [0x0; 8];
				B::write_u64(&mut buf, val);
				let capped = $crate::core_::cmp::min(Self::len_bytes(), 8);
				let mut bytes = [0x0; $crate::core_::mem::size_of::<Self>()];
				bytes[(Self::len_bytes() - capped)..].copy_from_slice(&buf[..capped]);
				Self::from_slice(&bytes)
			}

			/// Creates a new hash type from the given `u64` value.
			///
			/// # Note
			///
			/// - The given `u64` value is interpreted as big endian.
			/// - Ignores the most significant bits of the given value
			///   if the hash type has less than 8 bytes.
			#[inline]
			pub fn from_low_u64_be(val: u64) -> Self {
				Self::from_low_u64_with_byteorder::<$crate::byteorder::BigEndian>(val)
			}

			/// Creates a new hash type from the given `u64` value.
			///
			/// # Note
			///
			/// - The given `u64` value is interpreted as little endian.
			/// - Ignores the most significant bits of the given value
			///   if the hash type has less than 8 bytes.
			#[inline]
			pub fn from_low_u64_le(val: u64) -> Self {
				Self::from_low_u64_with_byteorder::<$crate::byteorder::LittleEndian>(val)
			}

			/// Creates a new hash type from the given `u64` value.
			///
			/// # Note
			///
			/// - The given `u64` value is interpreted as native endian.
			/// - Ignores the most significant bits of the given value
			///   if the hash type has less than 8 bytes.
			#[inline]
			pub fn from_low_u64_ne(val: u64) -> Self {
				Self::from_low_u64_with_byteorder::<$crate::byteorder::NativeEndian>(val)
			}
		}
	};
}

// Implementation for disabled rand crate support.
//
// # Note
//
// Feature guarded macro definitions instead of feature guarded impl blocks
// to work around the problems of introducing `rand` crate feature in
// a user crate.
#[cfg(not(feature = "rand"))]
#[macro_export]
#[doc(hidden)]
macro_rules! impl_rand_for_fixed_hash {
	( $name:ident ) => {};
}

// Implementation for enabled rand crate support.
//
// # Note
//
// Feature guarded macro definitions instead of feature guarded impl blocks
// to work around the problems of introducing `rand` crate feature in
// a user crate.
#[cfg(feature = "rand")]
#[macro_export]
#[doc(hidden)]
macro_rules! impl_rand_for_fixed_hash {
	( $name:ident ) => {
		impl $crate::rand::distributions::Distribution<$name> for $crate::rand::distributions::Standard {
			fn sample<R: $crate::rand::Rng + ?Sized>(&self, rng: &mut R) -> $name {
				let mut ret = $name::zero();
				for byte in ret.as_bytes_mut().iter_mut() {
					*byte = rng.gen();
				}
				ret
			}
		}

		/// Utilities using the `rand` crate.
		impl $name {
			/// Assign `self` to a cryptographically random value using the
			/// given random number generator.
			pub fn randomize_using<R>(&mut self, rng: &mut R)
			where
				R: $crate::rand::Rng + ?Sized,
			{
				use $crate::rand::distributions::Distribution;
				*self = $crate::rand::distributions::Standard.sample(rng);
			}

			/// Assign `self` to a cryptographically random value.
			pub fn randomize(&mut self) {
				let mut rng = $crate::rand::rngs::OsRng;
				self.randomize_using(&mut rng);
			}

			/// Create a new hash with cryptographically random content using the
			/// given random number generator.
			pub fn random_using<R>(rng: &mut R) -> Self
			where
				R: $crate::rand::Rng + ?Sized,
			{
				let mut ret = Self::zero();
				ret.randomize_using(rng);
				ret
			}

			/// Create a new hash with cryptographically random content.
			pub fn random() -> Self {
				let mut hash = Self::zero();
				hash.randomize();
				hash
			}
		}
	};
}

#[macro_export]
#[doc(hidden)]
macro_rules! impl_cmp_for_fixed_hash {
	( $name:ident ) => {
		impl $crate::core_::cmp::PartialEq for $name {
			#[inline]
			fn eq(&self, other: &Self) -> bool {
				self.as_bytes() == other.as_bytes()
			}
		}

		impl $crate::core_::cmp::Ord for $name {
			#[inline]
			fn cmp(&self, other: &Self) -> $crate::core_::cmp::Ordering {
				self.as_bytes().cmp(other.as_bytes())
			}
		}
	};
}

// Implementation for disabled rustc-hex crate support.
//
// # Note
//
// Feature guarded macro definitions instead of feature guarded impl blocks
// to work around the problems of introducing `rustc-hex` crate feature in
// a user crate.
#[cfg(not(feature = "rustc-hex"))]
#[macro_export]
#[doc(hidden)]
macro_rules! impl_rustc_hex_for_fixed_hash {
	( $name:ident ) => {};
}

// Implementation for enabled rustc-hex crate support.
//
// # Note
//
// Feature guarded macro definitions instead of feature guarded impl blocks
// to work around the problems of introducing `rustc-hex` crate feature in
// a user crate.
#[cfg(feature = "rustc-hex")]
#[macro_export]
#[doc(hidden)]
macro_rules! impl_rustc_hex_for_fixed_hash {
	( $name:ident ) => {
		impl $crate::core_::str::FromStr for $name {
			type Err = $crate::rustc_hex::FromHexError;

			/// Creates a hash type instance from the given string.
			///
			/// # Note
			///
			/// The given input string is interpreted in big endian.
			///
			/// # Errors
			///
			/// - When encountering invalid non hex-digits
			/// - Upon empty string input or invalid input length in general
			fn from_str(input: &str) -> $crate::core_::result::Result<$name, $crate::rustc_hex::FromHexError> {
				let input = input.strip_prefix("0x").unwrap_or(input);
				let mut iter = $crate::rustc_hex::FromHexIter::new(input);
				let mut result = Self::zero();
				for byte in result.as_mut() {
					*byte = iter.next().ok_or(Self::Err::InvalidHexLength)??;
				}
				if iter.next().is_some() {
					return Err(Self::Err::InvalidHexLength)
				}
				Ok(result)
			}
		}
	};
}

// Implementation for disabled quickcheck crate support.
//
// # Note
//
// Feature guarded macro definitions instead of feature guarded impl blocks
// to work around the problems of introducing `quickcheck` crate feature in
// a user crate.
#[cfg(not(feature = "quickcheck"))]
#[macro_export]
#[doc(hidden)]
macro_rules! impl_quickcheck_for_fixed_hash {
	( $name:ident ) => {};
}

// Implementation for enabled quickcheck crate support.
//
// # Note
//
// Feature guarded macro definitions instead of feature guarded impl blocks
// to work around the problems of introducing `quickcheck` crate feature in
// a user crate.
#[cfg(feature = "quickcheck")]
#[macro_export]
#[doc(hidden)]
macro_rules! impl_quickcheck_for_fixed_hash {
	( $name:ident ) => {
		impl $crate::quickcheck::Arbitrary for $name {
			fn arbitrary(g: &mut $crate::quickcheck::Gen) -> Self {
				let res: [u8; Self::len_bytes()] = $crate::core_::array::from_fn(|_| u8::arbitrary(g));
				Self::from(res)
			}
		}
	};
}

// When the `arbitrary` feature is disabled.
//
// # Note
//
// Feature guarded macro definitions instead of feature guarded impl blocks
// to work around the problems of introducing `arbitrary` crate feature in
// a user crate.
#[cfg(not(feature = "arbitrary"))]
#[macro_export]
#[doc(hidden)]
macro_rules! impl_arbitrary_for_fixed_hash {
	( $name:ident ) => {};
}

// When the `arbitrary` feature is enabled.
//
// # Note
//
// Feature guarded macro definitions instead of feature guarded impl blocks
// to work around the problems of introducing `arbitrary` crate feature in
// a user crate.
#[cfg(feature = "arbitrary")]
#[macro_export]
#[doc(hidden)]
macro_rules! impl_arbitrary_for_fixed_hash {
	( $name:ident ) => {
		impl $crate::arbitrary::Arbitrary<'_> for $name {
			fn arbitrary(u: &mut $crate::arbitrary::Unstructured<'_>) -> $crate::arbitrary::Result<Self> {
				let mut res = Self::zero();
				u.fill_buffer(&mut res.0)?;
				Ok(Self::from(res))
			}
		}
	};
}

#[macro_export]
#[doc(hidden)]
macro_rules! impl_ops_for_hash {
	(
		$impl_for:ident,
		$ops_trait_name:ident,
		$ops_fn_name:ident,
		$ops_assign_trait_name:ident,
		$ops_assign_fn_name:ident,
		$ops_tok:tt,
		$ops_assign_tok:tt
	) => {
		impl<'r> $crate::core_::ops::$ops_assign_trait_name<&'r $impl_for> for $impl_for {
			fn $ops_assign_fn_name(&mut self, rhs: &'r $impl_for) {
				for (lhs, rhs) in self.as_bytes_mut().iter_mut().zip(rhs.as_bytes()) {
					*lhs $ops_assign_tok rhs;
				}
			}
		}

		impl $crate::core_::ops::$ops_assign_trait_name<$impl_for> for $impl_for {
			#[inline]
			fn $ops_assign_fn_name(&mut self, rhs: $impl_for) {
				*self $ops_assign_tok &rhs;
			}
		}

		impl<'l, 'r> $crate::core_::ops::$ops_trait_name<&'r $impl_for> for &'l $impl_for {
			type Output = $impl_for;

			fn $ops_fn_name(self, rhs: &'r $impl_for) -> Self::Output {
				let mut ret = self.clone();
				ret $ops_assign_tok rhs;
				ret
			}
		}

		impl $crate::core_::ops::$ops_trait_name<$impl_for> for $impl_for {
			type Output = $impl_for;

			#[inline]
			fn $ops_fn_name(self, rhs: Self) -> Self::Output {
				&self $ops_tok &rhs
			}
		}
	};
}

/// Implements lossy conversions between the given types.
///
/// # Note
///
/// - Both types must be of different sizes.
/// - Type `large_ty` must have a larger memory footprint compared to `small_ty`.
///
/// # Panics
///
/// Both `From` implementations will panic if sizes of the given types
/// do not meet the requirements stated above.
///
/// # Example
///
/// ```
/// use fixed_hash::{construct_fixed_hash, impl_fixed_hash_conversions};
/// construct_fixed_hash!{ struct H160(20); }
/// construct_fixed_hash!{ struct H256(32); }
/// impl_fixed_hash_conversions!(H256, H160);
/// // now use it!
/// assert_eq!(H256::from(H160::zero()), H256::zero());
/// assert_eq!(H160::from(H256::zero()), H160::zero());
/// ```
#[macro_export(local_inner_macros)]
macro_rules! impl_fixed_hash_conversions {
	($large_ty:ident, $small_ty:ident) => {
		$crate::static_assertions::const_assert!(
			$crate::core_::mem::size_of::<$small_ty>() < $crate::core_::mem::size_of::<$large_ty>()
		);

		impl From<$small_ty> for $large_ty {
			fn from(value: $small_ty) -> $large_ty {
				let large_ty_size = $large_ty::len_bytes();
				let small_ty_size = $small_ty::len_bytes();

				$crate::core_::debug_assert!(
					large_ty_size > small_ty_size && large_ty_size % 2 == 0 && small_ty_size % 2 == 0
				);

				let mut ret = $large_ty::zero();
				ret.as_bytes_mut()[(large_ty_size - small_ty_size)..large_ty_size].copy_from_slice(value.as_bytes());
				ret
			}
		}

		impl From<$large_ty> for $small_ty {
			fn from(value: $large_ty) -> $small_ty {
				let large_ty_size = $large_ty::len_bytes();
				let small_ty_size = $small_ty::len_bytes();

				$crate::core_::debug_assert!(
					large_ty_size > small_ty_size && large_ty_size % 2 == 0 && small_ty_size % 2 == 0
				);

				let mut ret = $small_ty::zero();
				ret.as_bytes_mut()
					.copy_from_slice(&value[(large_ty_size - small_ty_size)..large_ty_size]);
				ret
			}
		}
	};
}
