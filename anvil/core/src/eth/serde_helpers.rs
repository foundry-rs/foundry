//! custom serde helper functions

use ethers_core::types::{BlockNumber, U256};
use serde::{Deserialize, Deserializer};

#[derive(Deserialize)]
#[serde(untagged)]
enum Numeric {
    U256(U256),
    Num(u64),
}

impl From<Numeric> for U256 {
    fn from(n: Numeric) -> U256 {
        match n {
            Numeric::U256(n) => n,
            Numeric::Num(n) => U256::from(n),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum NumericSeq {
    Seq([Numeric; 1]),
    U256(U256),
    Num(u64),
}

/// Deserializes single integer params: `1, [1], ["0x01"]`
pub fn deserialize_number_seq<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    let num = match NumericSeq::deserialize(deserializer)? {
        NumericSeq::Seq(seq) => seq.into_iter().next().unwrap().into(),
        NumericSeq::U256(n) => n,
        NumericSeq::Num(n) => U256::from(n),
    };

    Ok(num)
}

/// Deserializes a number from hex or int
pub fn deserialize_number<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    Numeric::deserialize(deserializer).map(Into::into)
}

/// Deserializes a number from hex or int, but optionally
pub fn deserialize_number_opt<'de, D>(deserializer: D) -> Result<Option<U256>, D::Error>
where
    D: Deserializer<'de>,
{
    let num = match Option::<Numeric>::deserialize(deserializer)? {
        Some(Numeric::U256(n)) => Some(n),
        Some(Numeric::Num(n)) => Some(U256::from(n)),
        _ => None,
    };

    Ok(num)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LenientBlockNumber {
    BlockNumber(BlockNumber),
    Num(u64),
}

impl From<LenientBlockNumber> for BlockNumber {
    fn from(b: LenientBlockNumber) -> Self {
        match b {
            LenientBlockNumber::BlockNumber(b) => b,
            LenientBlockNumber::Num(b) => b.into(),
        }
    }
}

/// Following the spec the block parameter is either:
///
/// > HEX String - an integer block number
/// > String "earliest" for the earliest/genesis block
/// > String "latest" - for the latest mined block
/// > String "pending" - for the pending state/transactions
///
/// and with EIP-1898:
/// > blockNumber: QUANTITY - a block number
/// > blockHash: DATA - a block hash
///
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1898.md
///
/// EIP-1898 does not all calls that use `BlockNumber` like `eth_getBlockByNumber` and doesn't list
/// raw integers as supported.
///
/// However, there are dev node implementations that support integers, such as ganache: <https://github.com/foundry-rs/foundry/issues/1868>
///
/// N.B.: geth does not support ints in `eth_getBlockByNumber`
pub fn lenient_block_number<'de, D>(deserializer: D) -> Result<BlockNumber, D::Error>
where
    D: Deserializer<'de>,
{
    LenientBlockNumber::deserialize(deserializer).map(Into::into)
}

/// Same as `lenient_block_number` but requires to be `[num; 1]`
pub fn lenient_block_number_seq<'de, D>(deserializer: D) -> Result<BlockNumber, D::Error>
where
    D: Deserializer<'de>,
{
    let num =
        <[LenientBlockNumber; 1]>::deserialize(deserializer)?.into_iter().next().unwrap().into();
    Ok(num)
}

/// Wrapper type that ensures the type is named `params`
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Params<T> {
    pub params: T,
}

#[allow(unused)]
pub mod sequence {
    use serde::{
        de::DeserializeOwned, ser::SerializeSeq, Deserialize, Deserializer, Serialize, Serializer,
    };

    #[allow(unused)]
    pub fn serialize<S, T>(val: &T, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        let mut seq = s.serialize_seq(Some(1))?;
        seq.serialize_element(val)?;
        seq.end()
    }

    pub fn deserialize<'de, T, D>(d: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let mut seq = Vec::<T>::deserialize(d)?;
        if seq.len() != 1 {
            return Err(serde::de::Error::custom(format!(
                "expected params sequence with length 1 but got {}",
                seq.len()
            )))
        }
        Ok(seq.remove(0))
    }
}

/// A module that deserializes `[]` optionally
pub mod empty_params {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(d: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        let seq = Option::<Vec<()>>::deserialize(d)?.unwrap_or_default();
        if !seq.is_empty() {
            return Err(serde::de::Error::custom(format!(
                "expected params sequence with length 0 but got {}",
                seq.len()
            )))
        }
        Ok(())
    }
}
