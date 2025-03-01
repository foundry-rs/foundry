use crate::NamedChain;
use core::{cmp::Ordering, fmt, str::FromStr, time::Duration};

#[allow(unused_imports)]
use alloc::string::String;

#[cfg(feature = "arbitrary")]
use proptest::{
    sample::Selector,
    strategy::{Map, TupleUnion, WA},
};
#[cfg(feature = "arbitrary")]
use strum::{EnumCount, IntoEnumIterator};

/// Either a known [`NamedChain`] or a EIP-155 chain ID.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chain(ChainKind);

/// The kind of chain. Returned by [`Chain::kind`]. Prefer using [`Chain`] instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChainKind {
    /// Known chain.
    Named(NamedChain),
    /// EIP-155 chain ID.
    Id(u64),
}

impl fmt::Debug for Chain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Chain::")?;
        self.kind().fmt(f)
    }
}

impl Default for Chain {
    #[inline]
    fn default() -> Self {
        Self::from_named(NamedChain::default())
    }
}

impl From<NamedChain> for Chain {
    #[inline]
    fn from(id: NamedChain) -> Self {
        Self::from_named(id)
    }
}

impl From<u64> for Chain {
    #[inline]
    fn from(id: u64) -> Self {
        Self::from_id(id)
    }
}

impl From<Chain> for u64 {
    #[inline]
    fn from(chain: Chain) -> Self {
        chain.id()
    }
}

impl TryFrom<Chain> for NamedChain {
    type Error = <NamedChain as TryFrom<u64>>::Error;

    #[inline]
    fn try_from(chain: Chain) -> Result<Self, Self::Error> {
        match *chain.kind() {
            ChainKind::Named(chain) => Ok(chain),
            ChainKind::Id(id) => id.try_into(),
        }
    }
}

impl FromStr for Chain {
    type Err = core::num::ParseIntError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(chain) = NamedChain::from_str(s) {
            Ok(Self::from_named(chain))
        } else {
            s.parse::<u64>().map(Self::from_id)
        }
    }
}

impl fmt::Display for Chain {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind() {
            ChainKind::Named(chain) => chain.fmt(f),
            ChainKind::Id(id) => id.fmt(f),
        }
    }
}

impl PartialEq<u64> for Chain {
    #[inline]
    fn eq(&self, other: &u64) -> bool {
        self.id().eq(other)
    }
}

impl PartialEq<Chain> for u64 {
    #[inline]
    fn eq(&self, other: &Chain) -> bool {
        other.eq(self)
    }
}

