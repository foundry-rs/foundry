//! [ERC-8021](https://github.com/ethereum/ERCs/pull/1209) transaction attribution suffixes.
//!
//! The suffix is `schemaData || schemaId (1 byte) || marker (16 bytes)` appended to calldata, and
//! the length of `schemaData` is read backwards according to `schemaId`.

use alloy_primitives::hex;
use itertools::Itertools;
use serde::Deserialize;
use std::fmt;

/// Marker that ends an ERC-8021 attribution suffix.
const MARKER: [u8; 16] = hex!("80218021802180218021802180218021");

/// Codes attributed by an ERC-8021 suffix.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Attribution {
    /// The attributed codes, with the role of the entity for schema 2.
    codes: Vec<(String, Option<&'static str>)>,
}

impl Attribution {
    /// Decodes the ERC-8021 attribution suffix at the end of the given calldata.
    ///
    /// Returns `None` if there is no suffix, its schema is unknown, or it attributes no codes.
    /// Custom registries and metadata are not decoded.
    pub fn decode(data: &[u8]) -> Option<Self> {
        let (_, schema_id, schema_data) = split_suffix(data)?;
        let codes = match schema_id {
            // `... || codes || codesLength (1)`
            0 | 1 => {
                let (&codes_len, rest) = schema_data.split_last()?;
                let codes = std::str::from_utf8(&rest[rest.len() - codes_len as usize..]).ok()?;
                codes
                    .split(',')
                    .filter(|code| !code.is_empty())
                    .map(|code| (code.into(), None))
                    .collect::<Vec<_>>()
            }
            // `cborData || cborLength (2)`
            2 => {
                let cbor = &schema_data[..schema_data.len() - 2];
                let CborAttribution { app, wallet, services } = ciborium::from_reader(cbor).ok()?;
                app.map(|code| (code, Some("app")))
                    .into_iter()
                    .chain(wallet.map(|code| (code, Some("wallet"))))
                    .chain(services.into_iter().map(|code| (code, Some("service"))))
                    .collect::<Vec<_>>()
            }
            _ => return None,
        };
        (!codes.is_empty()).then_some(Self { codes })
    }
}

impl fmt::Display for Attribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Codes come from calldata, so escape control characters before printing them.
        let codes = self.codes.iter().format_with(", ", |(code, role), f| match role {
            Some(role) => f(&format_args!("{} ({role})", code.escape_debug())),
            None => f(&code.escape_debug()),
        });
        write!(f, "{codes}")
    }
}

/// Entity codes of a schema 2 attribution.
#[derive(Deserialize)]
struct CborAttribution {
    #[serde(rename = "a")]
    app: Option<String>,
    #[serde(rename = "w")]
    wallet: Option<String>,
    #[serde(rename = "s", default)]
    services: Vec<String>,
}

/// Returns the given calldata without a trailing ERC-8021 attribution suffix.
pub fn strip_suffix(data: &[u8]) -> &[u8] {
    split_suffix(data).map_or(data, |(data, ..)| data)
}

/// Splits a trailing ERC-8021 attribution suffix off the given calldata, returning the calldata
/// before it, the schema id and the schema data.
fn split_suffix(data: &[u8]) -> Option<(&[u8], u8, &[u8])> {
    let rest = data.strip_suffix(&MARKER)?;
    let (&schema_id, rest) = rest.split_last()?;
    let schema_data_len = match schema_id {
        // `codes || codesLength (1)`
        0 => 1 + *rest.last()? as usize,
        // `codeRegistryAddress (20) || chainId || chainIdLength (1) || codes || codesLength (1)`
        1 => {
            let codes_len = *rest.last()? as usize;
            let chain_id_len = *rest.get(rest.len().checked_sub(2 + codes_len)?)? as usize;
            22 + codes_len + chain_id_len
        }
        // `schemaData || schemaDataLength (2)`
        _ => 2 + u16::from_be_bytes(*rest.last_chunk()?) as usize,
    };
    let (data, schema_data) = rest.split_at(rest.len().checked_sub(schema_data_len)?);
    Some((data, schema_id, schema_data))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test vectors from ERC-8021.
    const SCHEMA_0: &[u8] = &hex!("dddddddd62617365617070070080218021802180218021802180218021");
    const SCHEMA_1: &[u8] = &hex!(
        "ddddddddcccccccccccccccccccccccccccccccccccccccc210502626173656170702C6D6F7270686F0E0180218021802180218021802180218021"
    );
    const SCHEMA_2: &[u8] =
        &hex!("dddddddda161616762617365617070000b0280218021802180218021802180218021");
    const SCHEMA_2_METADATA: &[u8] = &hex!(
        "dddddddda46161676261736561707061776570726976796172a16161a26163663078323130356161782a307842636632423935393845633037383165453439323330436141434630464233433838314235313935616da26c75746d5f63616d706169676e6c77696e7465722d70726f6d6f66736f7572636566776562617070007b0280218021802180218021802180218021"
    );
    const SCHEMA_2_SERVICES: &[u8] = &hex!(
        "dddddddda361616762617365617070617765707269767961738269666c617368626f747365746974616e00260280218021802180218021802180218021"
    );
    const UNKNOWN_SCHEMA: &[u8] = &hex!("ddddddddff80218021802180218021802180218021");

    #[test]
    fn test_strip_suffix() {
        for data in [SCHEMA_0, SCHEMA_1, SCHEMA_2, SCHEMA_2_METADATA, SCHEMA_2_SERVICES] {
            assert_eq!(strip_suffix(data), hex!("dddddddd"));
        }
        assert_eq!(strip_suffix(UNKNOWN_SCHEMA), UNKNOWN_SCHEMA);
    }

    #[test]
    fn test_decode_attribution() {
        let decode =
            |data: &[u8]| Attribution::decode(data).map(|attribution| attribution.to_string());
        assert_eq!(decode(SCHEMA_0).as_deref(), Some("baseapp"));
        assert_eq!(decode(SCHEMA_1).as_deref(), Some("baseapp, morpho"));
        assert_eq!(decode(SCHEMA_2).as_deref(), Some("baseapp (app)"));
        assert_eq!(decode(SCHEMA_2_METADATA).as_deref(), Some("baseapp (app), privy (wallet)"));
        assert_eq!(
            decode(SCHEMA_2_SERVICES).as_deref(),
            Some("baseapp (app), privy (wallet), flashbots (service), titan (service)")
        );
        assert_eq!(decode(UNKNOWN_SCHEMA), None);
        // Control characters are escaped.
        let data = [&hex!("1b5b33316d0500")[..], &MARKER].concat();
        assert_eq!(decode(&data).as_deref(), Some("\\u{1b}[31m"));
    }
}
