//! Type aliases for common primitive types.

use crate::{FixedBytes, Signed, Uint};

pub use ruint::aliases::{U0, U1, U1024, U2048, U320, U384, U4096, U448};

macro_rules! int_aliases {
    ($($unsigned:ident, $signed:ident<$BITS:literal, $LIMBS:literal>),* $(,)?) => {$(
        #[doc = concat!($BITS, "-bit [unsigned integer type][Uint], consisting of ", $LIMBS, ", 64-bit limbs.")]
        pub type $unsigned = Uint<$BITS, $LIMBS>;

        #[doc = concat!($BITS, "-bit [signed integer type][Signed], consisting of ", $LIMBS, ", 64-bit limbs.")]
        pub type $signed = Signed<$BITS, $LIMBS>;

        const _: () = assert!($LIMBS == ruint::nlimbs($BITS));
    )*};
}

/// The 0-bit signed integer type, capable of representing 0.
pub type I0 = Signed<0, 0>;

/// The 1-bit signed integer type, capable of representing 0 and -1.
pub type I1 = Signed<1, 1>;

int_aliases! {
      U8,   I8<  8, 1>,
     U16,  I16< 16, 1>,
     U24,  I24< 24, 1>,
     U32,  I32< 32, 1>,
     U40,  I40< 40, 1>,
     U48,  I48< 48, 1>,
     U56,  I56< 56, 1>,
     U64,  I64< 64, 1>,

     U72,  I72< 72, 2>,
     U80,  I80< 80, 2>,
     U88,  I88< 88, 2>,
     U96,  I96< 96, 2>,
    U104, I104<104, 2>,
    U112, I112<112, 2>,
    U120, I120<120, 2>,
    U128, I128<128, 2>,

    U136, I136<136, 3>,
    U144, I144<144, 3>,
    U152, I152<152, 3>,
    U160, I160<160, 3>,
    U168, I168<168, 3>,
    U176, I176<176, 3>,
    U184, I184<184, 3>,
    U192, I192<192, 3>,

    U200, I200<200, 4>,
    U208, I208<208, 4>,
    U216, I216<216, 4>,
    U224, I224<224, 4>,
    U232, I232<232, 4>,
    U240, I240<240, 4>,
    U248, I248<248, 4>,
    U256, I256<256, 4>,

    U512, I512<512, 8>,
}

macro_rules! fixed_bytes_aliases {
    ($($(#[$attr:meta])* $name:ident<$N:literal>),* $(,)?) => {$(
        #[doc = concat!($N, "-byte [fixed byte-array][FixedBytes] type.")]
        $(#[$attr])*
        pub type $name = FixedBytes<$N>;
    )*};
}

fixed_bytes_aliases! {
    B8<1>,
    B16<2>,
    B32<4>,
    B64<8>,
    B96<12>,
    B128<16>,
    /// See [`crate::B160`] as to why you likely want to use
    /// [`Address`](crate::Address) instead.
    #[doc(hidden)]
    B160<20>,
    B192<24>,
    B224<28>,
    B256<32>,
    B512<64>,
    B1024<128>,
    B2048<256>,
}

/// A block hash.
pub type BlockHash = B256;

/// A block number.
pub type BlockNumber = u64;

/// A block timestamp.
pub type BlockTimestamp = u64;

/// A transaction hash is a keccak hash of an RLP encoded signed transaction.
#[doc(alias = "TransactionHash")]
pub type TxHash = B256;

/// The sequence number of all existing transactions.
#[doc(alias = "TransactionNumber")]
pub type TxNumber = u64;

/// The nonce of a transaction.
#[doc(alias = "TransactionNonce")]
pub type TxNonce = u64;

/// The index of transaction in a block.
#[doc(alias = "TransactionIndex")]
pub type TxIndex = u64;

/// Chain identifier type (introduced in EIP-155).
pub type ChainId = u64;

/// An account storage key.
pub type StorageKey = B256;

/// An account storage value.
pub type StorageValue = U256;

/// Solidity contract functions are addressed using the first four bytes of the
/// Keccak-256 hash of their signature.
pub type Selector = FixedBytes<4>;
