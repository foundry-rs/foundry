use alloy_primitives::{Address, address};

/// The ECRecover precompile address.
pub const EC_RECOVER: Address = address!("0x0000000000000000000000000000000000000001");

/// The SHA-256 precompile address.
pub const SHA_256: Address = address!("0x0000000000000000000000000000000000000002");

/// The RIPEMD-160 precompile address.
pub const RIPEMD_160: Address = address!("0x0000000000000000000000000000000000000003");

/// The Identity precompile address.
pub const IDENTITY: Address = address!("0x0000000000000000000000000000000000000004");

/// The ModExp precompile address.
pub const MOD_EXP: Address = address!("0x0000000000000000000000000000000000000005");

/// The ECAdd precompile address.
pub const EC_ADD: Address = address!("0x0000000000000000000000000000000000000006");

/// The ECMul precompile address.
pub const EC_MUL: Address = address!("0x0000000000000000000000000000000000000007");

/// The ECPairing precompile address.
pub const EC_PAIRING: Address = address!("0x0000000000000000000000000000000000000008");

/// The Blake2F precompile address.
pub const BLAKE_2F: Address = address!("0x0000000000000000000000000000000000000009");

/// The PointEvaluation precompile address.
pub const POINT_EVALUATION: Address = address!("0x000000000000000000000000000000000000000a");

/// The BLS12-381 G1ADD precompile address.
pub const BLS12_G1ADD: Address = address!("0x000000000000000000000000000000000000000b");

/// The BLS12-381 G1MSM precompile address.
pub const BLS12_G1MSM: Address = address!("0x000000000000000000000000000000000000000c");

/// The BLS12-381 G2ADD precompile address.
pub const BLS12_G2ADD: Address = address!("0x000000000000000000000000000000000000000d");

/// The BLS12-381 G2MSM precompile address.
pub const BLS12_G2MSM: Address = address!("0x000000000000000000000000000000000000000e");

/// The BLS12-381 pairing check precompile address.
pub const BLS12_PAIRING_CHECK: Address = address!("0x000000000000000000000000000000000000000f");

/// The BLS12-381 map Fp to G1 precompile address.
pub const BLS12_MAP_FP_TO_G1: Address = address!("0x0000000000000000000000000000000000000010");

/// The BLS12-381 map Fp2 to G2 precompile address.
pub const BLS12_MAP_FP2_TO_G2: Address = address!("0x0000000000000000000000000000000000000011");

/// The P256VERIFY precompile address.
pub const P256_VERIFY: Address = address!("0x0000000000000000000000000000000000000100");

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
