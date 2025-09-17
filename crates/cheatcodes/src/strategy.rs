use std::{any::Any, fmt::Debug, sync::Arc};

use alloy_consensus::BlobTransactionSidecar;
use alloy_primitives::TxKind;
use alloy_rpc_types::{SignedAuthorization, TransactionInput, TransactionRequest};
use revm::interpreter::{CallInputs, Interpreter};

use crate::{
    BroadcastableTransaction, BroadcastableTransactions, CheatcodesExecutor, CheatsConfig,
    CheatsCtxt, DynCheatcode, Result,
    inspector::{CommonCreateInput, Ecx, check_if_fixed_gas_limit},
    script::Broadcast,
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
pub trait CheatcodeInspectorStrategyRunner:
    Debug + Send + Sync + CheatcodeInspectorStrategyExt
{
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
        ecx: Ecx,
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
        ecx: Ecx,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
        active_delegations: Vec<SignedAuthorization>,
        active_blob_sidecar: Option<BlobTransactionSidecar>,
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
        ecx: Ecx,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
    ) {
        let is_fixed_gas_limit = check_if_fixed_gas_limit(&ecx, input.gas_limit());

        let to = None;
        let nonce: u64 = ecx.journaled_state.state()[&broadcast.new_origin].info.nonce;
        //drop the mutable borrow of account
        let call_init_code = input.init_code();
        let rpc = ecx.journaled_state.database.active_fork_url();

        broadcastable_transactions.push_back(BroadcastableTransaction {
            rpc,
            transaction: TransactionRequest {
                from: Some(broadcast.new_origin),
                to,
                value: Some(input.value()),
                input: TransactionInput::new(call_init_code),
                nonce: Some(nonce),
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
        inputs: &CallInputs,
        ecx: Ecx,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
        active_delegations: Vec<SignedAuthorization>,
        mut active_blob_sidecar: Option<BlobTransactionSidecar>,
    ) {
        let input = TransactionInput::new(inputs.input.bytes(ecx));
        let is_fixed_gas_limit = check_if_fixed_gas_limit(&ecx, inputs.gas_limit);

        let account = ecx.journaled_state.state().get_mut(&broadcast.new_origin).unwrap();
        let nonce = account.info.nonce;

        let mut tx_req = TransactionRequest {
            from: Some(broadcast.new_origin),
            to: Some(TxKind::from(Some(inputs.target_address))),
            value: inputs.transfer_value(),
            input,
            nonce: Some(nonce),
            chain_id: Some(ecx.cfg.chain_id),
            gas: if is_fixed_gas_limit { Some(inputs.gas_limit) } else { None },
            ..Default::default()
        };

        // Set active blob sidecar, if any.
        if let Some(blob_sidecar) = active_blob_sidecar.take()
            && active_delegations.is_empty()
        {
            use alloy_network::TransactionBuilder4844;
            // Ensure blob and delegation are not set for the same tx.
            tx_req.set_blob_sidecar(blob_sidecar);
        }

        if !active_delegations.is_empty() {
            for auth in &active_delegations {
                let Ok(authority) = auth.recover_authority() else {
                    continue;
                };
                if authority == broadcast.new_origin {
                    // Increment nonce of broadcasting account to reflect signed
                    // authorization.
                    account.info.nonce += 1;
                }
            }
            tx_req.authorization_list = Some(active_delegations);
        }

        broadcastable_transactions.push_back(BroadcastableTransaction {
            rpc: ecx.journaled_state.database.active_fork_url(),
            transaction: tx_req.into(),
        });
        debug!(target: "cheatcodes", tx=?broadcastable_transactions.back().unwrap(), "broadcastable call");
    }
}

impl CheatcodeInspectorStrategyExt for EvmCheatcodeInspectorStrategyRunner {}

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
}

impl Clone for CheatcodeInspectorStrategy {
    fn clone(&self) -> Self {
        Self { runner: self.runner, context: self.context.new_cloned() }
    }
}

/// Defined in revive-strategy
pub trait CheatcodeInspectorStrategyExt {
    fn revive_try_create(
        &self,
        _state: &mut crate::Cheatcodes,
        _ecx: Ecx,
        _input: &dyn CommonCreateInput,
        _executor: &mut dyn CheatcodesExecutor,
    ) -> Option<revm::interpreter::CreateOutcome> {
        None
    }

    fn revive_try_call(
        &self,
        _state: &mut crate::Cheatcodes,
        _ecx: Ecx,
        _input: &CallInputs,
        _executor: &mut dyn CheatcodesExecutor,
    ) -> Option<revm::interpreter::CallOutcome> {
        None
    }
}

// Legacy type aliases for backward compatibility
pub type CheatcodesStrategy = CheatcodeInspectorStrategy;
