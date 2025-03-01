use alloy_primitives::{address, Address};
use core::{cmp::Ordering, fmt, time::Duration};
use num_enum::TryFromPrimitiveError;

#[allow(unused_imports)]
use alloc::string::String;
// When adding a new chain:
//   1. add new variant to the NamedChain enum;
//   2. add extra information in the last `impl` block (explorer URLs, block time) when applicable;
//   3. (optional) add aliases:
//     - Strum (in kebab-case): `#[strum(to_string = "<main>", serialize = "<aliasX>", ...)]`
//      `to_string = "<main>"` must be present and will be used in `Display`, `Serialize`
//      and `FromStr`, while `serialize = "<aliasX>"` will be appended to `FromStr`.
//      More info: <https://docs.rs/strum/latest/strum/additional_attributes/index.html#attributes-on-variants>
//     - Serde (in snake_case): `#[cfg_attr(feature = "serde", serde(alias = "<aliasX>", ...))]`
//      Aliases are appended to the `Deserialize` implementation.
//      More info: <https://serde.rs/variant-attrs.html>
//     - Add a test at the bottom of the file
//   4. run `cargo test --all-features` to update the JSON bindings and schema.

// We don't derive Serialize because it is manually implemented using AsRef<str> and it would break
// a lot of things since Serialize is `kebab-case` vs Deserialize `snake_case`. This means that the
// NamedChain type is not "round-trippable", because the Serialize and Deserialize implementations
// do not use the same case style.

