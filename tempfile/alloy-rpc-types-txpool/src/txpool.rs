//! Types for the `txpool` namespace: <https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-txpool>

use alloy_primitives::{Address, U256};
use alloy_rpc_types_eth::{Transaction, TransactionTrait};
use serde::{
    de::{self, Deserializer, Visitor},
    Deserialize, Serialize,
};
use std::{collections::BTreeMap, fmt, str::FromStr};

/// Transaction summary as found in the Txpool Inspection property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxpoolInspectSummary {
    /// Recipient (None when contract creation)
    pub to: Option<Address>,
    /// Transferred value
    pub value: U256,
    /// Gas amount
    pub gas: u64,
    /// Gas Price
    pub gas_price: u128,
}

impl TxpoolInspectSummary {
    /// Extracts the [`TxpoolInspectSummary`] from a transaction.
    pub fn from_tx<T: TransactionTrait>(tx: T) -> Self {
        Self {
            to: tx.to(),
            value: tx.value(),
            gas: tx.gas_limit(),
            gas_price: tx.max_fee_per_gas(),
        }
    }
}

impl<T: TransactionTrait> From<T> for TxpoolInspectSummary {
    fn from(value: T) -> Self {
        Self::from_tx(value)
    }
}

/// Visitor struct for TxpoolInspectSummary.
struct TxpoolInspectSummaryVisitor;

/// Walk through the deserializer to parse a txpool inspection summary into the
/// `TxpoolInspectSummary` struct.
impl Visitor<'_> for TxpoolInspectSummaryVisitor {
    type Value = TxpoolInspectSummary;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("to: value wei + gasLimit gas × gas_price wei")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let addr_split: Vec<&str> = value.split(": ").collect();
        if addr_split.len() != 2 {
            return Err(de::Error::custom("invalid format for TxpoolInspectSummary: to"));
        }
        let value_split: Vec<&str> = addr_split[1].split(" wei + ").collect();
        if value_split.len() != 2 {
            return Err(de::Error::custom("invalid format for TxpoolInspectSummary: gasLimit"));
        }
        let gas_split: Vec<&str> = value_split[1].split(" gas × ").collect();
        if gas_split.len() != 2 {
            return Err(de::Error::custom("invalid format for TxpoolInspectSummary: gas"));
        }
        let gas_price_split: Vec<&str> = gas_split[1].split(" wei").collect();
        if gas_price_split.len() != 2 {
            return Err(de::Error::custom("invalid format for TxpoolInspectSummary: gas_price"));
        }
        let to = match addr_split[0] {
            "" | "0x" | "contract creation" => None,
            addr => {
                Some(Address::from_str(addr.trim_start_matches("0x")).map_err(de::Error::custom)?)
            }
        };
        let value = U256::from_str(value_split[0]).map_err(de::Error::custom)?;
        let gas = u64::from_str(gas_split[0]).map_err(de::Error::custom)?;
        let gas_price = u128::from_str(gas_price_split[0]).map_err(de::Error::custom)?;

        Ok(TxpoolInspectSummary { to, value, gas, gas_price })
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(&value)
    }
}

/// Implement the `Deserialize` trait for `TxpoolInspectSummary` struct.
impl<'de> Deserialize<'de> for TxpoolInspectSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(TxpoolInspectSummaryVisitor)
    }
}

/// Implement the `Serialize` trait for `TxpoolInspectSummary` struct so that the
/// format matches the one from geth.
impl Serialize for TxpoolInspectSummary {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let formatted_to =
            self.to.map_or_else(|| "contract creation".to_string(), |to| format!("{to:?}"));
        let formatted = format!(
            "{}: {} wei + {} gas × {} wei",
            formatted_to, self.value, self.gas, self.gas_price
        );
        serializer.serialize_str(&formatted)
    }
}

/// Transaction Pool Content
///
/// The content inspection property can be queried to list the exact details of all
/// the transactions currently pending for inclusion in the next block(s), as well
/// as the ones that are being scheduled for future execution only.
///
/// See [here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_content) for more details
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxpoolContent<T = Transaction> {
    /// pending tx
    pub pending: BTreeMap<Address, BTreeMap<String, T>>,
    /// queued tx
    pub queued: BTreeMap<Address, BTreeMap<String, T>>,
}

impl<T> Default for TxpoolContent<T> {
    fn default() -> Self {
        Self { pending: BTreeMap::new(), queued: BTreeMap::new() }
    }
}

