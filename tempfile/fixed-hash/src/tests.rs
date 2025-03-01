// Copyright 2020 Parity Technologies
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

construct_fixed_hash! { pub struct H32(4); }
construct_fixed_hash! { pub struct H64(8); }
construct_fixed_hash! { pub struct H128(16); }
construct_fixed_hash! { pub struct H160(20); }
construct_fixed_hash! { pub struct H256(32); }

impl_fixed_hash_conversions!(H256, H160);

mod repeat_byte {
	use super::*;

	#[test]
	fn patterns() {
		assert_eq!(H32::repeat_byte(0xFF), H32::from([0xFF; 4]));
		assert_eq!(H32::repeat_byte(0xAA), H32::from([0xAA; 4]));
	}

	#[test]
	fn zero() {
		assert_eq!(H32::repeat_byte(0x0), H32::zero());
		assert_eq!(H32::repeat_byte(0x0), H32::from([0x0; 4]));
	}
}

#[test]
fn len_bytes() {
	assert_eq!(H32::len_bytes(), 4);
	assert_eq!(H64::len_bytes(), 8);
	assert_eq!(H128::len_bytes(), 16);
	assert_eq!(H160::len_bytes(), 20);
	assert_eq!(H256::len_bytes(), 32);
}

#[test]
fn as_bytes() {
	assert_eq!(H32::from([0x55; 4]).as_bytes(), &[0x55; 4]);
	assert_eq!(H32::from([0x42; 4]).as_bytes_mut(), &mut [0x42; 4]);
}

mod assign_from_slice {
	use super::*;

	#[test]
	fn zeros_to_ones() {
		assert_eq!(H32::from([0xFF; 4]), {
			let mut cmp = H32::zero();
			cmp.assign_from_slice(&[0xFF; 4]);
			cmp
		});
	}

	#[test]
	#[should_panic]
	fn fail_too_few_elems() {
		let mut dummy = H32::zero();
		dummy.assign_from_slice(&[0x42; 3]);
	}

	#[test]
	#[should_panic]
	fn fail_too_many_elems() {
		let mut dummy = H32::zero();
		dummy.assign_from_slice(&[0x42; 5]);
	}
}

mod from_slice {
	use super::*;

	#[test]
	fn simple() {
		assert_eq!(H32::from([0x10; 4]), H32::from_slice(&[0x10; 4]));
	}

	#[test]
	#[should_panic]
	fn fail_too_few_elems() {
		H32::from_slice(&[0x10; 3]);
	}

	#[test]
	#[should_panic]
	fn fail_too_many_elems() {
		H32::from_slice(&[0x10; 5]);
	}
}

mod covers {
	use super::*;

	#[test]
	fn simple() {
		assert!(H32::from([0xFF; 4]).covers(&H32::zero()));
		assert!(!(H32::zero().covers(&H32::from([0xFF; 4]))));
	}

	#[test]
	fn zero_covers_zero() {
		assert!(H32::zero().covers(&H32::zero()));
	}

	#[test]
	fn ones_covers_ones() {
		assert!(H32::from([0xFF; 4]).covers(&H32::from([0xFF; 4])));
	}

	#[test]
	fn complex_covers() {
		#[rustfmt::skip]
		assert!(
			H32::from([0b0110_0101, 0b1000_0001, 0b1010_1010, 0b0110_0011]).covers(&
			H32::from([0b0010_0100, 0b1000_0001, 0b0010_1010, 0b0110_0010]))
		);
	}

	#[test]
	fn complex_uncovers() {
		#[rustfmt::skip]
		assert!(
			!(
				H32::from([0b0010_0100, 0b1000_0001, 0b0010_1010, 0b0110_0010]).covers(&
				H32::from([0b0110_0101, 0b1000_0001, 0b1010_1010, 0b0110_0011]))
			)
		);
	}
}

mod is_zero {
	use super::*;

	#[test]
	fn all_true() {
		assert!(H32::zero().is_zero());
		assert!(H64::zero().is_zero());
		assert!(H128::zero().is_zero());
		assert!(H160::zero().is_zero());
		assert!(H256::zero().is_zero());
	}

	#[test]
	fn all_false() {
		assert!(!H32::repeat_byte(42).is_zero());
		assert!(!H64::repeat_byte(42).is_zero());
		assert!(!H128::repeat_byte(42).is_zero());
		assert!(!H160::repeat_byte(42).is_zero());
		assert!(!H256::repeat_byte(42).is_zero());
	}
}

