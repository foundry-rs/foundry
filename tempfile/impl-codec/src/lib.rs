// Copyright 2020 Parity Technologies
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Parity Codec serialization support for uint and fixed hash.

#![cfg_attr(not(feature = "std"), no_std)]

#[doc(hidden)]
pub use parity_scale_codec as codec;

/// Add Parity Codec serialization support to an integer created by `construct_uint!`.
#[macro_export]
macro_rules! impl_uint_codec {
	($name: ident, $len: expr) => {
		impl $crate::codec::Encode for $name {
			fn using_encoded<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
				let mut bytes = [0u8; $len * 8];
				self.to_little_endian(&mut bytes);
				bytes.using_encoded(f)
			}
		}

		impl $crate::codec::EncodeLike for $name {}

		impl $crate::codec::Decode for $name {
			fn decode<I: $crate::codec::Input>(input: &mut I) -> core::result::Result<Self, $crate::codec::Error> {
				<[u8; $len * 8] as $crate::codec::Decode>::decode(input).map(|b| $name::from_little_endian(&b))
			}
		}

		impl $crate::codec::MaxEncodedLen for $name {
			fn max_encoded_len() -> usize {
				::core::mem::size_of::<$name>()
			}
		}
	};
}

/// Add Parity Codec serialization support to a fixed-sized hash type created by `construct_fixed_hash!`.
#[macro_export]
macro_rules! impl_fixed_hash_codec {
	($name: ident, $len: expr) => {
		impl $crate::codec::Encode for $name {
			fn using_encoded<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
				self.0.using_encoded(f)
			}
		}

		impl $crate::codec::EncodeLike for $name {}

		impl $crate::codec::Decode for $name {
			fn decode<I: $crate::codec::Input>(input: &mut I) -> core::result::Result<Self, $crate::codec::Error> {
				<[u8; $len] as $crate::codec::Decode>::decode(input).map($name)
			}
		}

		impl $crate::codec::MaxEncodedLen for $name {
			fn max_encoded_len() -> usize {
				::core::mem::size_of::<$name>()
			}
		}
	};
}
