use alloy_primitives::Address;
use std::fmt::Debug;

pub use crate::eth::backend::evm::{DynPrecompile, PrecompileInput, PrecompileOutput};
pub use evm2::{
    interpreter::GasTracker,
    precompiles::{PrecompileError, PrecompileId, PrecompileResult},
};

/// Object-safe trait that enables injecting extra precompiles when using
/// `anvil` as a library.
///
/// The precompiles run on every network family: they receive the call input and charge gas
/// through evm2's [`GasTracker`].
pub trait PrecompileFactory: Send + Sync + Unpin + Debug {
    /// Returns a set of precompiles to extend the EVM with.
    fn precompiles(&self) -> Vec<(Address, PrecompileId, DynPrecompile)>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eth::backend::{
        db::{CacheDB, EmptyDb},
        env::{CfgEnv, EvmEnv},
        evm::{EthEvmNetwork, EvmOptions, TxPayload, build_evm, evm_tx},
    };
    use alloy_consensus::{SignableTransaction, TxLegacy};
    use alloy_primitives::{Bytes, Signature, TxKind};
    use foundry_evm_hardforks::{EthereumHardfork, FoundryHardfork};
    use foundry_primitives::FoundryTxEnvelope;
    use std::sync::Arc;

    // A precompile activated in the `Prague` spec (BLS12-381 G2 map).
    const ETH_PRAGUE_PRECOMPILE: Address = Address::with_last_byte(0x11);

    // A custom precompile address and payload for testing.
    const PRECOMPILE_ADDR: Address = Address::with_last_byte(0x71);
    const PAYLOAD: &[u8] = &[0xde, 0xad, 0xbe, 0xef];

    #[derive(Debug)]
    struct CustomPrecompileFactory;

    impl PrecompileFactory for CustomPrecompileFactory {
        fn precompiles(&self) -> Vec<(Address, PrecompileId, DynPrecompile)> {
            vec![(
                PRECOMPILE_ADDR,
                PrecompileId::custom("echo"),
                Arc::new(|input: PrecompileInput<'_>, gas: &mut GasTracker| {
                    gas.spend(10)?;
                    Ok(PrecompileOutput::new(Bytes::copy_from_slice(input.data)))
                }),
            )]
        }
    }

    fn call(hardfork: EthereumHardfork, factory: Option<Arc<dyn PrecompileFactory>>) -> Bytes {
        let db = CacheDB::new(EmptyDb);
        let mut env = EvmEnv::new(
            CfgEnv { spec: FoundryHardfork::Ethereum(hardfork).into(), ..Default::default() },
            Default::default(),
        );
        env.cfg_env.disable_base_fee = true;
        let opts = EvmOptions {
            hardfork: FoundryHardfork::Ethereum(hardfork),
            precompile_factory: factory,
            ..Default::default()
        };
        let mut evm = build_evm::<EthEvmNetwork>(&db, &env, &opts).unwrap();
        assert!(
            evm2::evm::precompile::PrecompileProvider::contains(
                evm.precompiles(),
                &ETH_PRAGUE_PRECOMPILE
            ) == (hardfork >= EthereumHardfork::Prague)
        );
        let tx = TxLegacy {
            gas_limit: 100_000,
            to: TxKind::Call(PRECOMPILE_ADDR),
            input: PAYLOAD.into(),
            ..Default::default()
        };
        let tx = FoundryTxEnvelope::Legacy(tx.into_signed(Signature::test_signature()));
        let tx = evm_tx::<EthEvmNetwork>(&tx, Address::ZERO, TxPayload::Simulated, None).unwrap();
        let result = evm.transact(&tx).unwrap().discard();
        assert!(result.status);
        result.output
    }

    #[test]
    fn build_eth_evm_with_extra_precompiles_osaka_spec() {
        let output = call(EthereumHardfork::Osaka, Some(Arc::new(CustomPrecompileFactory)));
        assert_eq!(output, Bytes::from_static(PAYLOAD));
    }

    #[test]
    fn build_eth_evm_with_extra_precompiles_prague_spec() {
        let output = call(EthereumHardfork::Prague, Some(Arc::new(CustomPrecompileFactory)));
        assert_eq!(output, Bytes::from_static(PAYLOAD));
    }

    #[test]
    fn build_eth_evm_with_extra_precompiles_london_spec() {
        let output = call(EthereumHardfork::London, Some(Arc::new(CustomPrecompileFactory)));
        assert_eq!(output, Bytes::from_static(PAYLOAD));
    }

    #[test]
    fn build_eth_evm_without_extra_precompiles() {
        let output = call(EthereumHardfork::Osaka, None);
        assert!(output.is_empty());
    }
}
