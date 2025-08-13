use std::{any::Any, fmt::Debug, sync::Arc};

use alloy_primitives::TxKind;
use alloy_rpc_types::{TransactionInput, TransactionRequest};
use revm::{
    interpreter::{CallInputs, Interpreter},
    primitives::SignedAuthorization,
};

use crate::{
    inspector::{check_if_fixed_gas_limit, CommonCreateInput, Ecx, InnerEcx},
    script::Broadcast,
    BroadcastableTransaction, BroadcastableTransactions, CheatcodesExecutor, CheatsConfig,
    CheatsCtxt, DynCheatcode, Result,
};

/// Context for [CheatcodeInspectorStrategy].
pub trait CheatcodeInspectorStrategyContext: Debug + Send + Sync + Any {
    /// Clone the strategy context.
    fn new_cloned(&self) -> Box<dyn CheatcodeInspectorStrategyContext>;
    /// Alias as immutable reference of [Any].
    fn as_any_ref(&self) -> &dyn Any;
    /// Alias as mutable reference of [Any].
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl Clone for Box<dyn CheatcodeInspectorStrategyContext> {
    fn clone(&self) -> Self {
        self.new_cloned()
    }
}

/// Default strategy context object.
impl CheatcodeInspectorStrategyContext for () {
    fn new_cloned(&self) -> Box<dyn CheatcodeInspectorStrategyContext> {
        Box::new(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }
}

/// Stateless strategy runner for [CheatcodeInspectorStrategy].
pub trait CheatcodeInspectorStrategyRunner: Debug + Send + Sync {
    /// Apply cheatcodes.
    fn apply_full(
        &self,
        cheatcode: &dyn DynCheatcode,
        ccx: &mut CheatsCtxt,
        executor: &mut dyn CheatcodesExecutor,
    ) -> Result {
        cheatcode.dyn_apply(ccx, executor)
    }

    /// Called when the main test or script contract is deployed.
    fn base_contract_deployed(&self, _ctx: &mut dyn CheatcodeInspectorStrategyContext) {}

    /// Record broadcastable transaction during CREATE.
    fn record_broadcastable_create_transactions(
        &self,
        _ctx: &mut dyn CheatcodeInspectorStrategyContext,
        config: Arc<CheatsConfig>,
        input: &dyn CommonCreateInput,
        ecx_inner: InnerEcx,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
    );

    /// Record broadcastable transaction during CALL.
    #[allow(clippy::too_many_arguments)]
    fn record_broadcastable_call_transactions(
        &self,
        _ctx: &mut dyn CheatcodeInspectorStrategyContext,
        _config: Arc<CheatsConfig>,
        input: &CallInputs,
        ecx_inner: InnerEcx,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
        active_delegation: &mut Option<SignedAuthorization>,
    );

    /// Hook for pre initialize_interp.
    fn pre_initialize_interp(
        &self,
        _ctx: &mut dyn CheatcodeInspectorStrategyContext,
        _interpreter: &mut Interpreter,
        _ecx: Ecx,
    ) {
    }

    /// Hook for post initialize_interp.
    fn post_initialize_interp(
        &self,
        _ctx: &mut dyn CheatcodeInspectorStrategyContext,
        _interpreter: &mut Interpreter,
        _ecx: Ecx,
    ) {
    }

