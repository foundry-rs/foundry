use alloy_primitives::uint;

pub mod calldata;
pub mod element;
pub mod memory;
pub mod op;
pub mod stack;
pub mod vm;

pub use alloy_primitives::{I256, U256};

pub const VAL_0_B: [u8; 32] = U256::ZERO.to_be_bytes();

pub const VAL_1: U256 = uint!(1_U256);
pub const VAL_1_B: [u8; 32] = VAL_1.to_be_bytes();

pub const VAL_4: U256 = uint!(4_U256);

pub const VAL_32: U256 = uint!(32_U256);
pub const VAL_32_B: [u8; 32] = VAL_32.to_be_bytes();

pub const VAL_256: U256 = uint!(256_U256);

pub const VAL_1024: U256 = uint!(1024_U256);
pub const VAL_1024_B: [u8; 32] = VAL_1024.to_be_bytes();

pub const VAL_131072: U256 = uint!(131072_U256);

pub const VAL_1M: U256 = uint!(1000000_U256);
pub const VAL_1M_B: [u8; 32] = VAL_1M.to_be_bytes();