/// An Ethereum EIP-155 chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(strum::IntoStaticStr)] // Into<&'static str>, AsRef<str>, fmt::Display and serde::Serialize
#[derive(strum::VariantNames)] // NamedChain::VARIANTS
#[derive(strum::VariantArray)] // NamedChain::VARIANTS
#[derive(strum::EnumString)] // FromStr, TryFrom<&str>
#[derive(strum::EnumIter)] // NamedChain::iter
#[derive(strum::EnumCount)] // NamedChain::COUNT
#[derive(num_enum::TryFromPrimitive)] // TryFrom<u64>
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[strum(serialize_all = "kebab-case")]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[repr(u64)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum NamedChain {
    #[strum(to_string = "mainnet", serialize = "ethlive")]
    #[cfg_attr(feature = "serde", serde(alias = "ethlive"))]
    Mainnet = 1,
    Morden = 2,
    Ropsten = 3,
    Rinkeby = 4,
    Goerli = 5,
    Kovan = 42,
    Holesky = 17000,
    Sepolia = 11155111,

    #[cfg_attr(feature = "serde", serde(alias = "odyssey"))]
    Odyssey = 911867,

    Optimism = 10,
    #[cfg_attr(feature = "serde", serde(alias = "optimism-kovan"))]
    OptimismKovan = 69,
    #[cfg_attr(feature = "serde", serde(alias = "optimism-goerli"))]
    OptimismGoerli = 420,
    #[cfg_attr(feature = "serde", serde(alias = "optimism-sepolia"))]
    OptimismSepolia = 11155420,

    #[strum(to_string = "bob")]
    #[cfg_attr(feature = "serde", serde(alias = "bob"))]
    Bob = 60808,
    #[strum(to_string = "bob-sepolia")]
    #[cfg_attr(feature = "serde", serde(alias = "bob-sepolia"))]
    BobSepolia = 808813,

    #[cfg_attr(feature = "serde", serde(alias = "arbitrum_one", alias = "arbitrum-one"))]
    Arbitrum = 42161,
    ArbitrumTestnet = 421611,
    #[cfg_attr(feature = "serde", serde(alias = "arbitrum-goerli"))]
    ArbitrumGoerli = 421613,
    #[cfg_attr(feature = "serde", serde(alias = "arbitrum-sepolia"))]
    ArbitrumSepolia = 421614,
    #[cfg_attr(feature = "serde", serde(alias = "arbitrum-nova"))]
    ArbitrumNova = 42170,

    Cronos = 25,
    CronosTestnet = 338,

    Rsk = 30,

    #[strum(to_string = "telos")]
    #[cfg_attr(feature = "serde", serde(alias = "telos", alias = "telos_evm"))]
    TelosEvm = 40,
    #[strum(to_string = "telos-testnet")]
    #[cfg_attr(
        feature = "serde",
        serde(alias = "telos_testnet", alias = "telos-evm-testnet", alias = "telos_evm_testnet")
    )]
    TelosEvmTestnet = 41,

    #[strum(to_string = "crab")]
    #[cfg_attr(feature = "serde", serde(alias = "crab"))]
    Crab = 44,
    #[strum(to_string = "darwinia")]
    #[cfg_attr(feature = "serde", serde(alias = "darwinia"))]
    Darwinia = 46,
    #[strum(to_string = "koi")]
    #[cfg_attr(feature = "serde", serde(alias = "koi"))]
    Koi = 701,

    /// Note the correct name for BSC should be `BNB Smart Chain` due to the rebranding: <https://www.bnbchain.org/en/blog/bsc-is-now-bnb-chain-the-infrastructure-for-the-metafi-universe>
    /// We keep `Binance Smart Chain` for backward compatibility, and the enum could be renamed in
    /// the future release.
    #[strum(to_string = "bsc", serialize = "binance-smart-chain", serialize = "bnb-smart-chain")]
    #[cfg_attr(
        feature = "serde",
        serde(alias = "bsc", alias = "bnb-smart-chain", alias = "binance-smart-chain")
    )]
    BinanceSmartChain = 56,
    #[strum(
        to_string = "bsc-testnet",
        serialize = "binance-smart-chain-testnet",
        serialize = "bnb-smart-chain-testnet"
    )]
    #[cfg_attr(
        feature = "serde",
        serde(
            alias = "bsc_testnet",
            alias = "bsc-testnet",
            alias = "bnb-smart-chain-testnet",
            alias = "binance-smart-chain-testnet"
        )
    )]
    BinanceSmartChainTestnet = 97,

    Poa = 99,
    Sokol = 77,

    Scroll = 534352,
    #[cfg_attr(
        feature = "serde",
        serde(alias = "scroll_sepolia_testnet", alias = "scroll-sepolia")
    )]
    ScrollSepolia = 534351,

    Metis = 1088,

    #[cfg_attr(feature = "serde", serde(alias = "conflux-espace-testnet"))]
    CfxTestnet = 71,
    #[cfg_attr(feature = "serde", serde(alias = "conflux-espace"))]
    Cfx = 1030,

    #[strum(to_string = "xdai", serialize = "gnosis", serialize = "gnosis-chain")]
    #[cfg_attr(feature = "serde", serde(alias = "xdai", alias = "gnosis", alias = "gnosis-chain"))]
    Gnosis = 100,

    Polygon = 137,
    #[strum(to_string = "mumbai", serialize = "polygon-mumbai")]
    #[cfg_attr(feature = "serde", serde(alias = "mumbai", alias = "polygon-mumbai"))]
    PolygonMumbai = 80001,
    #[strum(to_string = "amoy", serialize = "polygon-amoy")]
    #[cfg_attr(feature = "serde", serde(alias = "amoy", alias = "polygon-amoy"))]
    PolygonAmoy = 80002,
    #[strum(serialize = "polygon-zkevm", serialize = "zkevm")]
    #[cfg_attr(
        feature = "serde",
        serde(alias = "zkevm", alias = "polygon_zkevm", alias = "polygon-zkevm")
    )]
    PolygonZkEvm = 1101,
    #[strum(serialize = "polygon-zkevm-testnet", serialize = "zkevm-testnet")]
    #[cfg_attr(
        feature = "serde",
        serde(
            alias = "zkevm-testnet",
            alias = "polygon_zkevm_testnet",
            alias = "polygon-zkevm-testnet"
        )
    )]
    PolygonZkEvmTestnet = 1442,

    Fantom = 250,
    FantomTestnet = 4002,

    Moonbeam = 1284,
    MoonbeamDev = 1281,

    Moonriver = 1285,

    Moonbase = 1287,

    Dev = 1337,
    #[strum(to_string = "anvil-hardhat", serialize = "anvil", serialize = "hardhat")]
    #[cfg_attr(
        feature = "serde",
        serde(alias = "anvil", alias = "hardhat", alias = "anvil-hardhat")
    )]
    AnvilHardhat = 31337,

    #[strum(to_string = "gravity-alpha-mainnet")]
    #[cfg_attr(feature = "serde", serde(alias = "gravity-alpha-mainnet"))]
    GravityAlphaMainnet = 1625,
    #[strum(to_string = "gravity-alpha-testnet-sepolia")]
    #[cfg_attr(feature = "serde", serde(alias = "gravity-alpha-testnet-sepolia"))]
    GravityAlphaTestnetSepolia = 13505,

    Evmos = 9001,
    EvmosTestnet = 9000,

    Chiado = 10200,

    Oasis = 26863,

    Emerald = 42262,
    EmeraldTestnet = 42261,

    FilecoinMainnet = 314,
    FilecoinCalibrationTestnet = 314159,

    Avalanche = 43114,
    #[strum(to_string = "fuji", serialize = "avalanche-fuji")]
    #[cfg_attr(feature = "serde", serde(alias = "fuji"))]
    AvalancheFuji = 43113,

    Celo = 42220,
    CeloAlfajores = 44787,
    CeloBaklava = 62320,

    Aurora = 1313161554,
    AuroraTestnet = 1313161555,

    Canto = 7700,
    CantoTestnet = 740,

    Boba = 288,

    Base = 8453,
    #[cfg_attr(feature = "serde", serde(alias = "base-goerli"))]
    BaseGoerli = 84531,
    #[cfg_attr(feature = "serde", serde(alias = "base-sepolia"))]
    BaseSepolia = 84532,
    #[cfg_attr(feature = "serde", serde(alias = "syndr"))]
    Syndr = 404,
    #[cfg_attr(feature = "serde", serde(alias = "syndr-sepolia"))]
    SyndrSepolia = 444444,

    Shimmer = 148,

    Ink = 57073,
    #[cfg_attr(feature = "serde", serde(alias = "ink_sepolia_testnet", alias = "ink-sepolia"))]
    InkSepolia = 763373,

    #[strum(to_string = "fraxtal")]
    #[cfg_attr(feature = "serde", serde(alias = "fraxtal"))]
    Fraxtal = 252,
    #[strum(to_string = "fraxtal-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "fraxtal-testnet"))]
    FraxtalTestnet = 2522,

    Blast = 81457,
    #[cfg_attr(feature = "serde", serde(alias = "blast-sepolia"))]
    BlastSepolia = 168587773,

    Linea = 59144,
    #[cfg_attr(feature = "serde", serde(alias = "linea-goerli"))]
    LineaGoerli = 59140,
    #[cfg_attr(feature = "serde", serde(alias = "linea-sepolia"))]
    LineaSepolia = 59141,

    #[strum(to_string = "zksync")]
    #[cfg_attr(feature = "serde", serde(alias = "zksync"))]
    ZkSync = 324,
    #[strum(to_string = "zksync-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "zksync_testnet", alias = "zksync-testnet"))]
    ZkSyncTestnet = 300,

    #[strum(to_string = "mantle")]
    #[cfg_attr(feature = "serde", serde(alias = "mantle"))]
    Mantle = 5000,
    #[strum(to_string = "mantle-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "mantle-testnet"))]
    MantleTestnet = 5001,
    #[strum(to_string = "mantle-sepolia")]
    #[cfg_attr(feature = "serde", serde(alias = "mantle-sepolia"))]
    MantleSepolia = 5003,

    #[strum(to_string = "xai")]
    #[cfg_attr(feature = "serde", serde(alias = "xai"))]
    Xai = 660279,
    #[strum(to_string = "xai-sepolia")]
    #[cfg_attr(feature = "serde", serde(alias = "xai-sepolia"))]
    XaiSepolia = 37714555429,

    #[strum(to_string = "happychain-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "happychain-testnet"))]
    HappychainTestnet = 216,

    Viction = 88,

    Zora = 7777777,
    #[cfg_attr(feature = "serde", serde(alias = "zora-goerli"))]
    ZoraGoerli = 999,
    #[cfg_attr(feature = "serde", serde(alias = "zora-sepolia"))]
    ZoraSepolia = 999999999,

    Pgn = 424,
    #[cfg_attr(feature = "serde", serde(alias = "pgn-sepolia"))]
    PgnSepolia = 58008,

    Mode = 34443,
    #[cfg_attr(feature = "serde", serde(alias = "mode-sepolia"))]
    ModeSepolia = 919,

    Elastos = 20,

    #[cfg_attr(
        feature = "serde",
        serde(alias = "kakarot-sepolia", alias = "kakarot-starknet-sepolia")
    )]
    KakarotSepolia = 920637907288165,

    #[cfg_attr(feature = "serde", serde(alias = "etherlink"))]
    Etherlink = 42793,

    #[cfg_attr(feature = "serde", serde(alias = "etherlink-testnet"))]
    EtherlinkTestnet = 128123,

    Degen = 666666666,

    #[strum(to_string = "opbnb-mainnet")]
    #[cfg_attr(
        feature = "serde",
        serde(rename = "opbnb_mainnet", alias = "opbnb-mainnet", alias = "op-bnb-mainnet")
    )]
    OpBNBMainnet = 204,
    #[strum(to_string = "opbnb-testnet")]
    #[cfg_attr(
        feature = "serde",
        serde(rename = "opbnb_testnet", alias = "opbnb-testnet", alias = "op-bnb-testnet")
    )]
    OpBNBTestnet = 5611,

    Ronin = 2020,

    #[cfg_attr(feature = "serde", serde(alias = "ronin-testnet"))]
    RoninTestnet = 2021,

    Taiko = 167000,
    #[cfg_attr(feature = "serde", serde(alias = "taiko-hekla"))]
    TaikoHekla = 167009,

    #[strum(to_string = "autonomys-nova-testnet")]
    #[cfg_attr(
        feature = "serde",
        serde(rename = "autonomys_nova_testnet", alias = "autonomys-nova-testnet")
    )]
    AutonomysNovaTestnet = 490000,

    Flare = 14,
    #[cfg_attr(feature = "serde", serde(alias = "flare-coston2"))]
    FlareCoston2 = 114,

    #[strum(to_string = "acala")]
    #[cfg_attr(feature = "serde", serde(alias = "acala"))]
    Acala = 787,
    #[strum(to_string = "acala-mandala-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "acala-mandala-testnet"))]
    AcalaMandalaTestnet = 595,
    #[strum(to_string = "acala-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "acala-testnet"))]
    AcalaTestnet = 597,

    #[strum(to_string = "karura")]
    #[cfg_attr(feature = "serde", serde(alias = "karura"))]
    Karura = 686,
    #[strum(to_string = "karura-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "karura-testnet"))]
    KaruraTestnet = 596,
    #[strum(to_string = "pulsechain")]
    #[cfg_attr(feature = "serde", serde(alias = "pulsechain"))]
    Pulsechain = 369,
    #[strum(to_string = "pulsechain-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "pulsechain-testnet"))]
    PulsechainTestnet = 943,

    #[strum(to_string = "immutable")]
    #[cfg_attr(feature = "serde", serde(alias = "immutable"))]
    Immutable = 13371,
    #[strum(to_string = "immutable-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "immutable-testnet"))]
    ImmutableTestnet = 13473,

    #[strum(to_string = "soneium")]
    #[cfg_attr(feature = "serde", serde(alias = "soneium"))]
    Soneium = 1868,

    #[strum(to_string = "soneium-minato-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "soneium-minato-testnet"))]
    SoneiumMinatoTestnet = 1946,

    #[cfg_attr(feature = "serde", serde(alias = "worldchain"))]
    World = 480,
    #[strum(to_string = "world-sepolia")]
    #[cfg_attr(feature = "serde", serde(alias = "worldchain-sepolia", alias = "world-sepolia"))]
    WorldSepolia = 4801,
    Iotex = 4689,
    Core = 1116,
    Merlin = 4200,
    Bitlayer = 200901,
    Vana = 1480,
    Zeta = 7000,
    Kaia = 8217,

    Unichain = 130,
    #[strum(to_string = "unichain-sepolia")]
    #[cfg_attr(feature = "serde", serde(alias = "unichain-sepolia"))]
    UnichainSepolia = 1301,

    #[strum(to_string = "apechain")]
    #[cfg_attr(feature = "serde", serde(alias = "apechain"))]
    ApeChain = 33139,
    #[strum(to_string = "curtis", serialize = "apechain-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "apechain-testnet", alias = "curtis"))]
    Curtis = 33111,

    #[strum(to_string = "sonic-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "sonic-testnet"))]
    SonicTestnet = 64165,
    #[strum(to_string = "sonic")]
    #[cfg_attr(feature = "serde", serde(alias = "sonic"))]
    Sonic = 146,

    #[strum(to_string = "treasure")]
    #[cfg_attr(feature = "serde", serde(alias = "treasure"))]
    Treasure = 61166,

    #[strum(to_string = "treasure-topaz", serialize = "treasure-topaz-testnet")]
    #[cfg_attr(
        feature = "serde",
        serde(alias = "treasure-topaz-testnet", alias = "treasure-topaz")
    )]
    TreasureTopaz = 978658,

    #[strum(to_string = "berachain-bartio", serialize = "berachain-bartio-testnet")]
    #[cfg_attr(
        feature = "serde",
        serde(alias = "berachain-bartio-testnet", alias = "berachain-bartio")
    )]
    BerachainBartio = 80084,

    #[strum(to_string = "berachain-artio", serialize = "berachain-artio-testnet")]
    #[cfg_attr(
        feature = "serde",
        serde(alias = "berachain-artio-testnet", alias = "berachain-artio")
    )]
    BerachainArtio = 80085,

    Berachain = 80094,

    #[strum(to_string = "superposition-testnet")]
    #[cfg_attr(feature = "serde", serde(alias = "superposition-testnet"))]
    SuperpositionTestnet = 98985,

    #[strum(to_string = "superposition")]
    #[cfg_attr(feature = "serde", serde(alias = "superposition"))]
    Superposition = 55244,
}