    /// Hook for pre step_end.
    ///
    /// Used to override opcode behaviors. Returns true if handled.
    fn pre_step_end(
        &self,
        _ctx: &mut dyn CheatcodeInspectorStrategyContext,
        _interpreter: &mut Interpreter,
        _ecx: Ecx,
    ) -> bool {
        false
    }
}

/// Implements [CheatcodeInspectorStrategyRunner] for EVM.
#[derive(Debug, Default, Clone)]
pub struct EvmCheatcodeInspectorStrategyRunner;

impl CheatcodeInspectorStrategyRunner for EvmCheatcodeInspectorStrategyRunner {
    fn record_broadcastable_create_transactions(
        &self,
        _ctx: &mut dyn CheatcodeInspectorStrategyContext,
        _config: Arc<CheatsConfig>,
        input: &dyn CommonCreateInput,
        ecx_inner: InnerEcx,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
    ) {
        let is_fixed_gas_limit = check_if_fixed_gas_limit(ecx_inner, input.gas_limit());

        let account = &ecx_inner.journaled_state.state()[&broadcast.new_origin];
        broadcastable_transactions.push_back(BroadcastableTransaction {
            rpc: ecx_inner.db.active_fork_url(),
            transaction: TransactionRequest {
                from: Some(broadcast.new_origin),
                to: None,
                value: Some(input.value()),
                input: TransactionInput::new(input.init_code()),
                nonce: Some(account.info.nonce),
                gas: if is_fixed_gas_limit { Some(input.gas_limit()) } else { None },
                ..Default::default()
            }
            .into(),
        });
    }

    fn record_broadcastable_call_transactions(
        &self,
        _ctx: &mut dyn CheatcodeInspectorStrategyContext,
        _config: Arc<CheatsConfig>,
        input: &CallInputs,
        ecx_inner: InnerEcx,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
        active_delegation: &mut Option<SignedAuthorization>,
    ) {
        let is_fixed_gas_limit = check_if_fixed_gas_limit(ecx_inner, input.gas_limit);

        let account = ecx_inner.journaled_state.state().get_mut(&broadcast.new_origin).unwrap();

        let mut tx_req = TransactionRequest {
            from: Some(broadcast.new_origin),
            to: Some(TxKind::from(Some(input.target_address))),
            value: input.transfer_value(),
            input: TransactionInput::new(input.input.clone()),
            nonce: Some(account.info.nonce),
            chain_id: Some(ecx_inner.env.cfg.chain_id),
            gas: if is_fixed_gas_limit { Some(input.gas_limit) } else { None },
            ..Default::default()
        };

        // Handle delegation if present
        if let Some(auth_list) = active_delegation.take() {
            tx_req.authorization_list = Some(vec![auth_list]);
            tx_req.sidecar = None;

            // Increment nonce to reflect the signed authorization.
            account.info.nonce += 1;
        } else {
            tx_req.authorization_list = None;
            tx_req.sidecar = None;
        }

        broadcastable_transactions.push_back(BroadcastableTransaction {
            rpc: ecx_inner.db.active_fork_url(),
            transaction: tx_req.into(),
        });
        debug!(target: "cheatcodes", tx=?broadcastable_transactions.back().unwrap(), "broadcastable call");
    }
}

/// Defines the strategy for [super::Cheatcodes].
#[derive(Debug)]
pub struct CheatcodeInspectorStrategy {
    /// Strategy runner.
    pub runner: &'static dyn CheatcodeInspectorStrategyRunner,
    /// Strategy context.
    pub context: Box<dyn CheatcodeInspectorStrategyContext>,
}

impl CheatcodeInspectorStrategy {
    /// Creates a new EVM strategy for the [super::Cheatcodes].
    pub fn new_evm() -> Self {
        Self { runner: &EvmCheatcodeInspectorStrategyRunner, context: Box::new(()) }
    }

    /// Creates a new PVM strategy for the [super::Cheatcodes].
    pub fn new_pvm() -> Self {
        Self {
            runner: &crate::pvm::PvmCheatcodeInspectorStrategyRunner,
            context: Box::new(crate::pvm::PvmCheatcodeInspectorStrategyContext::new()),
        }
    }
}

impl Clone for CheatcodeInspectorStrategy {
    fn clone(&self) -> Self {
        Self { runner: self.runner, context: self.context.new_cloned() }
    }
}

// Legacy type aliases for backward compatibility
pub type CheatcodesStrategy = CheatcodeInspectorStrategy;
