use alloy_evm::precompiles::DynPrecompile;
use alloy_primitives::Address;
use std::fmt::Debug;

/// Object-safe trait that enables injecting extra precompiles when using
/// `anvil` as a library.
pub trait PrecompileFactory: Send + Sync + Unpin + Debug {
    /// Returns a set of precompiles to extend the EVM with.
    fn precompiles(&self) -> Vec<(Address, DynPrecompile)>;
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
    use alloy_op_evm::{OpEvm, OpTx};
    use alloy_primitives::{Address, Bytes, TxKind, U256, address};
    use itertools::Itertools;
    use op_revm::{L1BlockInfo, OpContext, OpSpecId, OpTransaction, precompiles::OpPrecompiles};
    use revm::{
        Journal,
        context::{BlockEnv, CfgEnv, Evm as RevmEvm, JournalTr, LocalContext, TxEnv},
        database::{EmptyDB, EmptyDBTyped},
        handler::{EthPrecompiles, instructions::EthInstructions},
        inspector::NoOpInspector,
        interpreter::interpreter::EthInterpreter,
        precompile::{PrecompileOutput, PrecompileSpecId, Precompiles},
        primitives::hardfork::SpecId,
    };

    // A precompile activated in the `Prague` spec (BLS12-381 G2 map).
    const ETH_PRAGUE_PRECOMPILE: Address = address!("0x0000000000000000000000000000000000000011");

    // A precompile activated in the `Osaka` spec (EIP-7951).
    const ETH_OSAKA_PRECOMPILE: Address = address!("0x0000000000000000000000000000000000000100");

    // A precompile activated in the `Isthmus` spec.
    const OP_ISTHMUS_PRECOMPILE: Address = address!("0x0000000000000000000000000000000000000100");

    // A custom precompile address and payload for testing.
    const PRECOMPILE_ADDR: Address = address!("0x0000000000000000000000000000000000000071");
    const PAYLOAD: &[u8] = &[0xde, 0xad, 0xbe, 0xef];

    #[derive(Debug)]
    struct CustomPrecompileFactory;

    impl PrecompileFactory for CustomPrecompileFactory {
        fn precompiles(&self) -> Vec<(Address, DynPrecompile)> {
            use alloy_evm::precompiles::PrecompileInput;
            vec![(
                PRECOMPILE_ADDR,
                DynPrecompile::from(|input: PrecompileInput<'_>| {
                    Ok(PrecompileOutput {
                        bytes: Bytes::copy_from_slice(input.data),
                        gas_used: 0,
                        gas_refunded: 0,
                        reverted: false,
                    })
                }),
            )]
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

    /// Creates a new OP EVM instance.
    fn create_op_evm(
        spec: SpecId,
        op_spec: OpSpecId,
    ) -> (OpTx, OpEvm<EmptyDBTyped<Infallible>, NoOpInspector, PrecompilesMap, OpTx>) {
        let tx = OpTransaction::<TxEnv> {
            base: TxEnv {
                kind: TxKind::Call(PRECOMPILE_ADDR),
                data: PAYLOAD.into(),
                ..Default::default()
            },
            ..Default::default()
        };

        let mut chain = L1BlockInfo::default();

        if op_spec == OpSpecId::ISTHMUS {
            chain.operator_fee_constant = Some(U256::from(0));
            chain.operator_fee_scalar = Some(U256::from(0));
        }

        let op_cfg: CfgEnv<OpSpecId> = CfgEnv::new_with_spec(op_spec);
        let op_evm_context = OpContext {
            journaled_state: {
                let mut journal = Journal::new(EmptyDB::default());
                journal.set_spec_id(spec);
                journal
            },
            block: BlockEnv::default(),
            cfg: op_cfg.clone(),
            tx: tx.clone(),
            chain,
            local: LocalContext::default(),
            error: Ok(()),
        };

        let op_precompiles = OpPrecompiles::new_with_spec(op_cfg.spec).precompiles();
        let op_evm = OpEvm::new(
            op_revm::OpEvm(RevmEvm::new_with_inspector(
                op_evm_context,
                NoOpInspector,
                EthInstructions::<EthInterpreter, OpContext<EmptyDB>>::new_mainnet_with_spec(spec),
                PrecompilesMap::from_static(op_precompiles),
            )),
            true,
        );

        (OpTx(tx), op_evm)
    }

    #[test]
    fn build_eth_evm_with_extra_precompiles_osaka_spec() {
        let (tx_env, mut evm) = create_eth_evm(SpecId::OSAKA);

        assert!(evm.precompiles().addresses().contains(&ETH_OSAKA_PRECOMPILE));
        assert!(evm.precompiles().addresses().contains(&ETH_PRAGUE_PRECOMPILE));
        assert!(!evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        evm.precompiles_mut().extend_precompiles(CustomPrecompileFactory.precompiles());

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

        evm.precompiles_mut().extend_precompiles(CustomPrecompileFactory.precompiles());

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

        evm.precompiles_mut().extend_precompiles(CustomPrecompileFactory.precompiles());

        assert!(evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        let result = evm.transact(tx_env).unwrap();
        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }

    #[test]
    fn build_op_evm_with_extra_precompiles_isthmus_spec() {
        let (tx, mut evm) = create_op_evm(SpecId::OSAKA, OpSpecId::ISTHMUS);

        assert!(evm.precompiles().addresses().contains(&OP_ISTHMUS_PRECOMPILE));
        assert!(evm.precompiles().addresses().contains(&ETH_PRAGUE_PRECOMPILE));
        assert!(!evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        evm.precompiles_mut().extend_precompiles(CustomPrecompileFactory.precompiles());

        assert!(evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        let result = evm.transact(tx).unwrap();
        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }

    #[test]
    fn build_op_evm_with_extra_precompiles_bedrock_spec() {
        let (tx, mut evm) = create_op_evm(SpecId::OSAKA, OpSpecId::BEDROCK);

        assert!(!evm.precompiles().addresses().contains(&OP_ISTHMUS_PRECOMPILE));
        assert!(!evm.precompiles().addresses().contains(&ETH_PRAGUE_PRECOMPILE));
        assert!(!evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        evm.precompiles_mut().extend_precompiles(CustomPrecompileFactory.precompiles());

        assert!(evm.precompiles().addresses().contains(&PRECOMPILE_ADDR));

        let result = evm.transact(tx).unwrap();
        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }
}
