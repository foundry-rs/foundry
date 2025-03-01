use crate::{
    eip712::{PropertyDef, Resolver},
    DynSolType, DynSolValue, Result,
};
use alloc::{collections::BTreeMap, string::String, vec::Vec};
use alloy_primitives::{keccak256, B256};
use alloy_sol_types::{Eip712Domain, SolStruct};
use derive_more::{Deref, DerefMut, From, Into, IntoIterator};
use parser::TypeSpecifier;
use serde::{Deserialize, Serialize};

/// Custom types for `TypedData`.
#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deref, DerefMut, From, Into, IntoIterator,
)]
pub struct Eip712Types(#[into_iterator(owned, ref, ref_mut)] BTreeMap<String, Vec<PropertyDef>>);

impl<'de> Deserialize<'de> for Eip712Types {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map: BTreeMap<String, Vec<PropertyDef>> = BTreeMap::deserialize(deserializer)?;

        for key in map.keys() {
            // ensure that all types are valid specifiers
            let _ = TypeSpecifier::parse(key).map_err(serde::de::Error::custom)?;
        }

        Ok(Self(map))
    }
}

/// Represents the [EIP-712](https://eips.ethereum.org/EIPS/eip-712) typed data
/// object.
///
/// Typed data is a JSON object containing type information, domain separator
/// parameters and the message object which has the following schema:
///
/// ```json
/// {
///     "type": "object",
///     "properties": {
///         "types": {
///             "type": "object",
///             "properties": {
///                 "EIP712Domain": { "type": "array" }
///             },
///             "additionalProperties": {
///                 "type": "array",
///                 "items": {
///                     "type": "object",
///                     "properties": {
///                         "name": { "type": "string" },
///                         "type": { "type": "string" }
///                     },
///                     "required": ["name", "type"]
///                 }
///             },
///             "required": ["EIP712Domain"]
///         },
///         "primaryType": { "type": "string" },
///         "domain": { "type": "object" },
///         "message": { "type": "object" }
///     },
///     "required": ["types", "primaryType", "domain", "message"]
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TypedData {
    /// Signing domain metadata. The signing domain is the intended context for
    /// the signature (e.g. the dapp, protocol, etc. that it's intended for).
    /// This data is used to construct the domain separator of the message.
    pub domain: Eip712Domain,

    /// The custom types used by this message.
    #[serde(rename = "types")]
    pub resolver: Resolver,

    /// The type of the message.
    #[serde(rename = "primaryType")]
    pub primary_type: String,

    /// The message to be signed.
    pub message: serde_json::Value,
}

/// `TypedData` is most likely going to be a stringified JSON object, so we have
/// to implement Deserialize manually to parse the string first.
///
/// See:
/// - Ethers.js: <https://github.com/ethers-io/ethers.js/blob/17969fe4169b44389dbd4da1dd85682eb3284d6f/src.ts/providers/provider-jsonrpc.ts#L415>
/// - Viem: <https://github.com/wagmi-dev/viem/blob/9aba19289832b22422e57265258fdf4beba83570/src/actions/wallet/signTypedData.ts#L178-L185>
impl<'de> Deserialize<'de> for TypedData {
    fn deserialize<D: serde::de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct TypedDataHelper {
            #[serde(default)]
            domain: Eip712Domain,
            types: Resolver,
            #[serde(rename = "primaryType")]
            primary_type: String,
            #[serde(default)]
            message: serde_json::Value,
        }

        #[allow(unknown_lints, non_local_definitions)]
        impl From<TypedDataHelper> for TypedData {
            fn from(value: TypedDataHelper) -> Self {
                Self {
                    domain: value.domain,
                    resolver: value.types,
                    primary_type: value.primary_type,
                    message: value.message,
                }
            }
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StrOrVal {
            Str(String),
            Val(TypedDataHelper),
        }

        match StrOrVal::deserialize(deserializer) {
            Ok(StrOrVal::Str(s)) => serde_json::from_str(&s).map_err(serde::de::Error::custom),
            Ok(StrOrVal::Val(v)) => Ok(v),
            Err(e) => Err(e),
        }
        .map(Into::into)
    }
}

impl TypedData {
    /// Instantiate [`TypedData`] from a [`SolStruct`] that implements
    /// [`serde::Serialize`].
    pub fn from_struct<S: SolStruct + Serialize>(s: &S, domain: Option<Eip712Domain>) -> Self {
        let mut resolver = Resolver::from_struct::<S>();
        let domain = domain.unwrap_or_default();
        resolver.ingest_string(domain.encode_type()).expect("domain string always valid");
        Self {
            domain,
            resolver,
            primary_type: S::NAME.into(),
            message: serde_json::to_value(s).unwrap(),
        }
    }

