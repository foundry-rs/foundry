use ethers::prelude::{
    transaction::eip2718::TypedTransaction, Address, BlockNumber, Transaction, TxHash, U256,
};
use serde::{
    de::DeserializeOwned, ser::SerializeSeq, Deserialize, Deserializer, Serialize, Serializer,
};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "method", content = "params")]
pub enum EthRequest {
    #[serde(rename = "eth_getBalance")]
    EthGetBalance(Address, BlockNumber),

    #[serde(
        rename = "eth_getTransactionByHash",
        serialize_with = "ser_into_sequence",
        deserialize_with = "de_from_sequence"
    )]
    EthGetTransactionByHash(TxHash),

    #[serde(
        rename = "eth_sendTransaction",
        serialize_with = "ser_into_sequence",
        deserialize_with = "de_from_sequence"
    )]
    EthSendTransaction(TypedTransaction),
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

#[derive(Serialize)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum EthResponse {
    EthGetBalance(U256),
    EthGetTransactionByHash(Option<Transaction>),
    EthSendTransaction(Result<TxHash, Box<dyn erased_serde::Serialize>>),
}

#[cfg(test)]
mod tests {
    use super::*;

    use rand::Rng;

    #[test]
    fn test_serde_req() {
        let mut rng = rand::thread_rng();

        let val = EthRequest::EthGetBalance(
            Address::random(),
            BlockNumber::Number(rng.gen::<u64>().into()),
        );
        let ser = serde_json::to_string(&val).unwrap();
        let de_val: EthRequest = serde_json::from_str(&ser).unwrap();
        assert_eq!(de_val, val);

        let val = EthRequest::EthGetTransactionByHash(TxHash::random());
        let ser = serde_json::to_string(&val).unwrap();
        let de_val: EthRequest = serde_json::from_str(&ser).unwrap();
        assert_eq!(de_val, val);
    }

    #[test]
    fn test_serde_res() {
        let val = EthResponse::EthGetBalance(U256::from(123u64));
        let _ser = serde_json::to_string(&val).unwrap();

        let val = EthResponse::EthGetTransactionByHash(Some(Transaction::default()));
        let _ser = serde_json::to_string(&val).unwrap();
        let val = EthResponse::EthGetTransactionByHash(None);
        let _ser = serde_json::to_string(&val).unwrap();
    }
}