#[cfg(feature = "byteorder")]
mod to_low_u64 {
	use super::*;

	#[test]
	fn smaller_size() {
		assert_eq!(H32::from([0x01, 0x23, 0x45, 0x67]).to_low_u64_be(), 0x0123_4567);
		assert_eq!(H32::from([0x01, 0x23, 0x45, 0x67]).to_low_u64_le(), 0x6745_2301_0000_0000);
	}

	#[test]
	fn equal_size() {
		assert_eq!(H64::from([0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF]).to_low_u64_le(), 0xEFCD_AB89_6745_2301);
		assert_eq!(H64::from([0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF]).to_low_u64_be(), 0x0123_4567_89AB_CDEF)
	}

	#[test]
	#[rustfmt::skip]
	fn larger_size() {
		assert_eq!(
			H128::from([
				0x01, 0x23, 0x45, 0x67,
				0x89, 0xAB, 0xCD, 0xEF,
				0x09, 0x08, 0x07, 0x06,
				0x05, 0x04, 0x03, 0x02
			]).to_low_u64_be(),
			0x0908070605040302
		);
		assert_eq!(
			H128::from([
				0x01, 0x23, 0x45, 0x67,
				0x89, 0xAB, 0xCD, 0xEF,
				0x09, 0x08, 0x07, 0x06,
				0x05, 0x04, 0x03, 0x02
			]).to_low_u64_le(),
			0x0203040506070809
		)
	}
}

#[cfg(feature = "byteorder")]
mod from_low_u64 {
	use super::*;

	#[test]
	fn smaller_size() {
		assert_eq!(H32::from_low_u64_be(0x0123_4567_89AB_CDEF), H32::from([0x01, 0x23, 0x45, 0x67]));
		assert_eq!(H32::from_low_u64_le(0x0123_4567_89AB_CDEF), H32::from([0xEF, 0xCD, 0xAB, 0x89]));
	}

	#[test]
	fn equal_size() {
		assert_eq!(
			H64::from_low_u64_be(0x0123_4567_89AB_CDEF),
			H64::from([0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF])
		);
		assert_eq!(
			H64::from_low_u64_le(0x0123_4567_89AB_CDEF),
			H64::from([0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01])
		)
	}

	#[test]
	#[rustfmt::skip]
	fn larger_size() {
		assert_eq!(
			H128::from_low_u64_be(0x0123_4567_89AB_CDEF),
			H128::from([
				0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
				0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF
			])
		);
		assert_eq!(
			H128::from_low_u64_le(0x0123_4567_89AB_CDEF),
			H128::from([
				0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
				0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01
			])
		)
	}
}

#[cfg(feature = "rand")]
mod rand {
	use super::*;
	use ::rand::{rngs::StdRng, SeedableRng};

	#[test]
	fn random() {
		let mut rng = StdRng::seed_from_u64(123);
		assert_eq!(H32::random_using(&mut rng), H32::from([0xeb, 0x96, 0xaf, 0x1c]));
	}
}

#[cfg(feature = "rustc-hex")]
mod from_str {
	use super::*;

	#[test]
	fn valid() {
		use crate::core_::str::FromStr;

		assert_eq!(
			H64::from_str("0123456789ABCDEF").unwrap(),
			H64::from([0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF])
		)
	}

	#[test]
	fn empty_str() {
		use crate::core_::str::FromStr;
		assert!(H64::from_str("").is_err())
	}

	#[test]
	fn invalid_digits() {
		use crate::core_::str::FromStr;
		assert!(H64::from_str("Hello, World!").is_err())
	}

	#[test]
	fn too_many_digits() {
		use crate::core_::str::FromStr;
		assert!(H64::from_str("0123456789ABCDEF0").is_err())
	}
}

#[test]
fn from_h160_to_h256() {
	let h160 = H160::from([
		0xEF, 0x2D, 0x6D, 0x19, 0x40, 0x84, 0xC2, 0xDE, 0x36, 0xE0, 0xDA, 0xBF, 0xCE, 0x45, 0xD0, 0x46, 0xB3, 0x7D,
		0x11, 0x06,
	]);
	let h256 = H256::from(h160);
	let expected = H256::from([
		0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEF, 0x2D, 0x6D, 0x19, 0x40, 0x84,
		0xC2, 0xDE, 0x36, 0xE0, 0xDA, 0xBF, 0xCE, 0x45, 0xD0, 0x46, 0xB3, 0x7D, 0x11, 0x06,
	]);
	assert_eq!(h256, expected);
}

