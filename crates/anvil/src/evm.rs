use std::fmt::Debug;

use alloy_evm::{
    eth::EthEvmContext,
    precompiles::{DynPrecompile, PrecompilesMap},
    Database, Evm,
};
use foundry_evm_core::either_evm::EitherEvm;
use op_revm::OpContext;
use revm::{precompile::PrecompileWithAddress, Inspector};

/// Object-safe trait that enables injecting extra precompiles when using
/// `anvil` as a library.
pub trait PrecompileFactory: Send + Sync + Unpin + Debug {
    /// Returns a set of precompiles to extend the EVM with.
    fn precompiles(&self) -> Vec<PrecompileWithAddress>;
}

/// Inject precompiles into the EVM dynamically.
pub fn inject_precompiles<DB, I>(
    evm: &mut EitherEvm<DB, I, PrecompilesMap>,
    precompiles: Vec<PrecompileWithAddress>,
) where
    DB: Database,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
{
    for p in precompiles {
        evm.precompiles_mut()
            .apply_precompile(p.address(), |_| Some(DynPrecompile::from(*p.precompile())));
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use alloy_evm::{eth::EthEvmContext, precompiles::PrecompilesMap, EthEvm, Evm, EvmEnv};
    use alloy_op_evm::OpEvm;
    use alloy_primitives::{address, Address, Bytes, TxKind};
    use foundry_evm_core::either_evm::EitherEvm;
    use itertools::Itertools;
    use op_revm::{precompiles::OpPrecompiles, L1BlockInfo, OpContext, OpSpecId, OpTransaction};
    use revm::{
        context::{CfgEnv, Evm as RevmEvm, JournalTr, LocalContext, TxEnv},
        database::{EmptyDB, EmptyDBTyped},
        handler::{instructions::EthInstructions, EthPrecompiles},
        inspector::NoOpInspector,
        interpreter::interpreter::EthInterpreter,
        precompile::{
            PrecompileOutput, PrecompileResult, PrecompileSpecId, PrecompileWithAddress,
            Precompiles,
        },
        primitives::hardfork::SpecId,
        Journal,
    };

    use crate::{inject_precompiles, PrecompileFactory};

    // A precompile activated in the `Prague` spec.
    const ETH_PRAGUE_PRECOMPILE: Address = address!("0x0000000000000000000000000000000000000011");

    // A precompile activated in the `Fjord` spec.
    const OP_FROJD_PRECOMPILE: Address = address!("0x0000000000000000000000000000000000000100");

    // A custom precompile address and payload for testing.
    const PRECOMPILE_ADDR: Address = address!("0x0000000000000000000000000000000000000071");
    const PAYLOAD: &[u8] = &[0xde, 0xad, 0xbe, 0xef];

    #[derive(Debug)]
    struct CustomPrecompileFactory;

    impl PrecompileFactory for CustomPrecompileFactory {
        fn precompiles(&self) -> Vec<PrecompileWithAddress> {
            vec![PrecompileWithAddress::from((
                PRECOMPILE_ADDR,
                custom_echo_precompile as fn(&[u8], u64) -> PrecompileResult,
            ))]
        }
    }

    /// Custom precompile that echoes the input data.
    /// In this example it uses `0xdeadbeef` as the input data, returning it as output.
    fn custom_echo_precompile(input: &[u8], _gas_limit: u64) -> PrecompileResult {
        Ok(PrecompileOutput { bytes: Bytes::copy_from_slice(input), gas_used: 0 })
    }

    /// Creates a new EVM instance with the custom precompile factory.
    fn create_eth_evm(
        spec: SpecId,
    ) -> (foundry_evm::Env, EitherEvm<EmptyDBTyped<Infallible>, NoOpInspector, PrecompilesMap>)
    {
        let eth_env = foundry_evm::Env {
            evm_env: EvmEnv { block_env: Default::default(), cfg_env: CfgEnv::new_with_spec(spec) },
            tx: TxEnv {
                kind: TxKind::Call(PRECOMPILE_ADDR),
                data: PAYLOAD.into(),
                ..Default::default()
            },
        };

        let eth_evm_context = EthEvmContext {
            journaled_state: Journal::new(EmptyDB::default()),
            block: eth_env.evm_env.block_env.clone(),
            cfg: eth_env.evm_env.cfg_env.clone(),
            tx: eth_env.tx.clone(),
            chain: (),
            local: LocalContext::default(),
            error: Ok(()),
        };

        let eth_precompiles = EthPrecompiles {
            precompiles: Precompiles::new(PrecompileSpecId::from_spec_id(spec)),
            spec,
        }
        .precompiles;
        let eth_evm = EitherEvm::Eth(EthEvm::new(
            RevmEvm::new_with_inspector(
                eth_evm_context,
                NoOpInspector,
                EthInstructions::<EthInterpreter, EthEvmContext<EmptyDB>>::default(),
                PrecompilesMap::from_static(eth_precompiles),
            ),
            true,
        ));

        (eth_env, eth_evm)
    }

    /// Creates a new OP EVM instance with the custom precompile factory.
    fn create_op_evm(
        spec: SpecId,
        op_spec: OpSpecId,
    ) -> (
        crate::eth::backend::env::Env,
        EitherEvm<EmptyDBTyped<Infallible>, NoOpInspector, PrecompilesMap>,
    ) {
        let op_env = crate::eth::backend::env::Env {
            evm_env: EvmEnv { block_env: Default::default(), cfg_env: CfgEnv::new_with_spec(spec) },
            tx: OpTransaction::<TxEnv> {
                base: TxEnv {
                    kind: TxKind::Call(PRECOMPILE_ADDR),
                    data: PAYLOAD.into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            is_optimism: true,
        };

        let op_cfg = op_env.evm_env.cfg_env.clone().with_spec(op_spec);
        let op_evm_context = OpContext {
            journaled_state: {
                let mut journal = Journal::new(EmptyDB::default());
                // Converting SpecId into OpSpecId
                journal.set_spec_id(op_env.evm_env.cfg_env.spec);
                journal
            },
            block: op_env.evm_env.block_env.clone(),
            cfg: op_cfg.clone(),
            tx: op_env.tx.clone(),
            chain: L1BlockInfo::default(),
            local: LocalContext::default(),
            error: Ok(()),
        };

        let op_precompiles = OpPrecompiles::new_with_spec(op_cfg.spec).precompiles();
        let op_evm = EitherEvm::Op(OpEvm::new(
            op_revm::OpEvm(RevmEvm::new_with_inspector(
                op_evm_context,
                NoOpInspector,
                EthInstructions::<EthInterpreter, OpContext<EmptyDB>>::default(),
                PrecompilesMap::from_static(op_precompiles),
            )),
            true,
        ));

        (op_env, op_evm)
    }

    #[test]
    fn build_eth_evm_with_extra_precompiles_default_spec() {
        let (env, mut evm) = create_eth_evm(SpecId::default());

        // Check that the Prague precompile IS present when using the default spec.
        assert!(evm.precompiles_mut().addresses().contains(&ETH_PRAGUE_PRECOMPILE));

        assert!(!evm.precompiles_mut().addresses().contains(&PRECOMPILE_ADDR));

        inject_precompiles(&mut evm, CustomPrecompileFactory.precompiles());

        assert!(evm.precompiles_mut().addresses().contains(&PRECOMPILE_ADDR));

        let result = match &mut evm {
            EitherEvm::Eth(eth_evm) => eth_evm.transact(env.tx).unwrap(),
            _ => unreachable!(),
        };

        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }

    #[test]
    fn build_eth_evm_with_extra_precompiles_london_spec() {
        let (env, mut evm) = create_eth_evm(SpecId::LONDON);

        // Check that the Prague precompile IS NOT present when using the London spec.
        assert!(!evm.precompiles_mut().addresses().contains(&ETH_PRAGUE_PRECOMPILE));

        assert!(!evm.precompiles_mut().addresses().contains(&PRECOMPILE_ADDR));

        inject_precompiles(&mut evm, CustomPrecompileFactory.precompiles());

        assert!(evm.precompiles_mut().addresses().contains(&PRECOMPILE_ADDR));

        let result = match &mut evm {
            EitherEvm::Eth(eth_evm) => eth_evm.transact(env.tx).unwrap(),
            _ => unreachable!(),
        };

        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }

    #[test]
    fn build_op_evm_with_extra_precompiles_default_spec() {
        let (env, mut evm) = create_op_evm(
            SpecId::default(),
            // TODO: OpSpecId::ISTHMUS is not yet supported, fails with: `Missing operator fee
            // scalar for isthmus L1 Block`.
            OpSpecId::HOLOCENE,
        );

        // Check that the Fjord precompile IS present when using the default spec.
        assert!(evm.precompiles_mut().addresses().contains(&OP_FROJD_PRECOMPILE));

        // Check that the Prague precompile is NOT present when using the default spec.
        assert!(!evm.precompiles_mut().addresses().contains(&ETH_PRAGUE_PRECOMPILE));

        assert!(!evm.precompiles_mut().addresses().contains(&PRECOMPILE_ADDR));

        inject_precompiles(&mut evm, CustomPrecompileFactory.precompiles());

        assert!(evm.precompiles_mut().addresses().contains(&PRECOMPILE_ADDR));

        let result = match &mut evm {
            EitherEvm::Op(op_evm) => op_evm.transact(env.tx).unwrap(),
            _ => unreachable!(),
        };

        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }

    #[test]
    fn build_op_evm_with_extra_precompiles_bedrock_spec() {
        let (env, mut evm) = create_op_evm(SpecId::default(), OpSpecId::BEDROCK);

        // Check that the Fjord precompile IS NOT present when using the `OpSpecId::BEDROCK` spec.
        assert!(!evm.precompiles_mut().addresses().contains(&OP_FROJD_PRECOMPILE));

        // Check that the Prague precompile IS NOT present when using the `OpSpecId::BEDROCK` spec.
        assert!(!evm.precompiles_mut().addresses().contains(&ETH_PRAGUE_PRECOMPILE));

        assert!(!evm.precompiles_mut().addresses().contains(&PRECOMPILE_ADDR));

        inject_precompiles(&mut evm, CustomPrecompileFactory.precompiles());

        assert!(evm.precompiles_mut().addresses().contains(&PRECOMPILE_ADDR));

        let result = match &mut evm {
            EitherEvm::Op(op_evm) => op_evm.transact(env.tx).unwrap(),
            _ => unreachable!(),
        };

        assert!(result.result.is_success());
        assert_eq!(result.result.output(), Some(&PAYLOAD.into()));
    }
}
