use crate::{utils::parse_units, Client, EtherscanError, Response, Result};
use alloy_primitives::U256;
use serde::{de, Deserialize, Deserializer};
use std::{collections::HashMap, str::FromStr};

const WEI_PER_GWEI: u64 = 1_000_000_000;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct GasOracle {
    /// Safe Gas Price in wei
    #[serde(deserialize_with = "deser_gwei_amount")]
    pub safe_gas_price: U256,
    /// Propose Gas Price in wei
    #[serde(deserialize_with = "deser_gwei_amount")]
    pub propose_gas_price: U256,
    /// Fast Gas Price in wei
    #[serde(deserialize_with = "deser_gwei_amount")]
    pub fast_gas_price: U256,
    /// Last Block
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub last_block: u64,
    /// Suggested Base Fee in wei
    #[serde(deserialize_with = "deser_gwei_amount")]
    #[serde(rename = "suggestBaseFee")]
    pub suggested_base_fee: U256,
    /// Gas Used Ratio
    #[serde(deserialize_with = "deserialize_f64_vec")]
    #[serde(rename = "gasUsedRatio")]
    pub gas_used_ratio: Vec<f64>,
}

// This function is used to deserialize a string or number into a U256 with an
// amount of gwei. If the contents is a number, deserialize it. If the contents
// is a string, attempt to deser as first a decimal f64 then a decimal U256.
fn deser_gwei_amount<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt {
        Number(u64),
        String(String),
    }

    match StringOrInt::deserialize(deserializer)? {
        StringOrInt::Number(i) => Ok(U256::from(i) * U256::from(WEI_PER_GWEI)),
        StringOrInt::String(s) => {
            parse_units(s, "gwei").map(Into::into).map_err(serde::de::Error::custom)
        }
    }
}

fn deserialize_number_from_string<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + serde::Deserialize<'de>,
    <T as FromStr>::Err: std::fmt::Display,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt<T> {
        String(String),
        Number(T),
    }

    match StringOrInt::<T>::deserialize(deserializer)? {
        StringOrInt::String(s) => s.parse::<T>().map_err(serde::de::Error::custom),
        StringOrInt::Number(i) => Ok(i),
    }
}

fn deserialize_f64_vec<'de, D>(deserializer: D) -> core::result::Result<Vec<f64>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let str_sequence = String::deserialize(deserializer)?;
    str_sequence
        .split(',')
        .map(|item| f64::from_str(item).map_err(|err| de::Error::custom(err.to_string())))
        .collect()
}

impl Client {
    /// Returns the estimated time, in seconds, for a transaction to be confirmed on the blockchain
    /// for the specified gas price
    pub async fn gas_estimate(&self, gas_price: U256) -> Result<u32> {
        let query = self.create_query(
            "gastracker",
            "gasestimate",
            HashMap::from([("gasprice", gas_price.to_string())]),
        );
        let response: Response<String> = self.get_json(&query).await?;

        if response.status == "1" {
            Ok(u32::from_str(&response.result).map_err(|_| EtherscanError::GasEstimationFailed)?)
        } else {
            Err(EtherscanError::GasEstimationFailed)
        }
    }

    /// Returns the current Safe, Proposed and Fast gas prices
    /// Post EIP-1559 changes:
    /// - Safe/Proposed/Fast gas price recommendations are now modeled as Priority Fees.
    /// - New field `suggestBaseFee`, the baseFee of the next pending block
    /// - New field `gasUsedRatio`, to estimate how busy the network is
    pub async fn gas_oracle(&self) -> Result<GasOracle> {
        let query = self.create_query("gastracker", "gasoracle", serde_json::Value::Null);
        let response: Response<GasOracle> = self.get_json(&query).await?;

        Ok(response.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_works() {
        // Response from Polygon mainnet at 2023-04-05
        let v = r#"{
            "status": "1",
            "message": "OK",
            "result": {
                "LastBlock": "41171167",
                "SafeGasPrice": "119.9",
                "ProposeGasPrice": "141.9",
                "FastGasPrice": "142.9",
                "suggestBaseFee": "89.82627877",
                "gasUsedRatio": "0.399191166666667,0.4847166,0.997667533333333,0.538075133333333,0.343416033333333",
                "UsdPrice": "1.15"
            }
        }"#;
        let gas_oracle: Response<GasOracle> = serde_json::from_str(v).unwrap();
        assert_eq!(gas_oracle.message, "OK");
        assert_eq!(
            gas_oracle.result.propose_gas_price,
            parse_units("141.9", "gwei").unwrap().into()
        );

        let v = r#"{
            "status":"1",
            "message":"OK",
            "result":{
               "LastBlock":"13053741",
               "SafeGasPrice":"20",
               "ProposeGasPrice":"22",
               "FastGasPrice":"24",
               "suggestBaseFee":"19.230609716",
               "gasUsedRatio":"0.370119078777807,0.8954731,0.550911766666667,0.212457033333333,0.552463633333333"
            }
        }"#;
        let gas_oracle: Response<GasOracle> = serde_json::from_str(v).unwrap();
        assert_eq!(gas_oracle.message, "OK");
        assert_eq!(gas_oracle.result.propose_gas_price, parse_units(22, "gwei").unwrap().into());

        // remove quotes around integers
        let v = r#"{
            "status":"1",
            "message":"OK",
            "result":{
               "LastBlock":13053741,
               "SafeGasPrice":20,
               "ProposeGasPrice":22,
               "FastGasPrice":24,
               "suggestBaseFee":"19.230609716",
               "gasUsedRatio":"0.370119078777807,0.8954731,0.550911766666667,0.212457033333333,0.552463633333333"
            }
        }"#;
        let gas_oracle: Response<GasOracle> = serde_json::from_str(v).unwrap();
        assert_eq!(gas_oracle.message, "OK");
        assert_eq!(gas_oracle.result.propose_gas_price, parse_units(22, "gwei").unwrap().into());
    }
}
