use ethers::prelude::{Address, BlockNumber, TxHash};
use serde::{
    de::DeserializeOwned, ser::SerializeSeq, Deserialize, Deserializer, Serialize, Serializer,
};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "method", content = "params")]
pub enum JsonRpcMethods {
    #[serde(rename = "eth_getBalance")]
    EthGetBalance(Address, BlockNumber),

    #[serde(
        rename = "eth_getTransactionByHash",
        serialize_with = "ser_into_sequence",
        deserialize_with = "de_from_sequence"
    )]
    EthGetTransactionByHash(TxHash),
}

fn ser_into_sequence<S, T>(val: &T, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    let mut seq = s.serialize_seq(Some(1))?;
    seq.serialize_element(val)?;
    seq.end()
}

fn de_from_sequence<'de, T, D>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let mut seq = Vec::<T>::deserialize(d)?;
    assert!(seq.len() == 1);
    Ok(seq.pop().expect("length of vector is 1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use rand::Rng;

    #[test]
    fn test_serde() {
        let mut rng = rand::thread_rng();

        let val = JsonRpcMethods::EthGetBalance(
            Address::random(),
            BlockNumber::Number(rng.gen::<u64>().into()),
        );
        let ser = serde_json::to_string(&val).unwrap();
        let de_val: JsonRpcMethods = serde_json::from_str(&ser).unwrap();
        assert_eq!(de_val, val);

        let val = JsonRpcMethods::EthGetTransactionByHash(TxHash::random());
        let ser = serde_json::to_string(&val).unwrap();
        let de_val: JsonRpcMethods = serde_json::from_str(&ser).unwrap();
        assert_eq!(de_val, val);
    }
}