    /// Returns the domain for this typed data.
    pub const fn domain(&self) -> &Eip712Domain {
        &self.domain
    }

    fn resolve(&self) -> Result<DynSolType> {
        self.resolver.resolve(&self.primary_type)
    }

    /// Coerce the message to the type specified by `primary_type`, using the
    /// types map as a resolver.
    pub fn coerce(&self) -> Result<DynSolValue> {
        let ty = self.resolve()?;
        ty.coerce_json(&self.message)
    }

    /// Calculate the Keccak-256 hash of [`encodeType`] for this value.
    ///
    /// Fails if this type is not a struct.
    ///
    /// [`encodeType`]: https://eips.ethereum.org/EIPS/eip-712#definition-of-encodetype
    pub fn type_hash(&self) -> Result<B256> {
        self.encode_type().map(keccak256)
    }

    /// Calculate the [`hashStruct`] for this value.
    ///
    /// Fails if this type is not a struct.
    ///
    /// [`hashStruct`]: https://eips.ethereum.org/EIPS/eip-712#definition-of-hashstruct
    pub fn hash_struct(&self) -> Result<B256> {
        let mut type_hash = self.type_hash()?.to_vec();
        type_hash.extend(self.encode_data()?);
        Ok(keccak256(type_hash))
    }

    /// Calculate the [`encodeData`] for this value.
    ///
    /// Fails if this type is not a struct.
    ///
    /// [`encodeData`]: https://eips.ethereum.org/EIPS/eip-712#definition-of-encodedata
    pub fn encode_data(&self) -> Result<Vec<u8>> {
        let s = self.coerce()?;
        Ok(self.resolver.encode_data(&s)?.unwrap())
    }

    /// Calculate the [`encodeType`] for this value.
    ///
    /// Fails if this type is not a struct.
    ///
    /// [`encodeType`]: https://eips.ethereum.org/EIPS/eip-712#definition-of-encodetype
    pub fn encode_type(&self) -> Result<String> {
        self.resolver.encode_type(&self.primary_type)
    }

    /// Calculate the EIP-712 signing hash for this value.
    ///
    /// This is the hash of the magic bytes 0x1901 concatenated with the domain
    /// separator and the `hashStruct` result.
    pub fn eip712_signing_hash(&self) -> Result<B256> {
        let mut buf = [0u8; 66];
        buf[0] = 0x19;
        buf[1] = 0x01;
        buf[2..34].copy_from_slice(self.domain.separator().as_slice());

        // compatibility with <https://github.com/MetaMask/eth-sig-util>
        let len = if self.primary_type != "EIP712Domain" {
            buf[34..].copy_from_slice(self.hash_struct()?.as_slice());
            66
        } else {
            34
        };

        Ok(keccak256(&buf[..len]))
    }
}