// This must be implemented manually so we avoid a conflict with `TryFromPrimitive` where it treats
// the `#[default]` attribute as its own `#[num_enum(default)]`
impl Default for NamedChain {
    #[inline]
    fn default() -> Self {
        Self::Mainnet
    }
}

macro_rules! impl_into_numeric {
    ($($t:ty)+) => {$(
        impl From<NamedChain> for $t {
            #[inline]
            fn from(chain: NamedChain) -> Self {
                chain as $t
            }
        }
    )+};
}

impl_into_numeric!(u64 i64 u128 i128);
#[cfg(target_pointer_width = "64")]
impl_into_numeric!(usize isize);

macro_rules! impl_try_from_numeric {
    ($($native:ty)+) => {
        $(
            impl TryFrom<$native> for NamedChain {
                type Error = TryFromPrimitiveError<NamedChain>;

                #[inline]
                fn try_from(value: $native) -> Result<Self, Self::Error> {
                    (value as u64).try_into()
                }
            }
        )+
    };
}

impl_try_from_numeric!(u8 i8 u16 i16 u32 i32 usize isize);

impl fmt::Display for NamedChain {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}

impl AsRef<str> for NamedChain {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<u64> for NamedChain {
    #[inline]
    fn eq(&self, other: &u64) -> bool {
        (*self as u64) == *other
    }
}

impl PartialOrd<u64> for NamedChain {
    #[inline]
    fn partial_cmp(&self, other: &u64) -> Option<Ordering> {
        (*self as u64).partial_cmp(other)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for NamedChain {
    #[inline]
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_ref())
    }
}

#[cfg(feature = "rlp")]
impl alloy_rlp::Encodable for NamedChain {
    #[inline]
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        (*self as u64).encode(out)
    }

    #[inline]
    fn length(&self) -> usize {
        (*self as u64).length()
    }
}

#[cfg(feature = "rlp")]
impl alloy_rlp::Decodable for NamedChain {
    #[inline]
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let n = u64::decode(buf)?;
        Self::try_from(n).map_err(|_| alloy_rlp::Error::Overflow)
    }
}