impl<T> TxpoolContent<T> {
    /// Removes the transactions from the given sender
    pub fn remove_from(&mut self, sender: &Address) -> TxpoolContentFrom<T> {
        TxpoolContentFrom {
            pending: self.pending.remove(sender).unwrap_or_default(),
            queued: self.queued.remove(sender).unwrap_or_default(),
        }
    }
}

/// Transaction Pool Content From
///
/// Same as [TxpoolContent] but for a specific address.
///
/// See [here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_contentFrom) for more details
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxpoolContentFrom<T = Transaction> {
    /// pending tx
    pub pending: BTreeMap<String, T>,
    /// queued tx
    pub queued: BTreeMap<String, T>,
}

impl<T> Default for TxpoolContentFrom<T> {
    fn default() -> Self {
        Self { pending: BTreeMap::new(), queued: BTreeMap::new() }
    }
}

/// Transaction Pool Inspect
///
/// The inspect inspection property can be queried to list a textual summary
/// of all the transactions currently pending for inclusion in the next block(s),
/// as well as the ones that are being scheduled for future execution only.
/// This is a method specifically tailored to developers to quickly see the
/// transactions in the pool and find any potential issues.
///
/// See [here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_inspect) for more details
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxpoolInspect {
    /// pending tx
    pub pending: BTreeMap<Address, BTreeMap<String, TxpoolInspectSummary>>,
    /// queued tx
    pub queued: BTreeMap<Address, BTreeMap<String, TxpoolInspectSummary>>,
}