// Adapted tests from https://github.com/MetaMask/eth-sig-util/blob/dd8bd0e1ca7ca3ed81631b279b8e3a63a2b16b7f/src/sign-typed-data.test.ts
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use alloc::string::ToString;
    use alloy_sol_types::sol;
    use serde_json::json;

    #[test]
    fn test_round_trip_ser() {
        let json = json!({
            "types": {
                "EIP712Domain": []
            },
            "primaryType": "EIP712Domain",
            "domain": {},
            "message": {}
        });
        let typed_data: TypedData = serde_json::from_value(json.clone()).unwrap();
        let val = serde_json::to_value(typed_data).unwrap();
        assert_eq!(val, json);
    }

    #[test]
    fn test_full_domain() {
        let json = json!({
            "types": {
                "EIP712Domain": [
                    {
                        "name": "name",
                        "type": "string"
                    },
                    {
                        "name": "version",
                        "type": "string"
                    },
                    {
                        "name": "chainId",
                        "type": "uint256"
                    },
                    {
                        "name": "verifyingContract",
                        "type": "address"
                    },
                    {
                        "name": "salt",
                        "type": "bytes32"
                    }
                ]
            },
            "primaryType": "EIP712Domain",
            "domain": {
                "name": "example.metamask.io",
                "version": "1",
                "chainId": 1,
                "verifyingContract": "0x0000000000000000000000000000000000000000"
            },
            "message": {}
        });

        let typed_data: TypedData = serde_json::from_value(json).unwrap();

        let hash = typed_data.eip712_signing_hash().unwrap();
        assert_eq!(
            hex::encode(&hash[..]),
            "122d1c8ef94b76dad44dcb03fa772361e20855c63311a15d5afe02d1b38f6077",
        );
    }

    #[test]
    fn test_minimal_message() {
        let json = json!({
            "types": {
                "EIP712Domain": []
            },
            "primaryType": "EIP712Domain",
            "domain": {},
            "message": {}
        });

        let typed_data: TypedData = serde_json::from_value(json).unwrap();

        let hash = typed_data.eip712_signing_hash().unwrap();
        assert_eq!(
            hex::encode(&hash[..]),
            "8d4a3f4082945b7879e2b55f181c31a77c8c0a464b70669458abbaaf99de4c38",
        );
    }

    #[test]
    fn test_encode_custom_array_type() {
        let json = json!({
            "domain": {},
            "types": {
                "EIP712Domain": [],
                "Person": [
                    {
                        "name": "name",
                        "type": "string"
                    },
                    {
                        "name": "wallet",
                        "type": "address[]"
                    }
                ],
                "Mail": [
                    {
                        "name": "from",
                        "type": "Person"
                    },
                    {
                        "name": "to",
                        "type": "Person[]"
                    },
                    {
                        "name": "contents",
                        "type": "string"
                    }
                ]
            },
            "primaryType": "Mail",
            "message": {
                "from": {
                    "name": "Cow",
                    "wallet": [
                        "0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826",
                        "0xDD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"
                    ]
                },
                "to": [
                    {
                        "name": "Bob",
                        "wallet": [
                            "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"
                        ]
                    }
                ],
                "contents": "Hello, Bob!"
            }
        });

        let typed_data: TypedData = serde_json::from_value(json).unwrap();

        let hash = typed_data.eip712_signing_hash().unwrap();
        assert_eq!(
            hex::encode(&hash[..]),
            "80a3aeb51161cfc47884ddf8eac0d2343d6ae640efe78b6a69be65e3045c1321",
        );
    }

    #[test]
    fn test_hash_typed_message_with_data() {
        let json = json!({
            "types": {
                "EIP712Domain": [
                    {
                        "name": "name",
                        "type": "string"
                    },
                    {
                        "name": "version",
                        "type": "string"
                    },
                    {
                        "name": "chainId",
                        "type": "uint256"
                    },
                    {
                        "name": "verifyingContract",
                        "type": "address"
                    }
                ],
                "Message": [
                    {
                        "name": "data",
                        "type": "string"
                    }
                ]
            },
            "primaryType": "Message",
            "domain": {
                "name": "example.metamask.io",
                "version": "1",
                "chainId": "1",
                "verifyingContract": "0x0000000000000000000000000000000000000000"
            },
            "message": {
                "data": "Hello!"
            }
        });

        let typed_data: TypedData = serde_json::from_value(json).unwrap();

        let hash = typed_data.eip712_signing_hash().unwrap();
        assert_eq!(
            hex::encode(&hash[..]),
            "232cd3ec058eb935a709f093e3536ce26cc9e8e193584b0881992525f6236eef",
        );
    }

    #[test]
    fn test_hash_custom_data_type() {
        let json = json!({
            "domain": {},
            "types": {
                "EIP712Domain": [],
                "Person": [
                    {
                        "name": "name",
                        "type": "string"
                    },
                    {
                        "name": "wallet",
                        "type": "address"
                    }
                ],
                "Mail": [
                    {
                        "name": "from",
                        "type": "Person"
                    },
                    {
                        "name": "to",
                        "type": "Person"
                    },
                    {
                        "name": "contents",
                        "type": "string"
                    }
                ]
            },
            "primaryType": "Mail",
            "message": {
                "from": {
                    "name": "Cow",
                    "wallet": "0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"
                },
                "to": {
                    "name": "Bob",
                    "wallet": "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"
                },
                "contents": "Hello, Bob!"
            }
        });

        let typed_data: TypedData = serde_json::from_value(json).unwrap();

        let hash = typed_data.eip712_signing_hash().unwrap();
        assert_eq!(
            hex::encode(&hash[..]),
            "25c3d40a39e639a4d0b6e4d2ace5e1281e039c88494d97d8d08f99a6ea75d775",
        );
    }

    #[test]
    fn test_hash_recursive_types() {
        let json = json!({
            "domain": {},
            "types": {
                "EIP712Domain": [],
                "Person": [
                    {
                        "name": "name",
                        "type": "string"
                    },
                    {
                        "name": "wallet",
                        "type": "address"
                    }
                ],
                "Mail": [
                    {
                        "name": "from",
                        "type": "Person"
                    },
                    {
                        "name": "to",
                        "type": "Person"
                    },
                    {
                        "name": "contents",
                        "type": "string"
                    },
                    {
                        "name": "replyTo",
                        "type": "Mail"
                    }
                ]
            },
            "primaryType": "Mail",
            "message": {
                "from": {
                    "name": "Cow",
                    "wallet": "0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"
                },
                "to": {
                    "name": "Bob",
                    "wallet": "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"
                },
                "contents": "Hello, Bob!",
                "replyTo": {
                    "to": {
                        "name": "Cow",
                        "wallet": "0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"
                    },
                    "from": {
                        "name": "Bob",
                        "wallet": "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"
                    },
                    "contents": "Hello!"
                }
            }
        });

        let typed_data: TypedData = serde_json::from_value(json).unwrap();

        assert_eq!(typed_data.eip712_signing_hash(), Err(Error::CircularDependency("Mail".into())));
    }

    #[test]
    fn test_hash_nested_struct_array() {
        let json = json!({
            "types": {
                "EIP712Domain": [
                    {
                        "name": "name",
                        "type": "string"
                    },
                    {
                        "name": "version",
                        "type": "string"
                    },
                    {
                        "name": "chainId",
                        "type": "uint256"
                    },
                    {
                        "name": "verifyingContract",
                        "type": "address"
                    }
                ],
                "OrderComponents": [
                    {
                        "name": "offerer",
                        "type": "address"
                    },
                    {
                        "name": "zone",
                        "type": "address"
                    },
                    {
                        "name": "offer",
                        "type": "OfferItem[]"
                    },
                    {
                        "name": "startTime",
                        "type": "uint256"
                    },
                    {
                        "name": "endTime",
                        "type": "uint256"
                    },
                    {
                        "name": "zoneHash",
                        "type": "bytes32"
                    },
                    {
                        "name": "salt",
                        "type": "uint256"
                    },
                    {
                        "name": "conduitKey",
                        "type": "bytes32"
                    },
                    {
                        "name": "counter",
                        "type": "uint256"
                    }
                ],
                "OfferItem": [
                    {
                        "name": "token",
                        "type": "address"
                    }
                ],
                "ConsiderationItem": [
                    {
                        "name": "token",
                        "type": "address"
                    },
                    {
                        "name": "identifierOrCriteria",
                        "type": "uint256"
                    },
                    {
                        "name": "startAmount",
                        "type": "uint256"
                    },
                    {
                        "name": "endAmount",
                        "type": "uint256"
                    },
                    {
                        "name": "recipient",
                        "type": "address"
                    }
                ]
            },
            "primaryType": "OrderComponents",
            "domain": {
                "name": "Seaport",
                "version": "1.1",
                "chainId": "1",
                "verifyingContract": "0x00000000006c3852cbEf3e08E8dF289169EdE581"
            },
            "message": {
                "offerer": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                "offer": [
                    {
                        "token": "0xA604060890923Ff400e8c6f5290461A83AEDACec"
                    }
                ],
                "startTime": "1658645591",
                "endTime": "1659250386",
                "zone": "0x004C00500000aD104D7DBd00e3ae0A5C00560C00",
                "zoneHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "salt": "16178208897136618",
                "conduitKey": "0x0000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f0000",
                "totalOriginalConsiderationItems": "2",
                "counter": "0"
            }
        });

        let typed_data: TypedData = serde_json::from_value(json).unwrap();

        let hash = typed_data.eip712_signing_hash().unwrap();
        assert_eq!(
            hex::encode(&hash[..]),
            "0b8aa9f3712df0034bc29fe5b24dd88cfdba02c7f499856ab24632e2969709a8",
        );
    }

    #[test]
    fn from_sol_struct() {
        sol! {
            #[derive(Serialize, Deserialize)]
            struct MyStruct {
                string name;
                string otherThing;
            }
        }

        let s = MyStruct { name: "hello".to_string(), otherThing: "world".to_string() };

        let typed_data = TypedData::from_struct(&s, None);
        assert_eq!(typed_data.encode_type().unwrap(), "MyStruct(string name,string otherThing)");

        assert!(typed_data.resolver.contains_type_name("EIP712Domain"));
    }

    #[test]
    fn e2e_from_sol_struct() {
        sol! {
            #[derive(Serialize, Deserialize)]
            struct Person {
                string name;
                address wallet;
            }

            #[derive(Serialize, Deserialize)]
            struct Mail {
                Person from;
                Person to;
                string contents;
            }
        }

        let sender = Person {
            name: "Cow".to_string(),
            wallet: "0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826".parse().unwrap(),
        };
        let recipient = Person {
            name: "Bob".to_string(),
            wallet: "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB".parse().unwrap(),
        };
        let mail = Mail { from: sender, to: recipient, contents: "Hello, Bob!".to_string() };

        let typed_data = TypedData::from_struct(&mail, None);

        let hash = typed_data.eip712_signing_hash().unwrap();
        assert_eq!(
            hex::encode(&hash[..]),
            "25c3d40a39e639a4d0b6e4d2ace5e1281e039c88494d97d8d08f99a6ea75d775",
        );
    }
}
