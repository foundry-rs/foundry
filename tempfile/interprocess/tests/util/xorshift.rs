use std::{
	collections::hash_map::DefaultHasher,
	hash::{Hash, Hasher},
};

/// The 32-bit variant of the Xorshift PRNG algorithm.
///
/// Didn't feel like pulling in the `rand` crate, so have this here beauty instead.
#[repr(transparent)]
#[derive(Copy, Clone, Debug)]
pub struct Xorshift32(pub u32);
impl Xorshift32 {
	pub fn from_id(id: &str) -> Self {
		let mut hasher = DefaultHasher::new();
		id.hash(&mut hasher);
		let hash64 = hasher.finish();
		let hash32 = ((hash64 & 0xFFFF_FFFF_0000_0000) >> 32) ^ (hash64 & 0xFFFF_FFFF);
		Self(hash32.try_into().unwrap())
	}
	pub fn next(&mut self) -> u32 {
		self.0 ^= self.0 << 13;
		self.0 ^= self.0 >> 17;
		self.0 ^= self.0 << 5;
		self.0
	}
}
impl Iterator for Xorshift32 {
	type Item = u32;
	fn next(&mut self) -> Option<Self::Item> {
		Some(self.next())
	}
}