#[test]
#[rustfmt::skip]
fn from_h256_to_h160_lossless() {
	let h256 = H256::from([
		0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
		0xEF, 0x2D, 0x6D, 0x19, 0x40, 0x84, 0xC2, 0xDE, 0x36, 0xE0,
		0xDA, 0xBF, 0xCE, 0x45, 0xD0, 0x46, 0xB3, 0x7D, 0x11, 0x06,
	]);
	let h160 = H160::from(h256);
	let expected = H160::from([
		0xEF, 0x2D, 0x6D, 0x19, 0x40, 0x84, 0xC2, 0xDE, 0x36, 0xE0, 0xDA, 0xBF, 0xCE, 0x45, 0xD0,
		0x46, 0xB3, 0x7D, 0x11, 0x06,
	]);
	assert_eq!(h160, expected);
}

#[test]
#[rustfmt::skip]
fn from_h256_to_h160_lossy() {
	let h256 = H256::from([
		0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
		0xEF, 0x2D, 0x6D, 0x19, 0x40, 0x84, 0xC2, 0xDE, 0x36, 0xE0,
		0xDA, 0xBF, 0xCE, 0x45, 0xD0, 0x46, 0xB3, 0x7D, 0x11, 0x06,
	]);
	let h160 = H160::from(h256);
	let expected = H160::from([
		0xEF, 0x2D, 0x6D, 0x19, 0x40, 0x84, 0xC2, 0xDE, 0x36, 0xE0,
		0xDA, 0xBF, 0xCE, 0x45, 0xD0, 0x46, 0xB3, 0x7D, 0x11, 0x06,
	]);
	assert_eq!(h160, expected);
}

#[cfg(all(feature = "std", feature = "byteorder"))]
#[test]
fn display_and_debug() {
	fn test_for(x: u64, hex: &'static str, display: &'static str) {
		let hash = H64::from_low_u64_be(x);

		assert_eq!(format!("{}", hash), format!("0x{}", display));
		assert_eq!(format!("{:?}", hash), format!("0x{}", hex));
		assert_eq!(format!("{:x}", hash), hex);
		assert_eq!(format!("{:#x}", hash), format!("0x{}", hex));
	}

	test_for(0x0001, "0000000000000001", "0000…0001");
	test_for(0x000f, "000000000000000f", "0000…000f");
	test_for(0x0010, "0000000000000010", "0000…0010");
	test_for(0x00ff, "00000000000000ff", "0000…00ff");
	test_for(0x0100, "0000000000000100", "0000…0100");
	test_for(0x0fff, "0000000000000fff", "0000…0fff");
	test_for(0x1000, "0000000000001000", "0000…1000");
}

mod ops {
	use super::*;

	fn lhs() -> H32 {
		H32::from([0b0011_0110, 0b0001_0011, 0b1010_1010, 0b0001_0010])
	}

	fn rhs() -> H32 {
		H32::from([0b0101_0101, 0b1111_1111, 0b1100_1100, 0b0000_1111])
	}

	#[test]
	fn bitand() {
		assert_eq!(
			lhs() & rhs(),
			H32::from([
				0b0011_0110 & 0b0101_0101,
				0b0001_0011 & 0b1111_1111,
				0b1010_1010 & 0b1100_1100,
				0b0001_0010 & 0b0000_1111
			])
		)
	}

	#[test]
	fn bitor() {
		assert_eq!(
			lhs() | rhs(),
			H32::from([
				0b0011_0110 | 0b0101_0101,
				0b0001_0011 | 0b1111_1111,
				0b1010_1010 | 0b1100_1100,
				0b0001_0010 | 0b0000_1111
			])
		)
	}

	#[test]
	fn bitxor() {
		assert_eq!(
			lhs() ^ rhs(),
			H32::from([
				0b0011_0110 ^ 0b0101_0101,
				0b0001_0011 ^ 0b1111_1111,
				0b1010_1010 ^ 0b1100_1100,
				0b0001_0010 ^ 0b0000_1111
			])
		)
	}
}
