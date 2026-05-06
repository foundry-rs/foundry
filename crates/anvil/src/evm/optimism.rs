//! Optimism-specific EVM helpers.

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::super::tests::{
        CustomPrecompileFactory, ETH_PRAGUE_PRECOMPILE, PAYLOAD, PRECOMPILE_ADDR,
    };
    use crate::PrecompileFactory;
    use alloy_evm::{Evm, EvmEnv, EvmFactory, precompiles::PrecompilesMap};
    use alloy_op_evm::{OpEvm, OpEvmFactory, OpTx};
    use alloy_primitives::{Address, TxKind, U256, address};
    use itertools::Itertools;
    use op_revm::{OpSpecId, OpTransaction};
    use revm::{
        context::{BlockEnv, CfgEnv, TxEnv},
        database::{EmptyDB, EmptyDBTyped},
        inspector::NoOpInspector,
        primitives::hardfork::SpecId,
    };

    // A precompile activated in the `Isthmus` spec.
    const OP_ISTHMUS_PRECOMPILE: Address = address!("0x0000000000000000000000000000000000000100");

    /// Creates a new OP EVM instance.
    fn create_op_evm(
        _spec: SpecId,
        op_spec: OpSpecId,
    ) -> (OpTx, OpEvm<EmptyDBTyped<Infallible>, NoOpInspector, PrecompilesMap, OpTx>) {
        let tx = OpTx(OpTransaction::<TxEnv> {
            base: TxEnv {
                kind: TxKind::Call(PRECOMPILE_ADDR),
                data: PAYLOAD.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let mut evm = OpEvmFactory::<OpTx>::default().create_evm_with_inspector(
            EmptyDB::default(),
            EvmEnv::new(CfgEnv::new_with_spec(op_spec), BlockEnv::default()),
            NoOpInspector,
        );

        if op_spec == OpSpecId::ISTHMUS {
            evm.ctx_mut().chain.operator_fee_constant = Some(U256::ZERO);
            evm.ctx_mut().chain.operator_fee_scalar = Some(U256::ZERO);
        }

        (tx, evm)
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
