use alloy_evm::precompiles::{DynPrecompile, PrecompilesMap};
use alloy_primitives::Address;
use std::fmt::Debug;

#[cfg(feature = "optimism")]
mod optimism;

/// Object-safe trait that enables injecting extra precompiles when using
/// `anvil` as a library.
pub trait PrecompileFactory: Send + Sync + Unpin + Debug {
    /// Returns a set of precompiles to extend the EVM with.
    fn precompiles(&self) -> Vec<(Address, DynPrecompile)>;

    /// Installs precompiles into the EVM precompile map.
    fn install(&self, precompiles: &mut PrecompilesMap) {
        precompiles.extend_precompiles(self.precompiles());
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use crate::PrecompileFactory;
    use alloy_evm::{
        EthEvm, Evm,
        eth::EthEvmContext,
        precompiles::{DynPrecompile, PrecompilesMap},
    };
    use alloy_primitives::{Address, Bytes, TxKind, address};
    use itertools::Itertools;
    use revm::{
        Journal,
        context::{BlockEnv, CfgEnv, Evm as RevmEvm, JournalTr, LocalContext, TxEnv},
        database::{EmptyDB, EmptyDBTyped},
        handler::{EthPrecompiles, instructions::EthInstructions},
        inspector::NoOpInspector,
        interpreter::interpreter::EthInterpreter,
        precompile::{PrecompileOutput, PrecompileSpecId, PrecompileStatus, Precompiles},
        primitives::hardfork::SpecId,
    };

    // A precompile activated in the `Prague` spec (BLS12-381 G2 map).
    pub(super) const ETH_PRAGUE_PRECOMPILE: Address =
        address!("0x0000000000000000000000000000000000000011");

    // A precompile activated in the `Osaka` spec (EIP-7951).
    const ETH_OSAKA_PRECOMPILE: Address = address!("0x0000000000000000000000000000000000000100");

    // A custom precompile address and payload for testing.
    pub(super) const PRECOMPILE_ADDR: Address =
        address!("0x0000000000000000000000000000000000000071");
    const DYNAMIC_PRECOMPILE_ADDR: Address = address!("0xdead000000000000000000000000000000000071");
    const DYNAMIC_PRECOMPILE_PREFIX: [u8; 2] = [0xde, 0xad];
    pub(super) const PAYLOAD: &[u8] = &[0xde, 0xad, 0xbe, 0xef];

    fn echo_precompile() -> DynPrecompile {
        use alloy_evm::precompiles::PrecompileInput;
        DynPrecompile::from(|input: PrecompileInput<'_>| {
            Ok(PrecompileOutput {
                status: PrecompileStatus::Success,
                bytes: Bytes::copy_from_slice(input.data),
                gas_used: 0,
                gas_refunded: 0,
                state_gas_used: 0,
                reservoir: input.reservoir,
            })
        })
    }

    #[derive(Debug)]
    pub(super) struct CustomPrecompileFactory;

    impl PrecompileFactory for CustomPrecompileFactory {
        fn precompiles(&self) -> Vec<(Address, DynPrecompile)> {
            vec![(PRECOMPILE_ADDR, echo_precompile())]
        }
    }

    #[derive(Debug)]
    struct DynamicLookupPrecompileFactory;

    impl PrecompileFactory for DynamicLookupPrecompileFactory {
        fn precompiles(&self) -> Vec<(Address, DynPrecompile)> {
            Vec::new()
        }

        fn install(&self, precompiles: &mut PrecompilesMap) {
            precompiles.set_precompile_lookup(|address: &Address| {
                address.as_slice().starts_with(&DYNAMIC_PRECOMPILE_PREFIX).then(echo_precompile)
            });
        }
    }

    /// Creates a new Eth EVM instance.
    fn create_eth_evm(
        spec: SpecId,
    ) -> (TxEnv, EthEvm<EmptyDBTyped<Infallible>, NoOpInspector, PrecompilesMap>) {
        let tx_env = TxEnv {
            kind: TxKind::Call(PRECOMPILE_ADDR),
            data: PAYLOAD.into(),
            ..Default::default()
        };

        let eth_evm_context = EthEvmContext {
            journaled_state: Journal::new(EmptyDB::default()),
            block: BlockEnv::default(),
            cfg: CfgEnv::new_with_spec(spec),
            tx: tx_env.clone(),
            chain: (),
            local: LocalContext::default(),
            error: Ok(()),
        };

        let eth_precompiles = EthPrecompiles {
            precompiles: Precompiles::new(PrecompileSpecId::from_spec_id(spec)),
            spec,
        }
        .precompiles;
        let eth_evm = EthEvm::new(
            RevmEvm::new_with_inspector(
                eth_evm_context,
                NoOpInspector,
                EthInstructions::<EthInterpreter, EthEvmContext<EmptyDB>>::new_mainnet_with_spec(
                    spec,
                ),
                PrecompilesMap::from_static(eth_precompiles),
            ),
            true,
        );

        (tx_env, eth_evm)
    }

    #[test]
    fn build_eth_evm_with_extra_precompiles_osaka_spec() {
        let (tx_env, mut evm) = create_eth_evm(SpecId::OSAKA);

        assert!(evm.precompiles().addresses().contains(&ETH_OSAKA_PRECOMPILE));
        assert!(evm.precompiles().addresses().contains(&ETH_PRAGUE_PRECOMPILE));
        assert!(!evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        CustomPrecompileFactory.install(evm.precompiles_mut());

        assert!(evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        let result = evm.transact(tx_env).unwrap();
        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }

    #[test]
    fn build_eth_evm_with_extra_precompiles_london_spec() {
        let (tx_env, mut evm) = create_eth_evm(SpecId::LONDON);

        assert!(!evm.precompiles().addresses().contains(&ETH_OSAKA_PRECOMPILE));
        assert!(!evm.precompiles().addresses().contains(&ETH_PRAGUE_PRECOMPILE));
        assert!(!evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        CustomPrecompileFactory.install(evm.precompiles_mut());

        assert!(evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        let result = evm.transact(tx_env).unwrap();
        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }

    #[test]
    fn build_eth_evm_with_extra_precompiles_prague_spec() {
        let (tx_env, mut evm) = create_eth_evm(SpecId::PRAGUE);

        assert!(!evm.precompiles().addresses().contains(&ETH_OSAKA_PRECOMPILE));
        assert!(evm.precompiles().addresses().contains(&ETH_PRAGUE_PRECOMPILE));
        assert!(!evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        CustomPrecompileFactory.install(evm.precompiles_mut());

        assert!(evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        let result = evm.transact(tx_env).unwrap();
        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }

    #[test]
    fn factory_install_supports_dynamic_lookup() {
        let (mut tx_env, mut evm) = create_eth_evm(SpecId::PRAGUE);
        tx_env.kind = TxKind::Call(DYNAMIC_PRECOMPILE_ADDR);

        assert!(!evm.precompiles().addresses().contains(&DYNAMIC_PRECOMPILE_ADDR));
        assert!(evm.precompiles().get(&DYNAMIC_PRECOMPILE_ADDR).is_none());

        DynamicLookupPrecompileFactory.install(evm.precompiles_mut());

        assert!(!evm.precompiles().addresses().contains(&DYNAMIC_PRECOMPILE_ADDR));
        assert!(evm.precompiles().get(&DYNAMIC_PRECOMPILE_ADDR).is_some());

        let result = evm.transact(tx_env).unwrap();
        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }
}
