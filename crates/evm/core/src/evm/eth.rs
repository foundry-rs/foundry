use alloy_evm::{
    EthEvm, EthEvmFactory, Evm, EvmEnv, EvmFactory, eth::EthEvmContext, precompiles::PrecompilesMap,
};
use alloy_network::Ethereum;
use alloy_primitives::{Address, Bytes, map::AddressSet};
use foundry_evm_networks::{apply_bsc_p256_precompile, arc};
use foundry_fork_db::DatabaseError;
use revm::{
    context::{
        BlockEnv, CfgEnv, Evm as RevmEvm, Journal, TxEnv,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::{
        EthFrame, EvmTr, FrameResult, MainnetHandler, PrecompileProvider,
        instructions::EthInstructions,
    },
    inspector::InspectorHandler,
    interpreter::{
        CallInputs, FrameInput, InstructionResult, InterpreterResult, interpreter::EthInterpreter,
    },
    primitives::hardfork::SpecId,
};
use std::{
    ops::{Deref, DerefMut},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    evm::{
        FoundryEvmFactory, FoundryEvmNetwork, IntoInstructionResult, NestedEvm, NestedEvmFor,
        run_inspected_frame,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct EthEvmNetwork;
impl FoundryEvmNetwork for EthEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = EthEvmFactory;
}

type EthEvmHandler<'db, I> = MainnetHandler<EthRevmEvm<'db, I>, EVMError<DatabaseError>, EthFrame>;

pub type EthRevmEvm<'db, I> = RevmEvm<
    EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>,
    I,
    EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>,
    ForkAwarePrecompiles,
    EthFrame,
>;

impl IntoInstructionResult for HaltReason {
    fn into_instruction_result(self) -> InstructionResult {
        self.into()
    }
}

impl FoundryEvmFactory for EthEvmFactory {
    type Chain = ();
    type FoundryContext<'db> = EthEvmContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> = FoundryEthEvm<'db, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let chain_id = evm_env.cfg_env.chain_id;
        let timestamp = evm_env.block_env.timestamp.saturating_to();
        let mut eth_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        eth_evm.cfg.tx_chain_id_check = true;
        let networks = eth_evm.inspector().get_networks();
        networks.inject_precompiles(eth_evm.precompiles_mut());
        apply_bsc_p256_precompile(eth_evm.precompiles_mut(), chain_id, timestamp);
        let inner = ForkAwarePrecompiles::wrap_evm(eth_evm.into_inner(), chain_id);
        FoundryEthEvm(EthEvm::new(inner, true))
    }

    fn create_nested_evm_with_inspector<'db, I>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        inspector: I,
    ) -> NestedEvmFor<'db, Self>
    where
        I: FoundryInspectorExt<Self::FoundryContext<'db>> + 'db,
    {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).into_inner())
    }
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> NestedEvm
    for EthRevmEvm<'db, I>
{
    type Spec = SpecId;
    type Block = BlockEnv;
    type Tx = TxEnv;
    type Chain = ();
    type Journal = Journal<&'db mut dyn DatabaseExt<EthEvmFactory>>;

    fn tx_mut(&mut self) -> &mut Self::Tx {
        self.ctx_mut().tx_mut()
    }

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn chain_mut(&mut self) -> &mut Self::Chain {
        &mut self.ctx_mut().chain
    }

    fn precompiles_mut(&mut self) -> &mut alloy_evm::precompiles::PrecompilesMap {
        self.precompiles.chain_id.store(self.ctx.cfg.chain_id, Ordering::Relaxed);
        &mut self.precompiles.map
    }

    fn journal_mut(&mut self) -> &mut Self::Journal {
        &mut self.ctx_mut().journaled_state
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        run_inspected_frame(self, EthEvmHandler::<I>::default(), frame)
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> eyre::Result<ResultAndState> {
        self.set_tx(tx);

        let result = EthEvmHandler::<I>::default().inspect_run(self)?;

        Ok(ResultAndState::new(result, self.ctx.journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}

/// Ethereum EVM with a precompile registry that follows in-test fork changes.
pub struct FoundryEthEvm<'db, I>(
    EthEvm<&'db mut dyn DatabaseExt<EthEvmFactory>, I, ForkAwarePrecompiles>,
);

impl<'db, I> FoundryEthEvm<'db, I> {
    fn into_inner(self) -> EthRevmEvm<'db, I> {
        self.0.into_inner()
    }
}

impl<'db, I> Deref for FoundryEthEvm<'db, I> {
    type Target = EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<I> DerefMut for FoundryEthEvm<'_, I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> Evm
    for FoundryEthEvm<'db, I>
{
    type DB = &'db mut dyn DatabaseExt<EthEvmFactory>;
    type Tx = TxEnv;
    type Error = EVMError<DatabaseError>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type BlockEnv = BlockEnv;
    type Precompiles = PrecompilesMap;
    type Inspector = I;

    fn block(&self) -> &BlockEnv {
        self.0.block()
    }

    fn cfg_env(&self) -> &CfgEnv {
        self.0.cfg_env()
    }

    fn chain_id(&self) -> u64 {
        self.0.chain_id()
    }

    fn transact_raw(&mut self, tx: TxEnv) -> Result<ResultAndState, Self::Error> {
        self.0.transact_raw(tx)
    }

    fn transact_system_call(
        &mut self,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) -> Result<ResultAndState, Self::Error> {
        self.0.transact_system_call(caller, contract, data)
    }

    fn finish(self) -> (Self::DB, EvmEnv) {
        self.0.finish()
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        self.0.set_inspector_enabled(enabled);
    }

    fn components(&self) -> (&Self::DB, &I, &PrecompilesMap) {
        let (db, inspector, precompiles) = self.0.components();
        precompiles.chain_id.store(self.0.chain_id(), Ordering::Relaxed);
        (db, inspector, &precompiles.map)
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut I, &mut PrecompilesMap) {
        let chain_id = self.0.chain_id();
        let (db, inspector, precompiles) = self.0.components_mut();
        precompiles.chain_id.store(chain_id, Ordering::Relaxed);
        (db, inspector, &mut precompiles.map)
    }
}

/// Keeps cold Arc lookups scoped to the current context without replacing configured overrides.
#[derive(Debug)]
pub struct ForkAwarePrecompiles {
    map: PrecompilesMap,
    // Shared with the map's owned lookup, whose API only receives the requested address.
    chain_id: Arc<AtomicU64>,
}

impl ForkAwarePrecompiles {
    fn new(mut map: PrecompilesMap, chain_id: u64) -> Self {
        let chain_id = Arc::new(AtomicU64::new(chain_id));
        let lookup_chain_id = Arc::clone(&chain_id);
        map.map_precompile_lookup(move |address, previous| {
            arc::precompile(lookup_chain_id.load(Ordering::Relaxed), address)
                .or_else(|| previous.and_then(|lookup| lookup.lookup(address)))
        });
        Self { map, chain_id }
    }

    fn wrap_evm<CTX, INSP, INST, FRAME>(
        evm: RevmEvm<CTX, INSP, INST, PrecompilesMap, FRAME>,
        chain_id: u64,
    ) -> RevmEvm<CTX, INSP, INST, Self, FRAME> {
        let RevmEvm { ctx, inspector, instruction, precompiles, frame_stack } = evm;
        RevmEvm {
            ctx,
            inspector,
            instruction,
            precompiles: Self::new(precompiles, chain_id),
            frame_stack,
        }
    }
}

impl<DB: alloy_evm::Database> PrecompileProvider<EthEvmContext<DB>> for ForkAwarePrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: SpecId) -> bool {
        <PrecompilesMap as PrecompileProvider<EthEvmContext<DB>>>::set_spec(&mut self.map, spec)
    }

    fn run(
        &mut self,
        context: &mut EthEvmContext<DB>,
        inputs: &CallInputs,
    ) -> Result<Option<InterpreterResult>, String> {
        // Fork cheatcodes can replace the chain ID without constructing another EVM.
        // Refresh before resolving the call so leaving Arc also restores ordinary bytecode.
        self.chain_id.store(context.cfg.chain_id, Ordering::Relaxed);
        self.map.run(context, inputs)
    }

    fn warm_addresses(&self) -> &AddressSet {
        <PrecompilesMap as PrecompileProvider<EthEvmContext<DB>>>::warm_addresses(&self.map)
    }

    fn contains(&self, address: &Address) -> bool {
        self.map.get(address).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_evm::precompiles::DynPrecompile;
    use alloy_primitives::{TxKind, U256};
    use alloy_sol_types::{SolCall, SolValue, sol};
    use revm::{
        database::InMemoryDB,
        precompile::{PrecompileId, PrecompileOutput},
    };

    sol! {
        function isBlocklisted(address account) external view returns (bool);
    }

    #[test]
    fn arc_precompile_lookup_follows_chain_changes_and_preserves_previous_lookup() {
        let mut inner = EthEvmFactory::default()
            .create_evm(InMemoryDB::default(), EvmEnv::default())
            .into_inner();
        inner.precompiles.set_precompile_lookup(|address: &Address| {
            (*address == arc::NATIVE_COIN_CONTROL).then(|| {
                DynPrecompile::new(PrecompileId::Custom("existing".into()), |input| {
                    Ok(PrecompileOutput::new(
                        0,
                        U256::from(0x42).abi_encode().into(),
                        input.reservoir,
                    ))
                })
            })
        });
        let inner = ForkAwarePrecompiles::wrap_evm(inner, 31337);
        let mut evm = EthEvm::new(inner, false);
        for chain_id in [31337, 5_042_002, 1, 5_042, 31337] {
            evm.cfg.chain_id = chain_id;
            let result = evm
                .transact(TxEnv {
                    kind: TxKind::Call(arc::NATIVE_COIN_CONTROL),
                    data: isBlocklistedCall { account: Address::with_last_byte(0x42) }
                        .abi_encode()
                        .into(),
                    chain_id: Some(chain_id),
                    gas_limit: 100_000,
                    ..Default::default()
                })
                .unwrap();
            assert!(result.result.is_success());
            let expected = if matches!(chain_id, 5_042 | 5_042_002) { 0 } else { 0x42 };
            assert_eq!(
                result.result.output(),
                Some(&Bytes::from(U256::from(expected).abi_encode()))
            );
            assert!(
                !evm.precompiles()
                    .map
                    .addresses()
                    .any(|address| *address == arc::NATIVE_COIN_CONTROL)
            );
        }
    }

    #[test]
    fn arc_precompile_lookup_preserves_explicit_overrides() {
        let mut inner = EthEvmFactory::default()
            .create_evm(InMemoryDB::default(), EvmEnv::default())
            .into_inner();
        inner.precompiles.apply_precompile(&arc::NATIVE_COIN_CONTROL, |_| {
            Some(DynPrecompile::new(PrecompileId::Custom("override".into()), |input| {
                Ok(PrecompileOutput::new(0, U256::from(0x42).abi_encode().into(), input.reservoir))
            }))
        });
        let inner = ForkAwarePrecompiles::wrap_evm(inner, 31337);
        let mut evm = EthEvm::new(inner, false);
        for chain_id in [31337, 5_042_002, 31337] {
            evm.cfg.chain_id = chain_id;
            let result = evm
                .transact(TxEnv {
                    kind: TxKind::Call(arc::NATIVE_COIN_CONTROL),
                    chain_id: Some(chain_id),
                    gas_limit: 100_000,
                    ..Default::default()
                })
                .unwrap();
            assert!(result.result.is_success());
            assert_eq!(result.result.output(), Some(&Bytes::from(U256::from(0x42).abi_encode())));
        }
    }
}
