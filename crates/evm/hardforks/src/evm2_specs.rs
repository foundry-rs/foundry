//! [`ExecutionSpec`] for evm2 spec types.
//!
//! Each evm2 spec type has a REVM-era counterpart with the same hardfork names. Config parsing and
//! fork hardfork resolution delegate to that counterpart until the REVM types are removed, so both
//! engines resolve identical specs.

use crate::{ExecutionSpec, FoundryHardfork, FromEvmVersion};
use foundry_compilers::artifacts::EvmVersion;
use std::str::FromStr;

/// Implements [`FromEvmVersion`] and [`ExecutionSpec`] for `$new` through `$old`.
macro_rules! delegate_execution_spec {
    ($new:ty => $old:ty,to_old: $to_old:expr,to_new: $to_new:expr $(,)?) => {
        const _: () = {
            fn to_old() -> fn($new) -> Option<$old> {
                $to_old
            }

            fn to_new() -> fn($old) -> Option<$new> {
                $to_new
            }

            impl From<FoundryHardfork> for $new {
                fn from(hardfork: FoundryHardfork) -> Self {
                    to_new()(<$old>::from(hardfork)).unwrap_or_default()
                }
            }

            impl FromEvmVersion for $new {
                fn from_evm_version(version: EvmVersion) -> Self {
                    to_new()(<$old>::from_evm_version(version)).unwrap_or_default()
                }
            }

            impl ExecutionSpec for $new {
                fn evm_version_name(&self) -> String {
                    to_old()(*self)
                        .map_or_else(|| format!("{self:?}"), |old| old.evm_version_name())
                }

                fn from_network_hardfork(hardfork: &str) -> Option<Self> {
                    <$old>::from_network_hardfork(hardfork).and_then(to_new())
                }

                fn from_foundry_hardfork(hardfork: FoundryHardfork) -> Option<Self> {
                    <$old>::from_foundry_hardfork(hardfork).and_then(to_new())
                }

                fn historical_hardfork(chain_id: u64, timestamp: u64) -> Option<FoundryHardfork> {
                    <$old>::historical_hardfork(chain_id, timestamp)
                }

                fn fork_hardfork(
                    chain_id: u64,
                    timestamp: u64,
                    endpoint_hardfork: Option<FoundryHardfork>,
                ) -> Option<FoundryHardfork> {
                    <$old>::fork_hardfork(chain_id, timestamp, endpoint_hardfork)
                }

                fn reported_hardfork(self) -> Option<FoundryHardfork> {
                    to_old()(self).and_then(<$old>::reported_hardfork)
                }
            }
        };
    };
}

/// Converts between spec types that share hardfork names.
fn by_name<From: ToString, To: FromStr>(spec: From) -> Option<To> {
    To::from_str(&spec.to_string()).ok()
}

delegate_execution_spec!(
    evm2::SpecId => revm::primitives::hardfork::SpecId,
    to_old: |spec: evm2::SpecId| revm::primitives::hardfork::SpecId::try_from_u8(spec as u8),
    to_new: |spec: revm::primitives::hardfork::SpecId| evm2::SpecId::try_from_u32(spec as u32),
);

delegate_execution_spec!(
    tempo_hardfork_evm2::TempoHardfork => tempo_hardfork::TempoHardfork,
    to_old: by_name,
    to_new: by_name,
);

#[cfg(feature = "optimism")]
delegate_execution_spec!(
    op_evm2::OpSpecId => op_revm::OpSpecId,
    to_old: |spec: op_evm2::OpSpecId| op_revm::OpSpecId::from_str(&spec.to_string()).ok(),
    to_new: |spec: op_revm::OpSpecId| op_evm2::OpSpecId::from_str(<&str>::from(spec)).ok(),
);

#[cfg(feature = "monad")]
delegate_execution_spec!(
    monad_evm2::MonadHardfork => monad_revm::MonadHardfork,
    to_old: by_name,
    to_new: by_name,
);

#[cfg(feature = "base")]
delegate_execution_spec!(
    base_evm2::BaseSpecId => base_common_evm::BaseSpecId,
    to_old: |spec: base_evm2::BaseSpecId| Some(base_common_evm::BaseSpecId::new(spec.upgrade())),
    to_new: |spec: base_common_evm::BaseSpecId| Some(base_evm2::BaseSpecId::new(spec.upgrade())),
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evm_spec_id_from_str;

    #[test]
    fn evm2_specs_resolve_like_revm_specs() {
        for name in ["cancun", "prague", "osaka", "ethereum:prague"] {
            let old = evm_spec_id_from_str::<revm::primitives::hardfork::SpecId>(name).unwrap();
            let new = evm_spec_id_from_str::<evm2::SpecId>(name).unwrap();
            assert_eq!(new as u8, old as u8, "{name}");
            assert_eq!(new.evm_version_name(), old.evm_version_name(), "{name}");
        }
        for name in ["T3", "tempo:T3"] {
            let old = evm_spec_id_from_str::<tempo_hardfork::TempoHardfork>(name).unwrap();
            let new = evm_spec_id_from_str::<tempo_hardfork_evm2::TempoHardfork>(name).unwrap();
            assert_eq!(new.to_string(), old.to_string(), "{name}");
            assert_eq!(new.reported_hardfork(), old.reported_hardfork(), "{name}");
        }
    }

    #[cfg(feature = "optimism")]
    #[test]
    fn evm2_op_specs_resolve_like_revm_specs() {
        for name in ["prague", "isthmus", "optimism:jovian"] {
            let old = evm_spec_id_from_str::<op_revm::OpSpecId>(name).unwrap();
            let new = evm_spec_id_from_str::<op_evm2::OpSpecId>(name).unwrap();
            assert_eq!(new.to_string(), <&str>::from(old), "{name}");
        }
    }
}
