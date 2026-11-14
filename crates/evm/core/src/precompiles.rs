use alloy_primitives::{Address, address};

/// The ECRecover precompile address.
pub const EC_RECOVER: Address = Address::with_last_byte(1);

/// The SHA-256 precompile address.
pub const SHA_256: Address = Address::with_last_byte(2);

/// The RIPEMD-160 precompile address.
pub const RIPEMD_160: Address = Address::with_last_byte(3);

/// The Identity precompile address.
pub const IDENTITY: Address = Address::with_last_byte(4);

/// The ModExp precompile address.
pub const MOD_EXP: Address = Address::with_last_byte(5);

/// The ECAdd precompile address.
pub const EC_ADD: Address = Address::with_last_byte(6);

/// The ECMul precompile address.
pub const EC_MUL: Address = Address::with_last_byte(7);

/// The ECPairing precompile address.
pub const EC_PAIRING: Address = Address::with_last_byte(8);

/// The Blake2F precompile address.
pub const BLAKE_2F: Address = Address::with_last_byte(9);

/// The PointEvaluation precompile address.
pub const POINT_EVALUATION: Address = Address::with_last_byte(0x0a);

/// The BLS12-381 G1ADD precompile address.
pub const BLS12_G1ADD: Address = Address::with_last_byte(0x0b);

/// The BLS12-381 G1MSM precompile address.
pub const BLS12_G1MSM: Address = Address::with_last_byte(0x0c);

/// The BLS12-381 G2ADD precompile address.
pub const BLS12_G2ADD: Address = Address::with_last_byte(0x0d);

/// The BLS12-381 G2MSM precompile address.
pub const BLS12_G2MSM: Address = Address::with_last_byte(0x0e);

/// The BLS12-381 pairing check precompile address.
pub const BLS12_PAIRING_CHECK: Address = Address::with_last_byte(0x0f);

/// The BLS12-381 map Fp to G1 precompile address.
pub const BLS12_MAP_FP_TO_G1: Address = Address::with_last_byte(0x10);

/// The BLS12-381 map Fp2 to G2 precompile address.
pub const BLS12_MAP_FP2_TO_G2: Address = Address::with_last_byte(0x11);

/// The P256VERIFY precompile address.
pub const P256_VERIFY: Address = address!("0x0000000000000000000000000000000000000100");

/// The Celo transfer precompile address.
///
/// See <https://specs.celo.org/token_duality.html#the-transfer-precompile>
pub const CELO_TRANSFER: Address = Address::with_last_byte(0xfd);

/// Precompile addresses.
pub const PRECOMPILES: &[Address] = &[
    EC_RECOVER,
    SHA_256,
    RIPEMD_160,
    IDENTITY,
    MOD_EXP,
    EC_ADD,
    EC_MUL,
    EC_PAIRING,
    BLAKE_2F,
    POINT_EVALUATION,
    BLS12_G1ADD,
    BLS12_G1MSM,
    BLS12_G2ADD,
    BLS12_G2MSM,
    BLS12_PAIRING_CHECK,
    BLS12_MAP_FP_TO_G1,
    BLS12_MAP_FP2_TO_G2,
    P256_VERIFY,
];