/// Transaction Pool Status
///
/// The status inspection property can be queried for the number of transactions
/// currently pending for inclusion in the next block(s), as well as the ones that
/// are being scheduled for future execution only.
///
/// See [here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_status) for more details
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxpoolStatus {
    /// number of pending tx
    #[serde(with = "alloy_serde::quantity")]
    pub pending: u64,
    /// number of queued tx
    #[serde(with = "alloy_serde::quantity")]
    pub queued: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;

    #[test]
    fn serde_txpool_content() {
        // Gathered from geth v1.10.23-stable-d901d853/linux-amd64/go1.18.5
        // Addresses used as keys in `pending` and `queued` have been manually lowercased to
        // simplify the test.
        let txpool_content_json = r#"
{
  "pending": {
    "0x00000000863b56a3c1f0f1be8bc4f8b7bd78f57a": {
      "29": {
        "blockHash": null,
        "blockNumber": null,
        "from": "0x00000000863b56a3c1f0f1be8bc4f8b7bd78f57a",
        "gas": "0x2af9e",
        "maxFeePerGas": "0x218711a00",
        "maxPriorityFeePerGas": "0x3b9aca00",
        "hash": "0xfbc6fd04ba1c4114f06574263f04099b4fb2da72acc6f9709f0a3d2361308344",
        "input": "0x5ae401dc00000000000000000000000000000000000000000000000000000000636c757700000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000e404e45aaf000000000000000000000000a0b86991c6218b36c1d19d4a2e9eb0ce3606eb480000000000000000000000006b175474e89094c44da98b954eedeac495271d0f000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000863b56a3c1f0f1be8bc4f8b7bd78f57a000000000000000000000000000000000000000000000000000000007781df4000000000000000000000000000000000000000000000006c240454bf9c87cd84000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "nonce": "0x1d",
        "to": "0x68b3465833fb72a70ecdf485e0e4c7bd8665fc45",
        "transactionIndex": null,
        "value": "0x0",
        "type": "0x2",
        "v": "0x0",
        "accessList": [],
        "chainId": "0x1",
        "yParity": "0x0",
        "r": "0xbb809ae71b03319ba2811ebd581c85665169143ffade86e07d2eb4cd03b544dc",
        "s": "0x65a2aa7e0e70356f765205a611d580de8e84fa79086f117fd9ab4765f5cf1339"
      }
    },
    "0x000042429c09de5881f05a0c2a068222f4f5b091": {
      "38": {
        "blockHash": null,
        "blockNumber": null,
        "from": "0x000042429c09de5881f05a0c2a068222f4f5b091",
        "gas": "0x61a80",
        "gasPrice": "0x2540be400",
        "hash": "0x054ad1ccf5917139a9b1952f62048f742255a7c11100f593c4f18c1ed49b8dfd",
        "input": "0x27dc297e800332e506f28f49a13c1edf087bdd6482d6cb3abdf2a4c455642aef1e98fc240000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000002d7b22444149223a313439332e37342c2254555344223a313438392e36362c2255534443223a313439322e34387d00000000000000000000000000000000000000",
        "nonce": "0x26",
        "to": "0xabd279febe00c93fb0c9e683c6919ec4f107241f",
        "transactionIndex": null,
        "value": "0x0",
        "type": "0x0",
        "chainId": "0x1",
        "v": "0x25",
        "r": "0xaf46b2c0f067f7d1d63ac19daa349c0e1eb83f019ee00542ffa7095e05352e92",
        "s": "0x21d6d24d58ec361379ffffe4cc17bec8ce2b9f5f9759a91afc9a54dfdfa519c2"
      }
    },
    "0x000fab888651fbceb55de230493562159ead0340": {
      "12": {
        "blockHash": null,
        "blockNumber": null,
        "from": "0x000fab888651fbceb55de230493562159ead0340",
        "gas": "0x12fed",
        "maxFeePerGas": "0x1a13b8600",
        "maxPriorityFeePerGas": "0x59682f00",
        "hash": "0xfae0cffdae6774abe11662a2cdbea019fce48fca87ba9ebf5e9e7c2454c01715",
        "input": "0xa9059cbb00000000000000000000000050272a56ef9aff7238e8b40347da62e87c1f69e200000000000000000000000000000000000000000000000000000000428d3dfc",
        "nonce": "0xc",
        "to": "0x8e8d6ab093905c400d583efd37fbeeb1ee1c0c39",
        "transactionIndex": null,
        "value": "0x0",
        "type": "0x2",
        "v": "0x0",
        "accessList": [],
        "chainId": "0x1",
        "yParity": "0x0",
        "r": "0x7b717e689d1bd045ee7afd79b97219f2e36bd22a6a14e07023902194bca96fbf",
        "s": "0x7b0ba462c98e7b0f95a53f047cf568ee0443839628dfe4ab294bfab88fa8e251"
      }
    }
  },
  "queued": {
    "0x00b846f07f5e7c61569437ca16f88a9dfa00f1bf": {
      "143": {
        "blockHash": null,
        "blockNumber": null,
        "from": "0x00b846f07f5e7c61569437ca16f88a9dfa00f1bf",
        "gas": "0x33c3b",
        "maxFeePerGas": "0x218711a00",
        "maxPriorityFeePerGas": "0x77359400",
        "hash": "0x68959706857f7a58d752ede0a5118a5f55f4ae40801f31377e1af201944720b2",
        "input": "0x03a9ea6d00000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000f2ff840000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000041d0c4694374d7893d63605625687be2f01028a5b49eca00f72901e773ad8ba7906e58d43e114a28353efaf8abd6a2675de83a3a07af579b8b268e6b714376610d1c00000000000000000000000000000000000000000000000000000000000000",
        "nonce": "0x8f",
        "to": "0xfbddadd80fe7bda00b901fbaf73803f2238ae655",
        "transactionIndex": null,
        "value": "0x1f58a57c1794eb",
        "type": "0x2",
        "v": "0x0",
        "accessList": [],
        "chainId": "0x1",
        "yParity": "0x0",
        "r": "0x77d149add2b1b84af9408af55661b05b21e2a436f9bfcaa844584905a0f8f1ac",
        "s": "0x358d79063d702f0c3fb46ad0f6ce5db61f5fdb0b20359c8da2e72a11988db283"
      }
    },
    "0x025276ec2de8ee570cfd4c1010319f14a6d9f0dd": {
      "1": {
        "blockHash": null,
        "blockNumber": null,
        "from": "0x025276ec2de8ee570cfd4c1010319f14a6d9f0dd",
        "gas": "0x7918",
        "maxFeePerGas": "0x12e531724e",
        "maxPriorityFeePerGas": "0x59682f00",
        "hash": "0x35109918ab6129a4d69480514ebec0ea08dc4a4de032fec59003ea66718828c4",
        "input": "0x",
        "nonce": "0x1",
        "to": "0x025276ec2de8ee570cfd4c1010319f14a6d9f0dd",
        "transactionIndex": null,
        "value": "0x0",
        "type": "0x2",
        "v": "0x0",
        "accessList": [],
        "chainId": "0x1",
        "yParity": "0x0",
        "r": "0x863ed0413a14f3f1695fd9728f1500a2b46e69d6f4c82408af15354cc5a667d6",
        "s": "0x2d503050aa1c9ecbb6df9957459c296f2f6190bc07aa09047d541233100b1c7a"
      },
      "4": {
        "blockHash": null,
        "blockNumber": null,
        "from": "0x025276ec2de8ee570cfd4c1010319f14a6d9f0dd",
        "gas": "0x7530",
        "maxFeePerGas": "0x1919617600",
        "maxPriorityFeePerGas": "0x5c7261c0",
        "hash": "0xa58e54464b2ca62a5e2d976604ed9a53b13f8823a170ee4c3ae0cd91cde2a6c5",
        "input": "0x",
        "nonce": "0x4",
        "to": "0x025276ec2de8ee570cfd4c1010319f14a6d9f0dd",
        "transactionIndex": null,
        "value": "0x0",
        "type": "0x2",
        "v": "0x1",
        "accessList": [],
        "chainId": "0x1",
        "yParity": "0x1",
        "r": "0xb6a571191c4b5b667876295571c42c9411bbb4569eea1a6ad149572e4efc55a9",
        "s": "0x248a72dab9b24568dd9cbe289c205eaba1a6b58b32b5a96c48554945d3fd0d86"
      }
    },
    "0x02666081cfb787de3562efbbca5f0fe890e927f1": {
      "44": {
        "blockHash": null,
        "blockNumber": null,
        "from": "0x02666081cfb787de3562efbbca5f0fe890e927f1",
        "gas": "0x16404",
        "maxFeePerGas": "0x4bad00695",
        "maxPriorityFeePerGas": "0xa3e9ab80",
        "hash": "0xf627e59d7a59eb650f4c9df222858572601a566263809fdacbb755ac2277a4a7",
        "input": "0x095ea7b300000000000000000000000029fbd00940df70cfc5dad3f2370686991e2bbf5cffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "nonce": "0x2c",
        "to": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
        "transactionIndex": null,
        "value": "0x0",
        "type": "0x2",
        "v": "0x0",
        "accessList": [],
        "chainId": "0x1",
        "yParity": "0x0",
        "r": "0xcfc88f55fc0779d12705acba58719cd7d0ed5b0c1a7c3c3682b56397ca493dd5",
        "s": "0x7e7dc008058c543ebfdae67154c797639447db5e8006f8fc0585352d857c1b6c"
      }
    }
  }
}"#;
        let deserialized: TxpoolContent = serde_json::from_str(txpool_content_json).unwrap();
        let serialized: String = serde_json::to_string_pretty(&deserialized).unwrap();

        let origin: serde_json::Value = serde_json::from_str(txpool_content_json).unwrap();
        let serialized_value = serde_json::to_value(deserialized.clone()).unwrap();
        assert_eq!(origin, serialized_value);
        assert_eq!(deserialized, serde_json::from_str::<TxpoolContent>(&serialized).unwrap());
    }

    #[test]
    fn serde_txpool_inspect() {
        let txpool_inspect_json = r#"
{
  "pending": {
    "0x0512261a7486b1e29704ac49a5eb355b6fd86872": {
      "124930": "0x000000000000000000000000000000000000007E: 0 wei + 100187 gas × 20000000000 wei"
    },
    "0x201354729f8d0f8b64e9a0c353c672c6a66b3857": {
      "252350": "0xd10e3Be2bc8f959Bc8C41CF65F60dE721cF89ADF: 0 wei + 65792 gas × 2000000000 wei",
      "252351": "0xd10e3Be2bc8f959Bc8C41CF65F60dE721cF89ADF: 0 wei + 65792 gas × 2000000000 wei",
      "252352": "0xd10e3Be2bc8f959Bc8C41CF65F60dE721cF89ADF: 0 wei + 65780 gas × 2000000000 wei",
      "252353": "0xd10e3Be2bc8f959Bc8C41CF65F60dE721cF89ADF: 0 wei + 65780 gas × 2000000000 wei"
    },
    "0x00000000863B56a3C1f0F1be8BC4F8b7BD78F57a": {
      "40": "contract creation: 0 wei + 612412 gas × 6000000000 wei"
    }
  },
  "queued": {
    "0x0f87ffcd71859233eb259f42b236c8e9873444e3": {
      "7": "0x3479BE69e07E838D9738a301Bb0c89e8EA2Bef4a: 1000000000000000 wei + 21000 gas × 10000000000 wei",
      "8": "0x73Aaf691bc33fe38f86260338EF88f9897eCaa4F: 1000000000000000 wei + 21000 gas × 10000000000 wei"
    },
    "0x307e8f249bcccfa5b245449256c5d7e6e079943e": {
      "3": "0x73Aaf691bc33fe38f86260338EF88f9897eCaa4F: 10000000000000000 wei + 21000 gas × 10000000000 wei"
    }
  }
}"#;
        let deserialized: TxpoolInspect = serde_json::from_str(txpool_inspect_json).unwrap();
        assert_eq!(deserialized, expected_txpool_inspect());

        let serialized = serde_json::to_string(&deserialized).unwrap();
        let deserialized2: TxpoolInspect = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized2, deserialized);
    }

    #[test]
    fn serde_txpool_status() {
        let txpool_status_json = r#"
{
  "pending": "0x23",
  "queued": "0x20"
}"#;
        let deserialized: TxpoolStatus = serde_json::from_str(txpool_status_json).unwrap();
        let serialized: String = serde_json::to_string_pretty(&deserialized).unwrap();
        assert_eq!(txpool_status_json.trim(), serialized);
    }

    fn expected_txpool_inspect() -> TxpoolInspect {
        let mut pending_map = BTreeMap::new();
        let mut pending_map_inner = BTreeMap::new();
        pending_map_inner.insert(
            "124930".to_string(),
            TxpoolInspectSummary {
                to: Some(Address::from_str("000000000000000000000000000000000000007E").unwrap()),
                value: U256::from(0u128),
                gas: 100187,
                gas_price: 20000000000u128,
            },
        );
        pending_map.insert(
            Address::from_str("0512261a7486b1e29704ac49a5eb355b6fd86872").unwrap(),
            pending_map_inner.clone(),
        );
        pending_map_inner.clear();
        pending_map_inner.insert(
            "252350".to_string(),
            TxpoolInspectSummary {
                to: Some(Address::from_str("d10e3Be2bc8f959Bc8C41CF65F60dE721cF89ADF").unwrap()),
                value: U256::from(0u128),
                gas: 65792,
                gas_price: 2000000000u128,
            },
        );
        pending_map_inner.insert(
            "252351".to_string(),
            TxpoolInspectSummary {
                to: Some(Address::from_str("d10e3Be2bc8f959Bc8C41CF65F60dE721cF89ADF").unwrap()),
                value: U256::from(0u128),
                gas: 65792,
                gas_price: 2000000000u128,
            },
        );
        pending_map_inner.insert(
            "252352".to_string(),
            TxpoolInspectSummary {
                to: Some(Address::from_str("d10e3Be2bc8f959Bc8C41CF65F60dE721cF89ADF").unwrap()),
                value: U256::from(0u128),
                gas: 65780,
                gas_price: 2000000000u128,
            },
        );
        pending_map_inner.insert(
            "252353".to_string(),
            TxpoolInspectSummary {
                to: Some(Address::from_str("d10e3Be2bc8f959Bc8C41CF65F60dE721cF89ADF").unwrap()),
                value: U256::from(0u128),
                gas: 65780,
                gas_price: 2000000000u128,
            },
        );
        pending_map.insert(
            Address::from_str("201354729f8d0f8b64e9a0c353c672c6a66b3857").unwrap(),
            pending_map_inner.clone(),
        );
        pending_map_inner.clear();
        pending_map_inner.insert(
            "40".to_string(),
            TxpoolInspectSummary {
                to: None,
                value: U256::from(0u128),
                gas: 612412,
                gas_price: 6000000000u128,
            },
        );
        pending_map.insert(
            Address::from_str("00000000863B56a3C1f0F1be8BC4F8b7BD78F57a").unwrap(),
            pending_map_inner,
        );
        let mut queued_map = BTreeMap::new();
        let mut queued_map_inner = BTreeMap::new();
        queued_map_inner.insert(
            "7".to_string(),
            TxpoolInspectSummary {
                to: Some(Address::from_str("3479BE69e07E838D9738a301Bb0c89e8EA2Bef4a").unwrap()),
                value: U256::from(1000000000000000u128),
                gas: 21000,
                gas_price: 10000000000u128,
            },
        );
        queued_map_inner.insert(
            "8".to_string(),
            TxpoolInspectSummary {
                to: Some(Address::from_str("73Aaf691bc33fe38f86260338EF88f9897eCaa4F").unwrap()),
                value: U256::from(1000000000000000u128),
                gas: 21000,
                gas_price: 10000000000u128,
            },
        );
        queued_map.insert(
            Address::from_str("0f87ffcd71859233eb259f42b236c8e9873444e3").unwrap(),
            queued_map_inner.clone(),
        );
        queued_map_inner.clear();
        queued_map_inner.insert(
            "3".to_string(),
            TxpoolInspectSummary {
                to: Some(Address::from_str("73Aaf691bc33fe38f86260338EF88f9897eCaa4F").unwrap()),
                value: U256::from(10000000000000000u128),
                gas: 21000,
                gas_price: 10000000000u128,
            },
        );
        queued_map.insert(
            Address::from_str("307e8f249bcccfa5b245449256c5d7e6e079943e").unwrap(),
            queued_map_inner,
        );

        TxpoolInspect { pending: pending_map, queued: queued_map }
    }
}
