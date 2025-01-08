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

/// Context for [CheatcodesStrategy].
pub trait CheatcodesStrategyContext: Debug + Send + Sync + Any {
    /// Clone the strategy context.
    fn new_cloned(&self) -> Box<dyn CheatcodesStrategyContext>;
    /// Alias as immutable reference of [Any].
    fn as_any_ref(&self) -> &dyn Any;
    /// Alias as mutable reference of [Any].
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl Clone for Box<dyn CheatcodesStrategyContext> {
    fn clone(&self) -> Self {
        self.new_cloned()
    }
}

/// Default strategy context object.
impl CheatcodesStrategyContext for () {
    fn new_cloned(&self) -> Box<dyn CheatcodesStrategyContext> {
        Box::new(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }
}

/// Stateless strategy runner for [CheatcodesStrategy].
pub trait CheatcodesStrategyRunner: Debug + Send + Sync {
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
    fn base_contract_deployed(&self, _ctx: &mut dyn CheatcodesStrategyContext) {}

    /// Record broadcastable transaction during CREATE.
    fn record_broadcastable_create_transactions(
        &self,
        _ctx: &mut dyn CheatcodesStrategyContext,
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
        _ctx: &mut dyn CheatcodesStrategyContext,
        config: Arc<CheatsConfig>,
        input: &CallInputs,
        ecx_inner: InnerEcx,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
        active_delegation: &mut Option<SignedAuthorization>,
    );

    /// Hook for pre initialize_interp.
    fn pre_initialize_interp(
        &self,
        _ctx: &mut dyn CheatcodesStrategyContext,
        _interpreter: &mut Interpreter,
        _ecx: Ecx,
    ) {
    }

    /// Hook for post initialize_interp.
    fn post_initialize_interp(
        &self,
        _ctx: &mut dyn CheatcodesStrategyContext,
        _interpreter: &mut Interpreter,
        _ecx: Ecx,
    ) {
    }

    /// Hook for pre step_end.
    ///
    /// Used to override opcode behaviors. Returns true if handled.
    fn pre_step_end(
        &self,
        _ctx: &mut dyn CheatcodesStrategyContext,
        _interpreter: &mut Interpreter,
        _ecx: Ecx,
    ) -> bool {
        false
    }
}

/// Implements [CheatcodesStrategyRunner] for EVM.
#[derive(Debug, Default, Clone)]
pub struct EvmCheatcodesStrategyRunner;

impl CheatcodesStrategyRunner for EvmCheatcodesStrategyRunner {
    fn record_broadcastable_create_transactions(
        &self,
        _ctx: &mut dyn CheatcodesStrategyContext,
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
        _ctx: &mut dyn CheatcodesStrategyContext,
        _config: Arc<CheatsConfig>,
        call: &CallInputs,
        ecx_inner: InnerEcx,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
        active_delegation: &mut Option<SignedAuthorization>,
    ) {
        let is_fixed_gas_limit = check_if_fixed_gas_limit(ecx_inner, call.gas_limit);

        let account = ecx_inner.journaled_state.state().get_mut(&broadcast.new_origin).unwrap();

        let mut tx_req = TransactionRequest {
            from: Some(broadcast.new_origin),
            to: Some(TxKind::from(Some(call.target_address))),
            value: call.transfer_value(),
            input: TransactionInput::new(call.input.clone()),
            nonce: Some(account.info.nonce),
            chain_id: Some(ecx_inner.env.cfg.chain_id),
            gas: if is_fixed_gas_limit { Some(call.gas_limit) } else { None },
            ..Default::default()
        };

        if let Some(auth_list) = active_delegation.take() {
            tx_req.authorization_list = Some(vec![auth_list]);
        } else {
            tx_req.authorization_list = None;
        }

        broadcastable_transactions.push_back(BroadcastableTransaction {
            rpc: ecx_inner.db.active_fork_url(),
            transaction: tx_req.into(),
        });
        debug!(target: "cheatcodes", tx=?broadcastable_transactions.back().unwrap(), "broadcastable call");
    }
}

/// Defines the strategy for [Cheatcodes].
#[derive(Debug)]
pub struct CheatcodesStrategy {
    /// Strategy runner.
    pub runner: &'static dyn CheatcodesStrategyRunner,
    /// Strategy context.
    pub context: Box<dyn CheatcodesStrategyContext>,
}

impl CheatcodesStrategy {
    /// Creates a new EVM strategy for the [Cheatcodes].
    pub fn new_evm() -> Self {
        Self { runner: &EvmCheatcodesStrategyRunner, context: Box::new(()) }
    }
}

impl Clone for CheatcodesStrategy {
    fn clone(&self) -> Self {
        Self { runner: self.runner, context: self.context.new_cloned() }
    }
}
