//! Client identification: <https://github.com/ethereum/execution-apis/blob/main/src/engine/identification.md>

use alloc::string::String;

/// This enum defines a standard for specifying a client with just two letters. Clients teams which
/// have a code reserved in this list MUST use this code when identifying themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[derive(strum::IntoStaticStr)] // Into<&'static str>, AsRef<str>, fmt::Display and serde::Serialize
#[derive(strum::EnumString)] // FromStr, TryFrom<&str>
#[non_exhaustive]
pub enum ClientCode {
    /// Besu
    BU,
    /// EthereumJS
    EJ,
    /// Erigon
    EG,
    /// Geth, go-ethereum
    GE,
    /// Grandine
    GR,
    /// Lighthouse
    LH,
    /// Lodestar
    LS,
    /// Nethermind
    NM,
    /// Nimbus
    NB,
    /// Trin Execution
    TE,
    /// Teku
    TK,
    /// Prysm
    PM,
    /// Reth
    RH,
}

impl ClientCode {
    /// Returns the client identifier as str.
    pub fn as_str(&self) -> &'static str {
        (*self).into()
    }

    /// Returns the human readable client name for the given code.
    pub const fn client_name(&self) -> &'static str {
        match self {
            Self::BU => "Besu",
            Self::EJ => "EthereumJS",
            Self::EG => "Erigon",
            Self::GE => "Geth",
            Self::GR => "Grandine",
            Self::LH => "Lighthouse",
            Self::LS => "Lodestar",
            Self::NM => "Nethermind",
            Self::NB => "Nimbus",
            Self::TE => "Trin Execution",
            Self::TK => "Teku",
            Self::PM => "Prysm",
            Self::RH => "Reth",
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ClientCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl core::fmt::Display for ClientCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_str().fmt(f)
    }
}

/// Contains information which identifies a client implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct ClientVersionV1 {
    /// Client code, e.g. GE for Geth
    pub code: ClientCode,
    /// Human-readable name of the client, e.g. Lighthouse or go-ethereum
    pub name: String,
    /// The version string of the current implementation e.g. v4.6.0 or 1.0.0-alpha.1 or
    /// 1.0.0+20130313144700
    pub version: String,
    /// first four bytes of the latest commit hash of this build e.g: `fa4ff922`
    pub commit: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;

    #[test]
    #[cfg(feature = "serde")]
    fn client_id_serde() {
        let s = r#"{"code":"RH","name":"Reth","version":"v1.10.8","commit":"fa4ff922"}"#;
        let v: ClientVersionV1 = serde_json::from_str(s).unwrap();
        assert_eq!(v.code, ClientCode::RH);
        assert_eq!(v.name, "Reth");
        assert_eq!(serde_json::to_string(&v).unwrap(), s);
    }
}
