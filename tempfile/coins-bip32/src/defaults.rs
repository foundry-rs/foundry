use crate::enc::XKeyEncoder;

/// The default encoder, selected by feature flag
#[cfg(feature = "mainnet")]
pub type Encoder = crate::enc::MainnetEncoder;

/// The default encoder, selected by feature flag
#[cfg(feature = "testnet")]
pub type Encoder = crate::enc::TestnetEncoder;

impl std::str::FromStr for crate::xkeys::XPriv {
    type Err = crate::Bip32Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Encoder::xpriv_from_base58(s)
    }
}

impl std::str::FromStr for crate::xkeys::XPub {
    type Err = crate::Bip32Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Encoder::xpub_from_base58(s)
    }
}

impl serde::Serialize for crate::xkeys::XPub {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let encoded =
            Encoder::xpub_to_base58(self).map_err(|e| serde::ser::Error::custom(e.to_string()))?;
        serializer.serialize_str(&encoded)
    }
}

impl<'de> serde::Deserialize<'de> for crate::xkeys::XPub {
    fn deserialize<D>(deserializer: D) -> Result<crate::xkeys::XPub, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: &str = serde::Deserialize::deserialize(deserializer)?;
        Encoder::xpub_from_base58(s).map_err(|e| serde::de::Error::custom(e.to_string()))
    }
}

impl serde::Serialize for crate::xkeys::XPriv {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let encoded =
            Encoder::xpriv_to_base58(self).map_err(|e| serde::ser::Error::custom(e.to_string()))?;
        serializer.serialize_str(&encoded)
    }
}

impl<'de> serde::Deserialize<'de> for crate::xkeys::XPriv {
    fn deserialize<D>(deserializer: D) -> Result<crate::xkeys::XPriv, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: &str = serde::Deserialize::deserialize(deserializer)?;
        Encoder::xpriv_from_base58(s).map_err(|e| serde::de::Error::custom(e.to_string()))
    }
}
