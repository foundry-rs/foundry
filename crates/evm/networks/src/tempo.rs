//! Tempo network precompiles

use alloy_primitives::{Address, address};

/// TIP Fee Manager precompile address
pub const TIP_FEE_MANAGER_ADDRESS: Address = address!("0xfeec000000000000000000000000000000000000");

/// PathUSD token address
pub const PATH_USD_ADDRESS: Address = address!("0x20C0000000000000000000000000000000000000");

/// TIP403 Registry precompile address
pub const TIP403_REGISTRY_ADDRESS: Address = address!("0x403C000000000000000000000000000000000000");

/// TIP20 Factory precompile address
pub const TIP20_FACTORY_ADDRESS: Address = address!("0x20FC000000000000000000000000000000000000");

/// TIP20 Rewards Registry precompile address
pub const TIP20_REWARDS_REGISTRY_ADDRESS: Address =
    address!("0x3000000000000000000000000000000000000000");

/// TIP Account Registrar precompile address
pub const TIP_ACCOUNT_REGISTRAR: Address = address!("0x7702ac0000000000000000000000000000000000");

/// Stablecoin Exchange precompile address
pub const STABLECOIN_EXCHANGE_ADDRESS: Address =
    address!("0xdec0000000000000000000000000000000000000");

/// Nonce Manager precompile address
pub const NONCE_PRECOMPILE_ADDRESS: Address =
    address!("0x4E4F4E4345000000000000000000000000000000");

/// Validator Config precompile address
pub const VALIDATOR_CONFIG_ADDRESS: Address =
    address!("0xCCCCCCCC00000000000000000000000000000000");

/// Account Keychain precompile address
pub const ACCOUNT_KEYCHAIN_ADDRESS: Address =
    address!("0xAAAAAAAA00000000000000000000000000000000");

/// TIP Fee Manager label for traces
pub const TIP_FEE_MANAGER_LABEL: &str = "TIP Fee Manager";

/// PathUSD label for traces
pub const PATH_USD_LABEL: &str = "PathUSD";

/// TIP403 Registry label for traces
pub const TIP403_REGISTRY_LABEL: &str = "TIP403 Registry";

/// TIP20 Factory label for traces
pub const TIP20_FACTORY_LABEL: &str = "TIP20 Factory";

/// TIP20 Rewards Registry label for traces
pub const TIP20_REWARDS_REGISTRY_LABEL: &str = "TIP20 Rewards Registry";

/// TIP Account Registrar label for traces
pub const TIP_ACCOUNT_REGISTRAR_LABEL: &str = "TIP Account Registrar";

/// Stablecoin Exchange label for traces
pub const STABLECOIN_EXCHANGE_LABEL: &str = "Stablecoin Exchange";

/// Nonce Manager label for traces
pub const NONCE_PRECOMPILE_LABEL: &str = "Nonce Manager";

/// Validator Config label for traces
pub const VALIDATOR_CONFIG_LABEL: &str = "Validator Config";

/// Account Keychain label for traces
pub const ACCOUNT_KEYCHAIN_LABEL: &str = "Account Keychain";

/// Tempo precompile identifier
pub struct TempoPrecompileId {
    name: &'static str,
    address: Address,
}

impl TempoPrecompileId {
    /// Returns the precompile name
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the precompile address
    pub const fn address(&self) -> Address {
        self.address
    }
}

/// TIP Fee Manager precompile identifier
pub const PRECOMPILE_ID_TIP_FEE_MANAGER: TempoPrecompileId =
    TempoPrecompileId { name: TIP_FEE_MANAGER_LABEL, address: TIP_FEE_MANAGER_ADDRESS };

/// PathUSD precompile identifier
pub const PRECOMPILE_ID_PATH_USD: TempoPrecompileId =
    TempoPrecompileId { name: PATH_USD_LABEL, address: PATH_USD_ADDRESS };

/// TIP403 Registry precompile identifier
pub const PRECOMPILE_ID_TIP403_REGISTRY: TempoPrecompileId =
    TempoPrecompileId { name: TIP403_REGISTRY_LABEL, address: TIP403_REGISTRY_ADDRESS };

/// TIP20 Factory precompile identifier
pub const PRECOMPILE_ID_TIP20_FACTORY: TempoPrecompileId =
    TempoPrecompileId { name: TIP20_FACTORY_LABEL, address: TIP20_FACTORY_ADDRESS };

/// TIP20 Rewards Registry precompile identifier
pub const PRECOMPILE_ID_TIP20_REWARDS_REGISTRY: TempoPrecompileId = TempoPrecompileId {
    name: TIP20_REWARDS_REGISTRY_LABEL,
    address: TIP20_REWARDS_REGISTRY_ADDRESS,
};

/// TIP Account Registrar precompile identifier
pub const PRECOMPILE_ID_TIP_ACCOUNT_REGISTRAR: TempoPrecompileId =
    TempoPrecompileId { name: TIP_ACCOUNT_REGISTRAR_LABEL, address: TIP_ACCOUNT_REGISTRAR };

/// Stablecoin Exchange precompile identifier
pub const PRECOMPILE_ID_STABLECOIN_EXCHANGE: TempoPrecompileId =
    TempoPrecompileId { name: STABLECOIN_EXCHANGE_LABEL, address: STABLECOIN_EXCHANGE_ADDRESS };

/// Nonce Manager precompile identifier
pub const PRECOMPILE_ID_NONCE_PRECOMPILE: TempoPrecompileId =
    TempoPrecompileId { name: NONCE_PRECOMPILE_LABEL, address: NONCE_PRECOMPILE_ADDRESS };

/// Validator Config precompile identifier
pub const PRECOMPILE_ID_VALIDATOR_CONFIG: TempoPrecompileId =
    TempoPrecompileId { name: VALIDATOR_CONFIG_LABEL, address: VALIDATOR_CONFIG_ADDRESS };

/// Account Keychain precompile identifier
pub const PRECOMPILE_ID_ACCOUNT_KEYCHAIN: TempoPrecompileId =
    TempoPrecompileId { name: ACCOUNT_KEYCHAIN_LABEL, address: ACCOUNT_KEYCHAIN_ADDRESS };

/// All Tempo precompile identifiers
pub const TEMPO_PRECOMPILES: &[&TempoPrecompileId] = &[
    &PRECOMPILE_ID_TIP_FEE_MANAGER,
    &PRECOMPILE_ID_PATH_USD,
    &PRECOMPILE_ID_TIP403_REGISTRY,
    &PRECOMPILE_ID_TIP20_FACTORY,
    &PRECOMPILE_ID_TIP20_REWARDS_REGISTRY,
    &PRECOMPILE_ID_TIP_ACCOUNT_REGISTRAR,
    &PRECOMPILE_ID_STABLECOIN_EXCHANGE,
    &PRECOMPILE_ID_NONCE_PRECOMPILE,
    &PRECOMPILE_ID_VALIDATOR_CONFIG,
    &PRECOMPILE_ID_ACCOUNT_KEYCHAIN,
];
