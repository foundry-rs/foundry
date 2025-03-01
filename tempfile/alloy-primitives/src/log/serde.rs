//! This is an implementation of serde for Log for
//! both human-readable and binary forms.
//!
//! Ethereum JSON RPC requires logs in a flattened form.
//! However `serde(flatten)` breaks binary implementations.
//!
//! This module uses a trick to select a proxy for serde:
//! 1. LogFlattenSerializer for a human-readable (JSON) serializer,
//! 2. LogFlattenDeserializer for a human-readable (JSON) deserializer,
//! 3. LogUnflattenSerializer for a binary serializer,
//! 4. LogUnflattenDeserializer for a binary deserializer.

use crate::{Address, Log};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Serialize)]
#[serde(rename = "Log")]
struct LogFlattenSerializer<'a, T> {
    address: &'a Address,
    #[serde(flatten)]
    data: &'a T,
}

#[derive(Deserialize)]
#[serde(rename = "Log")]
struct LogFlattenDeserializer<T> {
    address: Address,
    #[serde(flatten)]
    data: T,
}

#[derive(Serialize)]
#[serde(rename = "Log")]
struct LogUnflattenSerializer<'a, T> {
    address: &'a Address,
    data: &'a T,
}

#[derive(Deserialize)]
#[serde(rename = "Log")]
struct LogUnflattenDeserializer<T> {
    address: Address,
    data: T,
}

impl<T: Serialize> Serialize for Log<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let Self { address, data } = self;
        if serializer.is_human_readable() {
            let replace = LogFlattenSerializer { address, data };
            replace.serialize(serializer)
        } else {
            let replace = LogUnflattenSerializer { address, data };
            replace.serialize(serializer)
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Log<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let LogFlattenDeserializer { address, data } = <_>::deserialize(deserializer)?;
            Ok(Self { address, data })
        } else {
            let LogUnflattenDeserializer { address, data } = <_>::deserialize(deserializer)?;
            Ok(Self { address, data })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        log::{Log, LogData},
        Bytes,
    };
    use alloc::vec::Vec;

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct TestStruct {
        logs: Vec<Log>,
    }

    fn gen_test_struct() -> TestStruct {
        // assume it's random:
        TestStruct {
            logs: vec![Log {
                address: address!("0x3100000000000000000000000000000000000001"),
                data: LogData::new(
                    vec![b256!("0x32eff959e2e8d1609edc4b39ccf75900aa6c1da5719f8432752963fdf008234f")],
                    Bytes::from_static(b"00000000000000000000000000000000000000000000000000000000000000021e9dbc1a11f8e046a72d1296cc2d8bb0d1544d56fd0b9bb8890a0f89b88036541e9dbc1a11f8e046a72d1296cc2d8bb0d1544d56fd0b9bb8890a0f89b8803654"),
                ).unwrap(),
            }],
        }
    }

    #[test]
    fn test_log_bincode_roundtrip() {
        let generated = gen_test_struct();

        let bytes = bincode::serialize(&generated).unwrap();
        let parsed: TestStruct = bincode::deserialize(&bytes).unwrap();
        assert_eq!(generated, parsed);
    }

    #[test]
    fn test_log_bcs_roundtrip() {
        let generated = gen_test_struct();

        let bytes = bcs::to_bytes(&generated).unwrap();
        let parsed: TestStruct = bcs::from_bytes(&bytes).unwrap();
        assert_eq!(generated, parsed);
    }

    #[test]
    fn test_log_json_roundtrip() {
        let expected = "{\"logs\":[{\"address\":\"0x3100000000000000000000000000000000000001\",\"topics\":[\"0x32eff959e2e8d1609edc4b39ccf75900aa6c1da5719f8432752963fdf008234f\"],\"data\":\"0x303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030323165396462633161313166386530343661373264313239366363326438626230643135343464353666643062396262383839306130663839623838303336353431653964626331613131663865303436613732643132393663633264386262306431353434643536666430623962623838393061306638396238383033363534\"}]}";

        let parsed: TestStruct = serde_json::from_str(expected).unwrap();
        let dumped = serde_json::to_string(&parsed).unwrap();

        assert_eq!(expected, dumped);
    }
}