// NB: all utility functions *should* be explicitly exhaustive (not use `_` matcher) so we don't
//     forget to update them when adding a new `NamedChain` variant.
#[allow(clippy::match_like_matches_macro)]
#[deny(unreachable_patterns, unused_variables)]
impl NamedChain {
    /// Returns the string representation of the chain.
    #[inline]
    pub fn as_str(&self) -> &'static str {
        self.into()
    }

    /// Returns `true` if this chain is Ethereum or an Ethereum testnet.
    pub const fn is_ethereum(&self) -> bool {
        use NamedChain::*;

        matches!(self, Mainnet | Morden | Ropsten | Rinkeby | Goerli | Kovan | Holesky | Sepolia)
    }

    /// Returns true if the chain contains Optimism configuration.
    pub const fn is_optimism(self) -> bool {
        use NamedChain::*;

        matches!(
            self,
            Optimism
                | OptimismGoerli
                | OptimismKovan
                | OptimismSepolia
                | Base
                | BaseGoerli
                | BaseSepolia
                | Fraxtal
                | FraxtalTestnet
                | Ink
                | InkSepolia
                | Mode
                | ModeSepolia
                | Pgn
                | PgnSepolia
                | Zora
                | ZoraGoerli
                | ZoraSepolia
                | BlastSepolia
                | OpBNBMainnet
                | OpBNBTestnet
                | Soneium
                | SoneiumMinatoTestnet
                | Odyssey
                | World
                | WorldSepolia
                | Unichain
                | UnichainSepolia
                | HappychainTestnet
        )
    }

    /// Returns true if the chain contains Arbitrum configuration.
    pub const fn is_arbitrum(self) -> bool {
        use NamedChain::*;

        matches!(self, Arbitrum | ArbitrumTestnet | ArbitrumGoerli | ArbitrumSepolia | ArbitrumNova)
    }

    /// Returns the chain's average blocktime, if applicable.
    ///
    /// It can be beneficial to know the average blocktime to adjust the polling of an HTTP provider
    /// for example.
    ///
    /// **Note:** this is not an accurate average, but is rather a sensible default derived from
    /// blocktime charts such as [Etherscan's](https://etherscan.com/chart/blocktime)
    /// or [Polygonscan's](https://polygonscan.com/chart/blocktime).
    ///
    /// # Examples
    ///
    /// ```
    /// use alloy_chains::NamedChain;
    /// use std::time::Duration;
    ///
    /// assert_eq!(NamedChain::Mainnet.average_blocktime_hint(), Some(Duration::from_millis(12_000)),);
    /// assert_eq!(NamedChain::Optimism.average_blocktime_hint(), Some(Duration::from_millis(2_000)),);
    /// ```
    pub const fn average_blocktime_hint(self) -> Option<Duration> {
        use NamedChain::*;

        Some(Duration::from_millis(match self {
            Mainnet | Taiko | TaikoHekla => 12_000,

            Arbitrum
            | ArbitrumTestnet
            | ArbitrumGoerli
            | ArbitrumSepolia
            | GravityAlphaMainnet
            | GravityAlphaTestnetSepolia
            | Xai
            | XaiSepolia
            | Syndr
            | SyndrSepolia
            | ArbitrumNova
            | ApeChain
            | Curtis
            | SuperpositionTestnet
            | Superposition => 260,

            Optimism | OptimismGoerli | OptimismSepolia | Base | BaseGoerli | BaseSepolia
            | Blast | BlastSepolia | Fraxtal | FraxtalTestnet | Zora | ZoraGoerli | ZoraSepolia
            | Mantle | MantleSepolia | Mode | ModeSepolia | Pgn | PgnSepolia
            | HappychainTestnet | Soneium | SoneiumMinatoTestnet | Bob | BobSepolia => 2_000,

            Ink | InkSepolia | Odyssey => 1_000,

            Viction => 2_000,

            Polygon | PolygonMumbai | PolygonAmoy => 2_100,

            Acala | AcalaMandalaTestnet | AcalaTestnet | Karura | KaruraTestnet | Moonbeam
            | Moonriver => 12_500,

            BinanceSmartChain | BinanceSmartChainTestnet => 3_000,

            Avalanche | AvalancheFuji => 2_000,

            Fantom | FantomTestnet => 1_200,

            Cronos | CronosTestnet | Canto | CantoTestnet => 5_700,

            Evmos | EvmosTestnet => 1_900,

            Aurora | AuroraTestnet => 1_100,

            Oasis => 5_500,

            Emerald | Darwinia | Crab | Koi => 6_000,

            Dev | AnvilHardhat => 200,

            Celo | CeloAlfajores | CeloBaklava => 5_000,

            FilecoinCalibrationTestnet | FilecoinMainnet => 30_000,

            Scroll | ScrollSepolia => 3_000,

            Shimmer => 5_000,

            Gnosis | Chiado => 5_000,

            Elastos => 5_000,

            Etherlink => 5_000,

            EtherlinkTestnet => 5_000,

            Degen => 600,

            Cfx | CfxTestnet => 500,

            OpBNBMainnet | OpBNBTestnet | AutonomysNovaTestnet => 1_000,

            Ronin | RoninTestnet => 3_000,

            Flare => 1_800,

            FlareCoston2 => 2_500,

            Pulsechain => 10000,
            PulsechainTestnet => 10101,

            Immutable | ImmutableTestnet => 2_000,

            World | WorldSepolia => 2_000,

            Iotex => 5_000,
            Core => 3_000,
            Merlin => 3_000,
            Bitlayer => 3_000,
            Vana => 6_000,
            Zeta => 6_000,
            Kaia => 1_000,

            Sonic => 1_000,

            TelosEvm | TelosEvmTestnet => 500,

            UnichainSepolia | Unichain => 1_000,

            BerachainBartio | BerachainArtio | Berachain => 2_000,

            Morden | Ropsten | Rinkeby | Goerli | Kovan | Sepolia | Holesky | MantleTestnet
            | Moonbase | MoonbeamDev | OptimismKovan | Poa | Sokol | Rsk | EmeraldTestnet
            | Boba | ZkSync | ZkSyncTestnet | PolygonZkEvm | PolygonZkEvmTestnet | Metis
            | Linea | LineaGoerli | LineaSepolia | KakarotSepolia | SonicTestnet | Treasure
            | TreasureTopaz => return None,
        }))
    }

    /// Returns whether the chain implements EIP-1559 (with the type 2 EIP-2718 transaction type).
    ///
    /// # Examples
    ///
    /// ```
    /// use alloy_chains::NamedChain;
    ///
    /// assert!(!NamedChain::Mainnet.is_legacy());
    /// assert!(NamedChain::Celo.is_legacy());
    /// ```
    pub const fn is_legacy(self) -> bool {
        use NamedChain::*;

        match self {
            // Known legacy chains / non EIP-1559 compliant.
            Acala
            | AcalaMandalaTestnet
            | AcalaTestnet
            | ArbitrumTestnet
            | BinanceSmartChain
            | BinanceSmartChainTestnet
            | Boba
            | Celo
            | CeloAlfajores
            | CeloBaklava
            | Elastos
            | Emerald
            | EmeraldTestnet
            | Fantom
            | FantomTestnet
            | Karura
            | KaruraTestnet
            | MantleTestnet
            | Metis
            | Oasis
            | OptimismKovan
            | PolygonZkEvm
            | PolygonZkEvmTestnet
            | Ronin
            | RoninTestnet
            | Rsk
            | Shimmer
            | TelosEvm
            | TelosEvmTestnet
            | Treasure
            | TreasureTopaz
            | Viction
            | ZkSync
            | ZkSyncTestnet => true,

            // Known EIP-1559 chains.
            Mainnet
            | Goerli
            | Sepolia
            | Holesky
            | Odyssey
            | Base
            | BaseGoerli
            | BaseSepolia
            | Blast
            | BlastSepolia
            | Fraxtal
            | FraxtalTestnet
            | Optimism
            | OptimismGoerli
            | OptimismSepolia
            | Bob
            | BobSepolia
            | Polygon
            | PolygonMumbai
            | PolygonAmoy
            | Avalanche
            | AvalancheFuji
            | Arbitrum
            | ArbitrumGoerli
            | ArbitrumSepolia
            | ArbitrumNova
            | GravityAlphaMainnet
            | GravityAlphaTestnetSepolia
            | Xai
            | XaiSepolia
            | HappychainTestnet
            | Syndr
            | SyndrSepolia
            | FilecoinMainnet
            | Linea
            | LineaGoerli
            | LineaSepolia
            | FilecoinCalibrationTestnet
            | Gnosis
            | Chiado
            | Zora
            | ZoraGoerli
            | ZoraSepolia
            | Ink
            | InkSepolia
            | Mantle
            | MantleSepolia
            | Mode
            | ModeSepolia
            | Pgn
            | PgnSepolia
            | KakarotSepolia
            | Etherlink
            | EtherlinkTestnet
            | Degen
            | OpBNBMainnet
            | OpBNBTestnet
            | Taiko
            | TaikoHekla
            | AutonomysNovaTestnet
            | Flare
            | FlareCoston2
            | Scroll
            | ScrollSepolia
            | Darwinia
            | Cfx
            | CfxTestnet
            | Crab
            | Pulsechain
            | PulsechainTestnet
            | Koi
            | Immutable
            | ImmutableTestnet
            | Soneium
            | SoneiumMinatoTestnet
            | Sonic
            | World
            | WorldSepolia
            | Unichain
            | UnichainSepolia
            | ApeChain
            | BerachainBartio
            | BerachainArtio
            | Berachain
            | Curtis
            | SuperpositionTestnet
            | Superposition => false,

            // Unknown / not applicable, default to false for backwards compatibility.
            Dev | AnvilHardhat | Morden | Ropsten | Rinkeby | Cronos | CronosTestnet | Kovan
            | Sokol | Poa | Moonbeam | MoonbeamDev | Moonriver | Moonbase | Evmos
            | EvmosTestnet | Aurora | AuroraTestnet | Canto | CantoTestnet | Iotex | Core
            | Merlin | Bitlayer | SonicTestnet | Vana | Zeta | Kaia => false,
        }
    }

    /// Returns whether the chain supports the [Shanghai hardfork][ref].
    ///
    /// [ref]: https://github.com/ethereum/execution-specs/blob/master/network-upgrades/mainnet-upgrades/shanghai.md
    pub const fn supports_shanghai(self) -> bool {
        use NamedChain::*;

        matches!(
            self,
            Mainnet
                | Goerli
                | Sepolia
                | Holesky
                | AnvilHardhat
                | Optimism
                | OptimismGoerli
                | OptimismSepolia
                | Bob
                | BobSepolia
                | Odyssey
                | Base
                | BaseGoerli
                | BaseSepolia
                | Blast
                | BlastSepolia
                | Fraxtal
                | FraxtalTestnet
                | Ink
                | InkSepolia
                | Gnosis
                | Chiado
                | ZoraSepolia
                | Mantle
                | MantleSepolia
                | Mode
                | ModeSepolia
                | PolygonMumbai
                | Polygon
                | Arbitrum
                | ArbitrumNova
                | ArbitrumSepolia
                | GravityAlphaMainnet
                | GravityAlphaTestnetSepolia
                | Xai
                | XaiSepolia
                | Syndr
                | SyndrSepolia
                | Etherlink
                | EtherlinkTestnet
                | Scroll
                | ScrollSepolia
                | HappychainTestnet
                | Shimmer
                | OpBNBMainnet
                | OpBNBTestnet
                | KakarotSepolia
                | Taiko
                | TaikoHekla
                | Avalanche
                | AvalancheFuji
                | AutonomysNovaTestnet
                | Acala
                | AcalaMandalaTestnet
                | AcalaTestnet
                | Karura
                | KaruraTestnet
                | Darwinia
                | Crab
                | Cfx
                | CfxTestnet
                | Pulsechain
                | PulsechainTestnet
                | Koi
                | Immutable
                | ImmutableTestnet
                | Soneium
                | SoneiumMinatoTestnet
                | World
                | WorldSepolia
                | Iotex
                | Unichain
                | UnichainSepolia
                | ApeChain
                | Curtis
                | SuperpositionTestnet
                | Superposition
        )
    }

    #[doc(hidden)]
    #[deprecated(since = "0.1.3", note = "use `supports_shanghai` instead")]
    pub const fn supports_push0(self) -> bool {
        self.supports_shanghai()
    }

    /// Returns whether the chain is a testnet.
    pub const fn is_testnet(self) -> bool {
        use NamedChain::*;

        match self {
            // Ethereum testnets.
            Goerli | Holesky | Kovan | Sepolia | Morden | Ropsten | Rinkeby => true,

            // Other testnets.
            ArbitrumGoerli
            | ArbitrumSepolia
            | ArbitrumTestnet
            | SyndrSepolia
            | AuroraTestnet
            | AvalancheFuji
            | Odyssey
            | BaseGoerli
            | BaseSepolia
            | BlastSepolia
            | BinanceSmartChainTestnet
            | CantoTestnet
            | CronosTestnet
            | CeloAlfajores
            | CeloBaklava
            | EmeraldTestnet
            | EvmosTestnet
            | FantomTestnet
            | FilecoinCalibrationTestnet
            | FraxtalTestnet
            | HappychainTestnet
            | LineaGoerli
            | LineaSepolia
            | InkSepolia
            | MantleTestnet
            | MantleSepolia
            | MoonbeamDev
            | OptimismGoerli
            | OptimismKovan
            | OptimismSepolia
            | BobSepolia
            | PolygonMumbai
            | PolygonAmoy
            | PolygonZkEvmTestnet
            | ScrollSepolia
            | Shimmer
            | ZkSyncTestnet
            | ZoraGoerli
            | ZoraSepolia
            | ModeSepolia
            | PgnSepolia
            | KakarotSepolia
            | EtherlinkTestnet
            | OpBNBTestnet
            | RoninTestnet
            | TaikoHekla
            | AutonomysNovaTestnet
            | FlareCoston2
            | AcalaMandalaTestnet
            | AcalaTestnet
            | KaruraTestnet
            | CfxTestnet
            | PulsechainTestnet
            | GravityAlphaTestnetSepolia
            | XaiSepolia
            | Koi
            | ImmutableTestnet
            | SoneiumMinatoTestnet
            | WorldSepolia
            | UnichainSepolia
            | Curtis
            | TreasureTopaz
            | SonicTestnet
            | BerachainBartio
            | BerachainArtio
            | SuperpositionTestnet
            | TelosEvmTestnet => true,

            // Dev chains.
            Dev | AnvilHardhat => true,

            // Mainnets.
            Mainnet | Optimism | Arbitrum | ArbitrumNova | Blast | Syndr | Cronos | Rsk
            | BinanceSmartChain | Poa | Sokol | Scroll | Metis | Gnosis | Polygon
            | PolygonZkEvm | Fantom | Moonbeam | Moonriver | Moonbase | Evmos | Chiado | Oasis
            | Emerald | FilecoinMainnet | Avalanche | Celo | Aurora | Canto | Boba | Base
            | Fraxtal | Ink | Linea | ZkSync | Mantle | GravityAlphaMainnet | Xai | Zora | Pgn
            | Mode | Viction | Elastos | Degen | OpBNBMainnet | Ronin | Taiko | Flare | Acala
            | Karura | Darwinia | Cfx | Crab | Pulsechain | Etherlink | Immutable | World
            | Iotex | Core | Merlin | Bitlayer | ApeChain | Vana | Zeta | Kaia | Treasure | Bob
            | Soneium | Sonic | Superposition | Berachain | Unichain | TelosEvm => false,
        }
    }

    /// Returns the symbol of the chain's native currency.
    pub const fn native_currency_symbol(self) -> Option<&'static str> {
        use NamedChain::*;

        Some(match self {
            Mainnet | Goerli | Holesky | Kovan | Sepolia | Morden | Ropsten | Rinkeby | Scroll
            | ScrollSepolia | Taiko | TaikoHekla | Unichain | UnichainSepolia
            | SuperpositionTestnet | Superposition => "ETH",

            Mantle | MantleSepolia => "MNT",

            GravityAlphaMainnet | GravityAlphaTestnetSepolia => "G",

            Xai | XaiSepolia => "XAI",

            HappychainTestnet => "HAPPY",

            BinanceSmartChain | BinanceSmartChainTestnet | OpBNBMainnet | OpBNBTestnet => "BNB",

            Etherlink | EtherlinkTestnet => "XTZ",

            Degen => "DEGEN",

            Ronin | RoninTestnet => "RON",

            Shimmer => "SMR",

            Flare => "FLR",

            FlareCoston2 => "C2FLR",

            Darwinia => "RING",

            Crab => "CRAB",

            Koi => "KRING",

            Cfx | CfxTestnet => "CFX",
            Pulsechain | PulsechainTestnet => "PLS",

            Immutable => "IMX",
            ImmutableTestnet => "tIMX",

            World | WorldSepolia => "WRLD",

            Iotex => "IOTX",
            Core => "CORE",
            Merlin => "BTC",
            Bitlayer => "BTC",
            Vana => "VANA",
            Zeta => "ZETA",
            Kaia => "KAIA",
            ApeChain | Curtis => "APE",

            Treasure | TreasureTopaz => "MAGIC",

            BerachainBartio | BerachainArtio | Berachain => "BERA",

            Sonic => "S",

            TelosEvm | TelosEvmTestnet => "TLOS",

            _ => return None,
        })
    }

    /// Returns the chain's blockchain explorer and its API (Etherscan and Etherscan-like) URLs.
    ///
    /// Returns `(API_URL, BASE_URL)`.
    ///
    /// All URLs have no trailing `/`
    ///
    /// # Examples
    ///
    /// ```
    /// use alloy_chains::NamedChain;
    ///
    /// assert_eq!(
    ///     NamedChain::Mainnet.etherscan_urls(),
    ///     Some(("https://api.etherscan.io/api", "https://etherscan.io"))
    /// );
    /// assert_eq!(
    ///     NamedChain::Avalanche.etherscan_urls(),
    ///     Some(("https://api.snowtrace.io/api", "https://snowtrace.io"))
    /// );
    /// assert_eq!(NamedChain::AnvilHardhat.etherscan_urls(), None);
    /// ```
    pub const fn etherscan_urls(self) -> Option<(&'static str, &'static str)> {
        use NamedChain::*;

        Some(match self {
            Mainnet => ("https://api.etherscan.io/api", "https://etherscan.io"),
            Ropsten => ("https://api-ropsten.etherscan.io/api", "https://ropsten.etherscan.io"),
            Kovan => ("https://api-kovan.etherscan.io/api", "https://kovan.etherscan.io"),
            Rinkeby => ("https://api-rinkeby.etherscan.io/api", "https://rinkeby.etherscan.io"),
            Goerli => ("https://api-goerli.etherscan.io/api", "https://goerli.etherscan.io"),
            Sepolia => ("https://api-sepolia.etherscan.io/api", "https://sepolia.etherscan.io"),
            Holesky => ("https://api-holesky.etherscan.io/api", "https://holesky.etherscan.io"),

            Polygon => ("https://api.polygonscan.com/api", "https://polygonscan.com"),
            PolygonMumbai => {
                ("https://api-testnet.polygonscan.com/api", "https://mumbai.polygonscan.com")
            }
            PolygonAmoy => ("https://api-amoy.polygonscan.com/api", "https://amoy.polygonscan.com"),

            PolygonZkEvm => {
                ("https://api-zkevm.polygonscan.com/api", "https://zkevm.polygonscan.com")
            }
            PolygonZkEvmTestnet => (
                "https://api-testnet-zkevm.polygonscan.com/api",
                "https://testnet-zkevm.polygonscan.com",
            ),

            Avalanche => ("https://api.snowtrace.io/api", "https://snowtrace.io"),
            AvalancheFuji => {
                ("https://api-testnet.snowtrace.io/api", "https://testnet.snowtrace.io")
            }

            Optimism => {
                ("https://api-optimistic.etherscan.io/api", "https://optimistic.etherscan.io")
            }
            OptimismGoerli => (
                "https://api-goerli-optimistic.etherscan.io/api",
                "https://goerli-optimism.etherscan.io",
            ),
            OptimismKovan => (
                "https://api-kovan-optimistic.etherscan.io/api",
                "https://kovan-optimistic.etherscan.io",
            ),
            OptimismSepolia => (
                "https://api-sepolia-optimistic.etherscan.io/api",
                "https://sepolia-optimism.etherscan.io",
            ),

            Bob => ("https://explorer.gobob.xyz/api", "https://explorer.gobob.xyz"),
            BobSepolia => (
                "https://bob-sepolia.explorer.gobob.xyz/api",
                "https://bob-sepolia.explorer.gobob.xyz",
            ),

            Fantom => ("https://api.ftmscan.com/api", "https://ftmscan.com"),
            FantomTestnet => ("https://api-testnet.ftmscan.com/api", "https://testnet.ftmscan.com"),

            BinanceSmartChain => ("https://api.bscscan.com/api", "https://bscscan.com"),
            BinanceSmartChainTestnet => {
                ("https://api-testnet.bscscan.com/api", "https://testnet.bscscan.com")
            }

            OpBNBMainnet => ("https://opbnb.bscscan.com/api", "https://opbnb.bscscan.com"),
            OpBNBTestnet => {
                ("https://opbnb-testnet.bscscan.com/api", "https://opbnb-testnet.bscscan.com")
            }

            Arbitrum => ("https://api.arbiscan.io/api", "https://arbiscan.io"),
            ArbitrumTestnet => {
                ("https://api-testnet.arbiscan.io/api", "https://testnet.arbiscan.io")
            }
            ArbitrumGoerli => ("https://api-goerli.arbiscan.io/api", "https://goerli.arbiscan.io"),
            ArbitrumSepolia => {
                ("https://api-sepolia.arbiscan.io/api", "https://sepolia.arbiscan.io")
            }
            ArbitrumNova => ("https://api-nova.arbiscan.io/api", "https://nova.arbiscan.io"),

            GravityAlphaMainnet => {
                ("https://explorer.gravity.xyz/api", "https://explorer.gravity.xyz")
            }
            GravityAlphaTestnetSepolia => {
                ("https://explorer-sepolia.gravity.xyz/api", "https://explorer-sepolia.gravity.xyz")
            }
            HappychainTestnet => (
                "https://happy-testnet-sepolia.explorer.caldera.xyz/api",
                "https://happy-testnet-sepolia.explorer.caldera.xyz",
            ),

            XaiSepolia => ("https://sepolia.xaiscan.io/api", "https://sepolia.xaiscan.io"),
            Xai => ("https://xaiscan.io/api", "https://xaiscan.io"),

            Syndr => ("https://explorer.syndr.com/api", "https://explorer.syndr.com"),
            SyndrSepolia => {
                ("https://sepolia-explorer.syndr.com/api", "https://sepolia-explorer.syndr.com")
            }

            Cronos => ("https://api.cronoscan.com/api", "https://cronoscan.com"),
            CronosTestnet => {
                ("https://api-testnet.cronoscan.com/api", "https://testnet.cronoscan.com")
            }

            Moonbeam => ("https://api-moonbeam.moonscan.io/api", "https://moonbeam.moonscan.io"),
            Moonbase => ("https://api-moonbase.moonscan.io/api", "https://moonbase.moonscan.io"),
            Moonriver => ("https://api-moonriver.moonscan.io/api", "https://moonriver.moonscan.io"),

            Gnosis => ("https://api.gnosisscan.io/api", "https://gnosisscan.io"),

            Scroll => ("https://api.scrollscan.com/api", "https://scrollscan.com"),
            ScrollSepolia => {
                ("https://api-sepolia.scrollscan.com/api", "https://sepolia.scrollscan.com")
            }

            Ink => ("https://explorer.inkonchain.com/api/v2", "https://explorer.inkonchain.com"),
            InkSepolia => (
                "https://explorer-sepolia.inkonchain.com/api/v2",
                "https://explorer-sepolia.inkonchain.com",
            ),

            Shimmer => {
                ("https://explorer.evm.shimmer.network/api", "https://explorer.evm.shimmer.network")
            }

            Metis => (
                "https://api.routescan.io/v2/network/mainnet/evm/1088/etherscan",
                "https://explorer.metis.io",
            ),

            Chiado => {
                ("https://blockscout.chiadochain.net/api", "https://blockscout.chiadochain.net")
            }

            FilecoinCalibrationTestnet => (
                "https://api.calibration.node.glif.io/rpc/v1",
                "https://calibration.filfox.info/en",
            ),

            Sokol => ("https://blockscout.com/poa/sokol/api", "https://blockscout.com/poa/sokol"),

            Poa => ("https://blockscout.com/poa/core/api", "https://blockscout.com/poa/core"),

            Rsk => ("https://blockscout.com/rsk/mainnet/api", "https://blockscout.com/rsk/mainnet"),

            Oasis => ("https://scan.oasischain.io/api", "https://scan.oasischain.io"),

            Emerald => {
                ("https://explorer.emerald.oasis.dev/api", "https://explorer.emerald.oasis.dev")
            }
            EmeraldTestnet => (
                "https://testnet.explorer.emerald.oasis.dev/api",
                "https://testnet.explorer.emerald.oasis.dev",
            ),

            Aurora => ("https://api.aurorascan.dev/api", "https://aurorascan.dev"),
            AuroraTestnet => {
                ("https://testnet.aurorascan.dev/api", "https://testnet.aurorascan.dev")
            }

            Evmos => ("https://evm.evmos.org/api", "https://evm.evmos.org"),
            EvmosTestnet => ("https://evm.evmos.dev/api", "https://evm.evmos.dev"),

            Celo => ("https://api.celoscan.io/api", "https://celoscan.io"),
            CeloAlfajores => {
                ("https://api-alfajores.celoscan.io/api", "https://alfajores.celoscan.io")
            }
            CeloBaklava => {
                ("https://explorer.celo.org/baklava/api", "https://explorer.celo.org/baklava")
            }

            Canto => ("https://evm.explorer.canto.io/api", "https://evm.explorer.canto.io"),
            CantoTestnet => (
                "https://testnet-explorer.canto.neobase.one/api",
                "https://testnet-explorer.canto.neobase.one",
            ),

            Boba => ("https://api.bobascan.com/api", "https://bobascan.com"),

            Base => ("https://api.basescan.org/api", "https://basescan.org"),
            BaseGoerli => ("https://api-goerli.basescan.org/api", "https://goerli.basescan.org"),
            BaseSepolia => ("https://api-sepolia.basescan.org/api", "https://sepolia.basescan.org"),

            Fraxtal => ("https://api.fraxscan.com/api", "https://fraxscan.com"),
            FraxtalTestnet => {
                ("https://api-holesky.fraxscan.com/api", "https://holesky.fraxscan.com")
            }

            Blast => ("https://api.blastscan.io/api", "https://blastscan.io"),
            BlastSepolia => {
                ("https://api-sepolia.blastscan.io/api", "https://sepolia.blastscan.io")
            }

            ZkSync => ("https://api-era.zksync.network/api", "https://era.zksync.network"),
            ZkSyncTestnet => {
                ("https://api-sepolia-era.zksync.network/api", "https://sepolia-era.zksync.network")
            }

            Linea => ("https://api.lineascan.build/api", "https://lineascan.build"),
            LineaGoerli => {
                ("https://explorer.goerli.linea.build/api", "https://explorer.goerli.linea.build")
            }
            LineaSepolia => {
                ("https://api-sepolia.lineascan.build/api", "https://sepolia.lineascan.build")
            }

            Mantle => ("https://explorer.mantle.xyz/api", "https://explorer.mantle.xyz"),
            MantleTestnet => {
                ("https://explorer.testnet.mantle.xyz/api", "https://explorer.testnet.mantle.xyz")
            }
            MantleSepolia => {
                ("https://explorer.sepolia.mantle.xyz/api", "https://explorer.sepolia.mantle.xyz")
            }

            Viction => ("https://www.vicscan.xyz/api", "https://www.vicscan.xyz"),

            Zora => ("https://explorer.zora.energy/api", "https://explorer.zora.energy"),
            ZoraGoerli => {
                ("https://testnet.explorer.zora.energy/api", "https://testnet.explorer.zora.energy")
            }
            ZoraSepolia => {
                ("https://sepolia.explorer.zora.energy/api", "https://sepolia.explorer.zora.energy")
            }

            Pgn => {
                ("https://explorer.publicgoods.network/api", "https://explorer.publicgoods.network")
            }

            PgnSepolia => (
                "https://explorer.sepolia.publicgoods.network/api",
                "https://explorer.sepolia.publicgoods.network",
            ),

            Mode => ("https://explorer.mode.network/api", "https://explorer.mode.network"),
            ModeSepolia => (
                "https://sepolia.explorer.mode.network/api",
                "https://sepolia.explorer.mode.network",
            ),

            Elastos => ("https://esc.elastos.io/api", "https://esc.elastos.io"),

            AnvilHardhat | Dev | Morden | MoonbeamDev | FilecoinMainnet | AutonomysNovaTestnet
            | Iotex => {
                return None;
            }
            KakarotSepolia => {
                ("https://sepolia.kakarotscan.org/api", "https://sepolia.kakarotscan.org")
            }
            Etherlink => ("https://explorer.etherlink.com/api", "https://explorer.etherlink.com"),
            EtherlinkTestnet => (
                "https://testnet-explorer.etherlink.com/api",
                "https://testnet-explorer.etherlink.com",
            ),
            Degen => ("https://explorer.degen.tips/api", "https://explorer.degen.tips"),
            Ronin => ("https://skynet-api.roninchain.com/ronin", "https://app.roninchain.com"),
            RoninTestnet => (
                "https://api-gateway.skymavis.com/rpc/testnet",
                "https://saigon-app.roninchain.com",
            ),
            Taiko => ("https://api.taikoscan.io/api", "https://taikoscan.io"),
            TaikoHekla => ("https://api-testnet.taikoscan.io/api", "https://hekla.taikoscan.io"),
            Flare => {
                ("https://flare-explorer.flare.network/api", "https://flare-explorer.flare.network")
            }
            FlareCoston2 => (
                "https://coston2-explorer.flare.network/api",
                "https://coston2-explorer.flare.network",
            ),
            Acala => ("https://blockscout.acala.network/api", "https://blockscout.acala.network"),
            AcalaMandalaTestnet => (
                "https://blockscout.mandala.aca-staging.network/api",
                "https://blockscout.mandala.aca-staging.network",
            ),
            AcalaTestnet => (
                "https://blockscout.acala-testnet.aca-staging.network/api",
                "https://blockscout.acala-testnet.aca-staging.network",
            ),
            Karura => {
                ("https://blockscout.karura.network/api", "https://blockscout.karura.network")
            }
            KaruraTestnet => (
                "https://blockscout.karura-testnet.aca-staging.network/api",
                "https://blockscout.karura-testnet.aca-staging.network",
            ),

            Darwinia => {
                ("https://explorer.darwinia.network/api", "https://explorer.darwinia.network")
            }
            Crab => {
                ("https://crab-scan.darwinia.network/api", "https://crab-scan.darwinia.network")
            }
            Koi => ("https://koi-scan.darwinia.network/api", "https://koi-scan.darwinia.network"),
            Cfx => ("https://evmapi.confluxscan.net/api", "https://evm.confluxscan.io"),
            CfxTestnet => {
                ("https://evmapi-testnet.confluxscan.net/api", "https://evmtestnet.confluxscan.io")
            }
            Pulsechain => ("https://api.scan.pulsechain.com", "https://scan.pulsechain.com"),
            PulsechainTestnet => (
                "https://api.scan.v4.testnet.pulsechain.com",
                "https://scan.v4.testnet.pulsechain.com",
            ),

            Immutable => ("https://explorer.immutable.com/api", "https://explorer.immutable.com"),
            ImmutableTestnet => (
                "https://explorer.testnet.immutable.com/api",
                "https://explorer.testnet.immutable.com",
            ),
            Soneium => ("https://soneium.blockscout.com/api", "https://soneium.blockscout.com"),
            SoneiumMinatoTestnet => (
                "https://soneium-minato.blockscout.com/api",
                "https://soneium-minato.blockscout.com",
            ),
            Odyssey => {
                ("https://odyssey-explorer.ithaca.xyz/api", "https://odyssey-explorer.ithaca.xyz")
            }
            World => ("https://api.worldscan.org/api", "https://worldscan.org"),
            WorldSepolia => {
                ("https://api-sepolia.worldscan.org/api", "https://sepolia.worldscan.org")
            }
            Unichain => ("https://api.uniscan.xyz/api", "https://uniscan.xyz"),
            UnichainSepolia => {
                ("https://api-sepolia.uniscan.xyz/api", "https://sepolia.uniscan.xyz")
            }
            Core => ("https://openapi.coredao.org/api", "https://scan.coredao.org"),
            Merlin => ("https://scan.merlinchain.io/api", "https://scan.merlinchain.io"),
            Bitlayer => ("https://api.btrscan.com/scan/api", "https://www.btrscan.com"),
            Vana => ("https://api.vanascan.io/api", "https://vanascan.io"),
            Zeta => ("https://zetachain.blockscout.com/api", "https://zetachain.blockscout.com"),
            Kaia => ("https://mainnet-oapi.kaiascan.io/api", "https://kaiascan.io"),

            ApeChain => ("https://api.apescan.io/api", "https://apescan.io"),
            Curtis => ("https://curtis.explorer.caldera.xyz/api/v2", "https://curtis.apescan.io"),
            SonicTestnet => (
                "https://api.routescan.io/v2/network/testnet/evm/64165/etherscan/api",
                "https://scan.soniclabs.com",
            ),
            Sonic => ("https://api.sonicscan.org/api", "https://sonicscan.org"),
            Treasure => ("https://block-explorer.treasurescan.io/api", "https://treasurescan.io"),
            TreasureTopaz => (
                "https://block-explorer.topaz.treasurescan.io/api",
                "https://topaz.treasurescan.io",
            ),
            BerachainBartio => ("https://bartio.beratrail.io/api", "https://bartio.beratrail.io"),
            BerachainArtio => ("https://artio.beratrail.io/api", "https://artio.beratrail.io"),
            Berachain => ("https://api.berascan.com/api", "https://berascan.com"),
            SuperpositionTestnet => (
                "https://testnet-explorer.superposition.so/api",
                "https://testnet-explorer.superposition.so",
            ),
            Superposition => {
                ("https://explorer.superposition.so/api", "https://explorer.superposition.so")
            }
            TelosEvm => ("https://api.teloscan.io/api", "https://teloscan.io"),
            TelosEvmTestnet => {
                ("https://api.testnet.teloscan.io/api", "https://testnet.teloscan.io")
            }
        })
    }

    /// Returns the chain's blockchain explorer's API key environment variable's default name.
    ///
    /// # Examples
    ///
    /// ```
    /// use alloy_chains::NamedChain;
    ///
    /// assert_eq!(NamedChain::Mainnet.etherscan_api_key_name(), Some("ETHERSCAN_API_KEY"));
    /// assert_eq!(NamedChain::AnvilHardhat.etherscan_api_key_name(), None);
    /// ```
    pub const fn etherscan_api_key_name(self) -> Option<&'static str> {
        use NamedChain::*;

        let api_key_name = match self {
            Mainnet
            | Morden
            | Ropsten
            | Kovan
            | Rinkeby
            | Goerli
            | Holesky
            | Optimism
            | OptimismGoerli
            | OptimismKovan
            | OptimismSepolia
            | BinanceSmartChain
            | BinanceSmartChainTestnet
            | OpBNBMainnet
            | OpBNBTestnet
            | Arbitrum
            | ArbitrumTestnet
            | ArbitrumGoerli
            | ArbitrumSepolia
            | ArbitrumNova
            | Syndr
            | SyndrSepolia
            | Cronos
            | CronosTestnet
            | Aurora
            | AuroraTestnet
            | Celo
            | CeloAlfajores
            | Base
            | Linea
            | LineaSepolia
            | Mantle
            | MantleTestnet
            | MantleSepolia
            | Xai
            | XaiSepolia
            | BaseGoerli
            | BaseSepolia
            | Fraxtal
            | FraxtalTestnet
            | Blast
            | BlastSepolia
            | Gnosis
            | Scroll
            | ScrollSepolia
            | Taiko
            | TaikoHekla
            | Unichain
            | UnichainSepolia
            | ApeChain => "ETHERSCAN_API_KEY",

            Avalanche | AvalancheFuji => "SNOWTRACE_API_KEY",

            Polygon | PolygonMumbai | PolygonAmoy | PolygonZkEvm | PolygonZkEvmTestnet => {
                "POLYGONSCAN_API_KEY"
            }

            Fantom | FantomTestnet => "FTMSCAN_API_KEY",

            Moonbeam | Moonbase | MoonbeamDev | Moonriver => "MOONSCAN_API_KEY",

            Acala | AcalaMandalaTestnet | AcalaTestnet | Canto | CantoTestnet | CeloBaklava
            | Etherlink | EtherlinkTestnet | Flare | FlareCoston2 | KakarotSepolia | Karura
            | KaruraTestnet | Mode | ModeSepolia | Pgn | PgnSepolia | Shimmer | Zora
            | ZoraGoerli | ZoraSepolia | Darwinia | Crab | Koi | Immutable | ImmutableTestnet
            | Soneium | SoneiumMinatoTestnet | World | WorldSepolia | Curtis | Ink | InkSepolia
            | SuperpositionTestnet | Superposition => "BLOCKSCOUT_API_KEY",

            Boba => "BOBASCAN_API_KEY",

            Core => "CORESCAN_API_KEY",
            Merlin => "MERLINSCAN_API_KEY",
            Bitlayer => "BITLAYERSCAN_API_KEY",
            Vana => "VANASCAN_API_KEY",
            Zeta => "ZETASCAN_API_KEY",
            Kaia => "KAIASCAN_API_KEY",
            Sonic => "SONICSCAN_API_KEY",
            Berachain => "BERASCAN_API_KEY",
            // Explicitly exhaustive. See NB above.
            Metis
            | Chiado
            | Odyssey
            | Sepolia
            | Rsk
            | Sokol
            | Poa
            | Oasis
            | Emerald
            | EmeraldTestnet
            | Evmos
            | EvmosTestnet
            | AnvilHardhat
            | Dev
            | GravityAlphaMainnet
            | GravityAlphaTestnetSepolia
            | Bob
            | BobSepolia
            | ZkSync
            | ZkSyncTestnet
            | FilecoinMainnet
            | LineaGoerli
            | FilecoinCalibrationTestnet
            | Viction
            | Elastos
            | Degen
            | Ronin
            | RoninTestnet
            | Cfx
            | CfxTestnet
            | Pulsechain
            | PulsechainTestnet
            | AutonomysNovaTestnet
            | Iotex
            | HappychainTestnet
            | SonicTestnet
            | Treasure
            | TreasureTopaz
            | BerachainBartio
            | BerachainArtio
            | TelosEvm
            | TelosEvmTestnet => return None,
        };

        Some(api_key_name)
    }

    /// Returns the chain's blockchain explorer's API key, from the environment variable with the
    /// name specified in [`etherscan_api_key_name`](NamedChain::etherscan_api_key_name).
    ///
    /// # Examples
    ///
    /// ```
    /// use alloy_chains::NamedChain;
    ///
    /// let chain = NamedChain::Mainnet;
    /// std::env::set_var(chain.etherscan_api_key_name().unwrap(), "KEY");
    /// assert_eq!(chain.etherscan_api_key().as_deref(), Some("KEY"));
    /// ```
    #[cfg(feature = "std")]
    pub fn etherscan_api_key(self) -> Option<String> {
        self.etherscan_api_key_name().and_then(|name| std::env::var(name).ok())
    }

    /// Returns the address of the public DNS node list for the given chain.
    ///
    /// See also <https://github.com/ethereum/discv4-dns-lists>.
    pub fn public_dns_network_protocol(self) -> Option<String> {
        use NamedChain::*;

        const DNS_PREFIX: &str = "enrtree://AKA3AM6LPBYEUDMVNU3BSVQJ5AD45Y7YPOHJLEF6W26QOE4VTUDPE@";
        if let Mainnet | Goerli | Sepolia | Ropsten | Rinkeby | Holesky = self {
            // `{DNS_PREFIX}all.{self.lower()}.ethdisco.net`
            let mut s = String::with_capacity(DNS_PREFIX.len() + 32);
            s.push_str(DNS_PREFIX);
            s.push_str("all.");
            let chain_str = self.as_ref();
            s.push_str(chain_str);
            let l = s.len();
            s[l - chain_str.len()..].make_ascii_lowercase();
            s.push_str(".ethdisco.net");

            Some(s)
        } else {
            None
        }
    }

    /// Returns the address of the most popular wrapped native token address for this chain, if it
    /// exists.
    ///
    /// Example:
    ///
    /// ```
    /// use alloy_chains::NamedChain;
    /// use alloy_primitives::address;
    ///
    /// let chain = NamedChain::Mainnet;
    /// assert_eq!(
    ///     chain.wrapped_native_token(),
    ///     Some(address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"))
    /// );
    /// ```
    pub const fn wrapped_native_token(self) -> Option<Address> {
        use NamedChain::*;

        let addr = match self {
            Mainnet => address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
            Optimism => address!("4200000000000000000000000000000000000006"),
            BinanceSmartChain => address!("bb4cdb9cbd36b01bd1cbaebf2de08d9173bc095c"),
            OpBNBMainnet => address!("4200000000000000000000000000000000000006"),
            Arbitrum => address!("82af49447d8a07e3bd95bd0d56f35241523fbab1"),
            Base => address!("4200000000000000000000000000000000000006"),
            Linea => address!("e5d7c2a44ffddf6b295a15c148167daaaf5cf34f"),
            Mantle => address!("deaddeaddeaddeaddeaddeaddeaddeaddead1111"),
            Blast => address!("4300000000000000000000000000000000000004"),
            Gnosis => address!("e91d153e0b41518a2ce8dd3d7944fa863463a97d"),
            Scroll => address!("5300000000000000000000000000000000000004"),
            Taiko => address!("a51894664a773981c6c112c43ce576f315d5b1b6"),
            Avalanche => address!("b31f66aa3c1e785363f0875a1b74e27b85fd66c7"),
            Polygon => address!("0d500b1d8e8ef31e21c99d1db9a6444d3adf1270"),
            Fantom => address!("21be370d5312f44cb42ce377bc9b8a0cef1a4c83"),
            Iotex => address!("a00744882684c3e4747faefd68d283ea44099d03"),
            Core => address!("40375C92d9FAf44d2f9db9Bd9ba41a3317a2404f"),
            Merlin => address!("F6D226f9Dc15d9bB51182815b320D3fBE324e1bA"),
            Bitlayer => address!("ff204e2681a6fa0e2c3fade68a1b28fb90e4fc5f"),
            ApeChain => address!("48b62137EdfA95a428D35C09E44256a739F6B557"),
            Vana => address!("00EDdD9621Fb08436d0331c149D1690909a5906d"),
            Zeta => address!("5F0b1a82749cb4E2278EC87F8BF6B618dC71a8bf"),
            Kaia => address!("19aac5f612f524b754ca7e7c41cbfa2e981a4432"),
            Treasure => address!("263d8f36bb8d0d9526255e205868c26690b04b88"),
            Superposition => address!("1fB719f10b56d7a85DCD32f27f897375fB21cfdd"),
            Sonic => address!("039e2fB66102314Ce7b64Ce5Ce3E5183bc94aD38"),
            Berachain => address!("6969696969696969696969696969696969696969"),
            _ => return None,
        };

        Some(addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::{EnumCount, IntoEnumIterator};

    #[allow(unused_imports)]
    use alloc::string::ToString;

    #[test]
    #[cfg(feature = "serde")]
    fn default() {
        assert_eq!(serde_json::to_string(&NamedChain::default()).unwrap(), "\"mainnet\"");
    }

    #[test]
    fn enum_iter() {
        assert_eq!(NamedChain::COUNT, NamedChain::iter().size_hint().0);
    }

    #[test]
    fn roundtrip_string() {
        for chain in NamedChain::iter() {
            let chain_string = chain.to_string();
            assert_eq!(chain_string, format!("{chain}"));
            assert_eq!(chain_string.as_str(), chain.as_ref());
            #[cfg(feature = "serde")]
            assert_eq!(serde_json::to_string(&chain).unwrap(), format!("\"{chain_string}\""));

            assert_eq!(chain_string.parse::<NamedChain>().unwrap(), chain);
        }
    }

    #[test]
    #[cfg(feature = "serde")]
    fn roundtrip_serde() {
        for chain in NamedChain::iter() {
            let chain_string = serde_json::to_string(&chain).unwrap();
            let chain_string = chain_string.replace('-', "_");
            assert_eq!(serde_json::from_str::<'_, NamedChain>(&chain_string).unwrap(), chain);
        }
    }

    #[test]
    fn aliases() {
        use NamedChain::*;

        // kebab-case
        const ALIASES: &[(NamedChain, &[&str])] = &[
            (Mainnet, &["ethlive"]),
            (BinanceSmartChain, &["bsc", "bnb-smart-chain", "binance-smart-chain"]),
            (
                BinanceSmartChainTestnet,
                &["bsc-testnet", "bnb-smart-chain-testnet", "binance-smart-chain-testnet"],
            ),
            (Gnosis, &["gnosis", "gnosis-chain"]),
            (PolygonMumbai, &["mumbai"]),
            (PolygonZkEvm, &["zkevm", "polygon-zkevm"]),
            (PolygonZkEvmTestnet, &["zkevm-testnet", "polygon-zkevm-testnet"]),
            (AnvilHardhat, &["anvil", "hardhat"]),
            (AvalancheFuji, &["fuji"]),
            (ZkSync, &["zksync"]),
            (Mantle, &["mantle"]),
            (MantleTestnet, &["mantle-testnet"]),
            (MantleSepolia, &["mantle-sepolia"]),
            (GravityAlphaMainnet, &["gravity-alpha-mainnet"]),
            (GravityAlphaTestnetSepolia, &["gravity-alpha-testnet-sepolia"]),
            (Bob, &["bob"]),
            (BobSepolia, &["bob-sepolia"]),
            (HappychainTestnet, &["happychain-testnet"]),
            (Xai, &["xai"]),
            (XaiSepolia, &["xai-sepolia"]),
            (Base, &["base"]),
            (BaseGoerli, &["base-goerli"]),
            (BaseSepolia, &["base-sepolia"]),
            (Fraxtal, &["fraxtal"]),
            (FraxtalTestnet, &["fraxtal-testnet"]),
            (Ink, &["ink"]),
            (InkSepolia, &["ink-sepolia"]),
            (BlastSepolia, &["blast-sepolia"]),
            (Syndr, &["syndr"]),
            (SyndrSepolia, &["syndr-sepolia"]),
            (LineaGoerli, &["linea-goerli"]),
            (LineaSepolia, &["linea-sepolia"]),
            (AutonomysNovaTestnet, &["autonomys-nova-testnet"]),
            (Immutable, &["immutable"]),
            (ImmutableTestnet, &["immutable-testnet"]),
            (Soneium, &["soneium"]),
            (SoneiumMinatoTestnet, &["soneium-minato-testnet"]),
            (ApeChain, &["apechain"]),
            (Curtis, &["apechain-testnet", "curtis"]),
            (Treasure, &["treasure"]),
            (TreasureTopaz, &["treasure-topaz-testnet", "treasure-topaz"]),
            (BerachainArtio, &["berachain-artio-testnet", "berachain-artio"]),
            (BerachainBartio, &["berachain-bartio-testnet", "berachain-bartio"]),
            (SuperpositionTestnet, &["superposition-testnet"]),
            (Superposition, &["superposition"]),
        ];

        for &(chain, aliases) in ALIASES {
            for &alias in aliases {
                let named = alias.parse::<NamedChain>().expect(alias);
                assert_eq!(named, chain);

                #[cfg(feature = "serde")]
                {
                    assert_eq!(
                        serde_json::from_str::<NamedChain>(&format!("\"{alias}\"")).unwrap(),
                        chain
                    );

                    assert_eq!(
                        serde_json::from_str::<NamedChain>(&format!("\"{named}\"")).unwrap(),
                        chain
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_to_string_match() {
        for chain in NamedChain::iter() {
            let chain_serde = serde_json::to_string(&chain).unwrap();
            let chain_string = format!("\"{chain}\"");
            assert_eq!(chain_serde, chain_string);
        }
    }

    #[test]
    fn test_dns_network() {
        let s = "enrtree://AKA3AM6LPBYEUDMVNU3BSVQJ5AD45Y7YPOHJLEF6W26QOE4VTUDPE@all.mainnet.ethdisco.net";
        assert_eq!(NamedChain::Mainnet.public_dns_network_protocol().unwrap(), s);
    }

    #[test]
    fn ensure_no_trailing_etherscan_url_separator() {
        for chain in NamedChain::iter() {
            if let Some((api, base)) = chain.etherscan_urls() {
                assert!(!api.ends_with('/'), "{:?} api url has trailing /", chain);
                assert!(!base.ends_with('/'), "{:?} base url has trailing /", chain);
            }
        }
    }
}
