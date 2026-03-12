use alloy_evm::{eth::EthEvmContext, precompiles::PrecompilesMap};
use alloy_monad_evm::MonadPrecompilesMap;
use alloy_primitives::{Address, address};
use monad_revm::{MonadContext, MonadSpecId};
use op_revm::OpContext;
use revm::{Database, context::Cfg, handler::PrecompileProvider, interpreter::InterpreterResult};
use std::ops::{Deref, DerefMut};

/// The ECRecover precompile address.
pub const EC_RECOVER: Address = address!("0x0000000000000000000000000000000000000001");

/// The SHA-256 precompile address.
pub const SHA_256: Address = address!("0x0000000000000000000000000000000000000002");

/// The RIPEMD-160 precompile address.
pub const RIPEMD_160: Address = address!("0x0000000000000000000000000000000000000003");

/// The Identity precompile address.
pub const IDENTITY: Address = address!("0x0000000000000000000000000000000000000004");

/// The ModExp precompile address.
pub const MOD_EXP: Address = address!("0x0000000000000000000000000000000000000005");

/// The ECAdd precompile address.
pub const EC_ADD: Address = address!("0x0000000000000000000000000000000000000006");

/// The ECMul precompile address.
pub const EC_MUL: Address = address!("0x0000000000000000000000000000000000000007");

/// The ECPairing precompile address.
pub const EC_PAIRING: Address = address!("0x0000000000000000000000000000000000000008");

/// The Blake2F precompile address.
pub const BLAKE_2F: Address = address!("0x0000000000000000000000000000000000000009");

/// The PointEvaluation precompile address.
pub const POINT_EVALUATION: Address = address!("0x000000000000000000000000000000000000000a");

/// Precompile addresses.
pub const PRECOMPILES: &[Address] = &[
    EC_RECOVER,
    SHA_256,
    RIPEMD_160,
    IDENTITY,
    MOD_EXP,
    EC_ADD,
    EC_MUL,
    EC_PAIRING,
    BLAKE_2F,
    POINT_EVALUATION,
];

/// Shared Foundry precompile wrapper used across Eth, OP, and Monad EVMs.
///
/// Foundry's `EitherEvm` expects a single precompile provider type across all networks. Eth and
/// OP can use the stock `PrecompilesMap`, while Monad needs `MonadPrecompilesMap` for
/// `MonadJournal`-aware precompiles such as MIP-4's reserve-balance precompile.
#[derive(Clone, Debug)]
pub enum FoundryPrecompiles {
    /// Standard precompiles for Ethereum-like contexts.
    Standard(PrecompilesMap),
    /// Monad-aware precompiles for Monad contexts.
    Monad(MonadPrecompilesMap),
}

impl FoundryPrecompiles {
    /// Creates a new standard precompile wrapper.
    pub fn standard(precompiles: PrecompilesMap) -> Self {
        Self::Standard(precompiles)
    }

    /// Creates a new Monad precompile wrapper for the given Monad spec.
    pub fn monad(spec: MonadSpecId) -> Self {
        Self::Monad(MonadPrecompilesMap::new_with_spec(spec))
    }

    /// Returns all known precompile addresses for the active network.
    pub fn addresses(&self) -> Box<dyn Iterator<Item = Address> + '_> {
        match self {
            Self::Standard(precompiles) => Box::new(precompiles.addresses().copied()),
            Self::Monad(precompiles) => Box::new(precompiles.addresses()),
        }
    }

    /// Returns true if the address is a precompile for the active network.
    pub fn contains(&self, address: &Address) -> bool {
        match self {
            Self::Standard(precompiles) => precompiles.get(address).is_some(),
            Self::Monad(precompiles) => precompiles.contains(address),
        }
    }
}

impl Deref for FoundryPrecompiles {
    type Target = PrecompilesMap;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Standard(precompiles) => precompiles,
            Self::Monad(precompiles) => precompiles.deref(),
        }
    }
}

impl DerefMut for FoundryPrecompiles {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Standard(precompiles) => precompiles,
            Self::Monad(precompiles) => precompiles.deref_mut(),
        }
    }
}

impl<DB: Database + std::fmt::Debug> PrecompileProvider<EthEvmContext<DB>> for FoundryPrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: revm::primitives::hardfork::SpecId) -> bool {
        PrecompileProvider::<EthEvmContext<DB>>::set_spec(self.deref_mut(), spec)
    }

    fn run(
        &mut self,
        context: &mut EthEvmContext<DB>,
        inputs: &revm::interpreter::CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        PrecompileProvider::<EthEvmContext<DB>>::run(self.deref_mut(), context, inputs)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        Box::new(self.addresses())
    }

    fn contains(&self, address: &Address) -> bool {
        Self::contains(self, address)
    }
}

impl<DB: Database + std::fmt::Debug> PrecompileProvider<OpContext<DB>> for FoundryPrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: op_revm::OpSpecId) -> bool {
        PrecompileProvider::<OpContext<DB>>::set_spec(self.deref_mut(), spec)
    }

    fn run(
        &mut self,
        context: &mut OpContext<DB>,
        inputs: &revm::interpreter::CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        PrecompileProvider::<OpContext<DB>>::run(self.deref_mut(), context, inputs)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        Box::new(self.addresses())
    }

    fn contains(&self, address: &Address) -> bool {
        Self::contains(self, address)
    }
}

impl<DB: Database + std::fmt::Debug> PrecompileProvider<MonadContext<DB>> for FoundryPrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: MonadSpecId) -> bool {
        match self {
            Self::Standard(_) => {
                *self = Self::monad(spec);
                true
            }
            Self::Monad(precompiles) => {
                PrecompileProvider::<MonadContext<DB>>::set_spec(precompiles, spec)
            }
        }
    }

    fn run(
        &mut self,
        context: &mut MonadContext<DB>,
        inputs: &revm::interpreter::CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        if !matches!(self, Self::Monad(_)) {
            *self = Self::monad(context.cfg.spec());
        }

        match self {
            Self::Standard(_) => unreachable!("Monad contexts must use Monad precompiles"),
            Self::Monad(precompiles) => {
                PrecompileProvider::<MonadContext<DB>>::run(precompiles, context, inputs)
            }
        }
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        Box::new(self.addresses())
    }

    fn contains(&self, address: &Address) -> bool {
        Self::contains(self, address)
    }
}