impl PartialOrd<u64> for Chain {
    #[inline]
    fn partial_cmp(&self, other: &u64) -> Option<Ordering> {
        self.id().partial_cmp(other)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Chain {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.kind() {
            ChainKind::Named(chain) => chain.serialize(serializer),
            ChainKind::Id(id) => id.serialize(serializer),
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Chain {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ChainVisitor;

        impl serde::de::Visitor<'_> for ChainVisitor {
            type Value = Chain;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("chain name or ID")
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                if v.is_negative() {
                    Err(serde::de::Error::invalid_value(serde::de::Unexpected::Signed(v), &self))
                } else {
                    Ok(Chain::from_id(v as u64))
                }
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(Chain::from_id(value))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                value.parse().map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_any(ChainVisitor)
    }
}

#[cfg(feature = "rlp")]
impl alloy_rlp::Encodable for Chain {
    #[inline]
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.id().encode(out)
    }

    #[inline]
    fn length(&self) -> usize {
        self.id().length()
    }
}

#[cfg(feature = "rlp")]
impl alloy_rlp::Decodable for Chain {
    #[inline]
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        u64::decode(buf).map(Self::from)
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for Chain {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        if u.ratio(1, 2)? {
            let chain = u.int_in_range(0..=(NamedChain::COUNT - 1))?;

            return Ok(Self::from_named(NamedChain::iter().nth(chain).expect("in range")));
        }

        Ok(Self::from_id(u64::arbitrary(u)?))
    }
}

#[cfg(feature = "arbitrary")]
impl proptest::arbitrary::Arbitrary for Chain {
    type Parameters = ();
    type Strategy = TupleUnion<(
        WA<Map<proptest::sample::SelectorStrategy, fn(proptest::sample::Selector) -> Chain>>,
        WA<Map<proptest::num::u64::Any, fn(u64) -> Chain>>,
    )>;

    fn arbitrary_with((): ()) -> Self::Strategy {
        use proptest::prelude::*;
        prop_oneof![
            any::<Selector>().prop_map(move |sel| Self::from_named(sel.select(NamedChain::iter()))),
            any::<u64>().prop_map(Self::from_id),
        ]
    }
}

impl Chain {
    #[allow(non_snake_case)]
    #[doc(hidden)]
    #[deprecated(since = "0.1.0", note = "use `Self::from_named()` instead")]
    #[inline]
    pub const fn Named(named: NamedChain) -> Self {
        Self::from_named(named)
    }

    #[allow(non_snake_case)]
    #[doc(hidden)]
    #[deprecated(since = "0.1.0", note = "use `Self::from_id()` instead")]
    #[inline]
    pub const fn Id(id: u64) -> Self {
        Self::from_id_unchecked(id)
    }

    /// Creates a new [`Chain`] by wrapping a [`NamedChain`].
    #[inline]
    pub const fn from_named(named: NamedChain) -> Self {
        Self(ChainKind::Named(named))
    }

    /// Creates a new [`Chain`] by wrapping a [`NamedChain`].
    #[inline]
    pub fn from_id(id: u64) -> Self {
        if let Ok(named) = NamedChain::try_from(id) {
            Self::from_named(named)
        } else {
            Self::from_id_unchecked(id)
        }
    }

    /// Creates a new [`Chain`] from the given ID, without checking if an associated [`NamedChain`]
    /// exists.
    ///
    /// This is discouraged, as other methods assume that the chain ID is not known, but it is not
    /// unsafe.
    #[inline]
    pub const fn from_id_unchecked(id: u64) -> Self {
        Self(ChainKind::Id(id))
    }

    /// Returns the mainnet chain.
    #[inline]
    pub const fn mainnet() -> Self {
        Self::from_named(NamedChain::Mainnet)
    }

    /// Returns the goerli chain.
    #[inline]
    pub const fn goerli() -> Self {
        Self::from_named(NamedChain::Goerli)
    }

    /// Returns the holesky chain.
    #[inline]
    pub const fn holesky() -> Self {
        Self::from_named(NamedChain::Holesky)
    }

    /// Returns the sepolia chain.
    #[inline]
    pub const fn sepolia() -> Self {
        Self::from_named(NamedChain::Sepolia)
    }

    /// Returns the optimism mainnet chain.
    #[inline]
    pub const fn optimism_mainnet() -> Self {
        Self::from_named(NamedChain::Optimism)
    }

    /// Returns the optimism goerli chain.
    #[inline]
    pub const fn optimism_goerli() -> Self {
        Self::from_named(NamedChain::OptimismGoerli)
    }

    /// Returns the optimism sepolia chain.
    #[inline]
    pub const fn optimism_sepolia() -> Self {
        Self::from_named(NamedChain::OptimismSepolia)
    }

    /// Returns the base mainnet chain.
    #[inline]
    pub const fn base_mainnet() -> Self {
        Self::from_named(NamedChain::Base)
    }

    /// Returns the base goerli chain.
    #[inline]
    pub const fn base_goerli() -> Self {
        Self::from_named(NamedChain::BaseGoerli)
    }

    /// Returns the base sepolia chain.
    #[inline]
    pub const fn base_sepolia() -> Self {
        Self::from_named(NamedChain::BaseSepolia)
    }

    /// Returns the arbitrum mainnet chain.
    #[inline]
    pub const fn arbitrum_mainnet() -> Self {
        Self::from_named(NamedChain::Arbitrum)
    }

    /// Returns the arbitrum nova chain.
    #[inline]
    pub const fn arbitrum_nova() -> Self {
        Self::from_named(NamedChain::ArbitrumNova)
    }

    /// Returns the arbitrum goerli chain.
    #[inline]
    pub const fn arbitrum_goerli() -> Self {
        Self::from_named(NamedChain::ArbitrumGoerli)
    }

    /// Returns the arbitrum sepolia chain.
    #[inline]
    pub const fn arbitrum_sepolia() -> Self {
        Self::from_named(NamedChain::ArbitrumSepolia)
    }

    /// Returns the arbitrum testnet chain.
    #[inline]
    pub const fn arbitrum_testnet() -> Self {
        Self::from_named(NamedChain::ArbitrumTestnet)
    }

    /// Returns the syndr l3 mainnet chain.
    #[inline]
    pub const fn syndr() -> Self {
        Self::from_named(NamedChain::Syndr)
    }

    /// Returns the syndr sepolia chain.
    #[inline]
    pub const fn syndr_sepolia() -> Self {
        Self::from_named(NamedChain::SyndrSepolia)
    }

    /// Returns the fraxtal mainnet chain.
    #[inline]
    pub const fn fraxtal() -> Self {
        Self::from_named(NamedChain::Fraxtal)
    }

    /// Returns the fraxtal testnet chain.
    #[inline]
    pub const fn fraxtal_testnet() -> Self {
        Self::from_named(NamedChain::FraxtalTestnet)
    }

    /// Returns the blast chain.
    #[inline]
    pub const fn blast() -> Self {
        Self::from_named(NamedChain::Blast)
    }

    /// Returns the blast sepolia chain.
    #[inline]
    pub const fn blast_sepolia() -> Self {
        Self::from_named(NamedChain::BlastSepolia)
    }

    /// Returns the linea mainnet chain.
    #[inline]
    pub const fn linea() -> Self {
        Self::from_named(NamedChain::Linea)
    }

    /// Returns the linea goerli chain.
    #[inline]
    pub const fn linea_goerli() -> Self {
        Self::from_named(NamedChain::LineaGoerli)
    }

    /// Returns the linea sepolia chain.
    #[inline]
    pub const fn linea_sepolia() -> Self {
        Self::from_named(NamedChain::LineaSepolia)
    }

    /// Returns the mode mainnet chain.
    #[inline]
    pub const fn mode() -> Self {
        Self::from_named(NamedChain::Mode)
    }

    /// Returns the mode sepolia chain.
    #[inline]
    pub const fn mode_sepolia() -> Self {
        Self::from_named(NamedChain::ModeSepolia)
    }

    /// Returns the elastos mainnet chain.
    #[inline]
    pub const fn elastos() -> Self {
        Self::from_named(NamedChain::Elastos)
    }

    /// Returns the degen l3 mainnet chain.
    #[inline]
    pub const fn degen() -> Self {
        Self::from_named(NamedChain::Degen)
    }

    /// Returns the dev chain.
    #[inline]
    pub const fn dev() -> Self {
        Self::from_named(NamedChain::Dev)
    }

    /// Returns the bsc mainnet chain.
    #[inline]
    pub const fn bsc_mainnet() -> Self {
        Self::from_named(NamedChain::BinanceSmartChain)
    }

    /// Returns the bsc testnet chain.
    #[inline]
    pub const fn bsc_testnet() -> Self {
        Self::from_named(NamedChain::BinanceSmartChainTestnet)
    }

    /// Returns the opbnb mainnet chain.
    #[inline]
    pub const fn opbnb_mainnet() -> Self {
        Self::from_named(NamedChain::OpBNBMainnet)
    }

    /// Returns the opbnb testnet chain.
    #[inline]
    pub const fn opbnb_testnet() -> Self {
        Self::from_named(NamedChain::OpBNBTestnet)
    }

    /// Returns the ronin mainnet chain.
    #[inline]
    pub const fn ronin() -> Self {
        Self::from_named(NamedChain::Ronin)
    }

    /// Returns the ronin testnet chain.
    #[inline]
    pub const fn ronin_testnet() -> Self {
        Self::from_named(NamedChain::RoninTestnet)
    }

    /// Returns the taiko mainnet chain.
    #[inline]
    pub const fn taiko() -> Self {
        Self::from_named(NamedChain::Taiko)
    }

    /// Returns the taiko hekla chain.
    #[inline]
    pub const fn taiko_hekla() -> Self {
        Self::from_named(NamedChain::TaikoHekla)
    }

    /// Returns the shimmer testnet chain.
    #[inline]
    pub const fn shimmer() -> Self {
        Self::from_named(NamedChain::Shimmer)
    }

    /// Returns the flare mainnet chain.
    #[inline]
    pub const fn flare() -> Self {
        Self::from_named(NamedChain::Flare)
    }

    /// Returns the flare testnet chain.
    #[inline]
    pub const fn flare_coston2() -> Self {
        Self::from_named(NamedChain::FlareCoston2)
    }

    /// Returns the darwinia mainnet chain.
    #[inline]
    pub const fn darwinia() -> Self {
        Self::from_named(NamedChain::Darwinia)
    }

    /// Returns the crab mainnet chain.
    #[inline]
    pub const fn crab() -> Self {
        Self::from_named(NamedChain::Crab)
    }

    /// Returns the koi testnet chain.
    #[inline]
    pub const fn koi() -> Self {
        Self::from_named(NamedChain::Koi)
    }

    /// Returns the Immutable zkEVM mainnet chain.
    #[inline]
    pub const fn immutable() -> Self {
        Self::from_named(NamedChain::Immutable)
    }

    /// Returns the Immutable zkEVM testnet chain.
    #[inline]
    pub const fn immutable_testnet() -> Self {
        Self::from_named(NamedChain::ImmutableTestnet)
    }

    /// Returns the ink sepolia chain.
    #[inline]
    pub const fn ink_sepolia() -> Self {
        Self::from_named(NamedChain::InkSepolia)
    }

    /// Returns the ink mainnet chain.
    #[inline]
    pub const fn ink_mainnet() -> Self {
        Self::from_named(NamedChain::Ink)
    }

    /// Returns the scroll mainnet chain
    #[inline]
    pub const fn scroll_mainnet() -> Self {
        Self::from_named(NamedChain::Scroll)
    }

    /// Returns the scroll sepolia chain
    #[inline]
    pub const fn scroll_sepolia() -> Self {
        Self::from_named(NamedChain::ScrollSepolia)
    }

    /// Returns the Treasure mainnet chain.
    #[inline]
    pub const fn treasure() -> Self {
        Self::from_named(NamedChain::Treasure)
    }

    /// Returns the Treasure Topaz testnet chain.
    #[inline]
    pub const fn treasure_topaz_testnet() -> Self {
        Self::from_named(NamedChain::TreasureTopaz)
    }

    /// Returns the Superposition testnet chain.
    #[inline]
    pub const fn superposition_testnet() -> Self {
        Self::from_named(NamedChain::SuperpositionTestnet)
    }

    /// Returns the Superposition mainnet chain.
    #[inline]
    pub const fn superposition() -> Self {
        Self::from_named(NamedChain::Superposition)
    }

    /// Returns the kind of this chain.
    #[inline]
    pub const fn kind(&self) -> &ChainKind {
        &self.0
    }

    /// Returns the kind of this chain.
    #[inline]
    pub const fn into_kind(self) -> ChainKind {
        self.0
    }

    /// Returns `true` if this chain is Ethereum or an Ethereum testnet.
    #[inline]
    pub const fn is_ethereum(&self) -> bool {
        matches!(self.named(), Some(named) if named.is_ethereum())
    }

    /// Returns true if the chain contains Optimism configuration.
    #[inline]
    pub const fn is_optimism(self) -> bool {
        matches!(self.named(), Some(named) if named.is_optimism())
    }

    /// Returns true if the chain contains Arbitrum configuration.
    #[inline]
    pub const fn is_arbitrum(self) -> bool {
        matches!(self.named(), Some(named) if named.is_arbitrum())
    }

    /// Attempts to convert the chain into a named chain.
    #[inline]
    pub const fn named(self) -> Option<NamedChain> {
        match *self.kind() {
            ChainKind::Named(named) => Some(named),
            ChainKind::Id(_) => None,
        }
    }

    /// The ID of the chain.
    #[inline]
    pub const fn id(self) -> u64 {
        match *self.kind() {
            ChainKind::Named(named) => named as u64,
            ChainKind::Id(id) => id,
        }
    }
}

/// Methods delegated to `NamedChain`. Note that [`ChainKind::Id`] won't be converted because it was
/// already done at construction.
impl Chain {
    /// Returns the chain's average blocktime, if applicable.
    ///
    /// See [`NamedChain::average_blocktime_hint`] for more info.
    pub const fn average_blocktime_hint(self) -> Option<Duration> {
        match self.kind() {
            ChainKind::Named(named) => named.average_blocktime_hint(),
            ChainKind::Id(_) => None,
        }
    }

    /// Returns whether the chain implements EIP-1559 (with the type 2 EIP-2718 transaction type).
    ///
    /// See [`NamedChain::is_legacy`] for more info.
    pub const fn is_legacy(self) -> bool {
        match self.kind() {
            ChainKind::Named(named) => named.is_legacy(),
            ChainKind::Id(_) => false,
        }
    }

    /// Returns whether the chain supports the [Shanghai hardfork][ref].
    ///
    /// See [`NamedChain::supports_shanghai`] for more info.
    ///
    /// [ref]: https://github.com/ethereum/execution-specs/blob/master/network-upgrades/mainnet-upgrades/shanghai.md
    pub const fn supports_shanghai(self) -> bool {
        match self.kind() {
            ChainKind::Named(named) => named.supports_shanghai(),
            ChainKind::Id(_) => false,
        }
    }

    #[doc(hidden)]
    #[deprecated(since = "0.1.3", note = "use `supports_shanghai` instead")]
    pub const fn supports_push0(self) -> bool {
        self.supports_shanghai()
    }

    /// Returns the chain's blockchain explorer and its API (Etherscan and Etherscan-like) URLs.
    ///
    /// See [`NamedChain::etherscan_urls`] for more info.
    pub const fn etherscan_urls(self) -> Option<(&'static str, &'static str)> {
        match self.kind() {
            ChainKind::Named(named) => named.etherscan_urls(),
            ChainKind::Id(_) => None,
        }
    }

    /// Returns the chain's blockchain explorer's API key environment variable's default name.
    ///
    /// See [`NamedChain::etherscan_api_key_name`] for more info.
    pub const fn etherscan_api_key_name(self) -> Option<&'static str> {
        match self.kind() {
            ChainKind::Named(named) => named.etherscan_api_key_name(),
            ChainKind::Id(_) => None,
        }
    }

    /// Returns the chain's blockchain explorer's API key, from the environment variable with the
    /// name specified in [`etherscan_api_key_name`](NamedChain::etherscan_api_key_name).
    ///
    /// See [`NamedChain::etherscan_api_key`] for more info.
    #[cfg(feature = "std")]
    pub fn etherscan_api_key(self) -> Option<String> {
        match self.kind() {
            ChainKind::Named(named) => named.etherscan_api_key(),
            ChainKind::Id(_) => None,
        }
    }

    /// Returns the address of the public DNS node list for the given chain.
    ///
    /// See [`NamedChain::public_dns_network_protocol`] for more info.
    pub fn public_dns_network_protocol(self) -> Option<String> {
        match self.kind() {
            ChainKind::Named(named) => named.public_dns_network_protocol(),
            ChainKind::Id(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(unused_imports)]
    use alloc::string::ToString;

    #[test]
    fn test_id() {
        assert_eq!(Chain::from_id(1234).id(), 1234);
    }

    #[test]
    fn test_named_id() {
        assert_eq!(Chain::from_named(NamedChain::Goerli).id(), 5);
    }

    #[test]
    fn test_display_named_chain() {
        assert_eq!(Chain::from_named(NamedChain::Mainnet).to_string(), "mainnet");
    }

    #[test]
    fn test_display_id_chain() {
        assert_eq!(Chain::from_id(1234).to_string(), "1234");
    }

    #[test]
    fn test_from_str_named_chain() {
        let result = Chain::from_str("mainnet");
        let expected = Chain::from_named(NamedChain::Mainnet);
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_from_str_named_chain_error() {
        let result = Chain::from_str("chain");
        assert!(result.is_err());
    }

    #[test]
    fn test_from_str_id_chain() {
        let result = Chain::from_str("1234");
        let expected = Chain::from_id(1234);
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_default() {
        let default = Chain::default();
        let expected = Chain::from_named(NamedChain::Mainnet);
        assert_eq!(default, expected);
    }

    #[cfg(feature = "rlp")]
    #[test]
    fn test_id_chain_encodable_length() {
        use alloy_rlp::Encodable;

        let chain = Chain::from_id(1234);
        assert_eq!(chain.length(), 3);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_serde() {
        let chains = r#"["mainnet",1,80001,80002,"mumbai"]"#;
        let re = r#"["mainnet","mainnet","mumbai","amoy","mumbai"]"#;
        let expected = [
            Chain::mainnet(),
            Chain::mainnet(),
            Chain::from_named(NamedChain::PolygonMumbai),
            Chain::from_id(80002),
            Chain::from_named(NamedChain::PolygonMumbai),
        ];
        assert_eq!(serde_json::from_str::<alloc::vec::Vec<Chain>>(chains).unwrap(), expected);
        assert_eq!(serde_json::to_string(&expected).unwrap(), re);
    }
}
