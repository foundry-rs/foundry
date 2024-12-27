use std::{any::Any, fmt::Debug, sync::Arc};

use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_rpc_types::{TransactionInput, TransactionRequest};
use alloy_sol_types::SolValue;
use revm::{
    interpreter::{CallInputs, InstructionResult, Interpreter},
    primitives::{Bytecode, SignedAuthorization, KECCAK_EMPTY},
};

use crate::{
    evm::{
        self, journaled_account,
        mock::{make_acc_non_empty, mock_call},
        DealRecord,
    },
    inspector::{check_if_fixed_gas_limit, CommonCreateInput, Ecx, InnerEcx},
    script::Broadcast,
    BroadcastableTransaction, BroadcastableTransactions, Cheatcodes, CheatsConfig, CheatsCtxt,
    Result,
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
    /// Strategy name used when printing.
    fn name(&self) -> &'static str;

    /// Clone the strategy runner.
    fn new_cloned(&self) -> Box<dyn CheatcodesStrategyRunner>;

    /// Get nonce.
    fn get_nonce(&self, ccx: &mut CheatsCtxt, address: Address) -> Result<u64> {
        let account = ccx.ecx.journaled_state.load_account(address, &mut ccx.ecx.db)?;
        Ok(account.info.nonce)
    }

    /// Called when the main test or script contract is deployed.
    fn base_contract_deployed(&self, _ctx: &mut dyn CheatcodesStrategyContext) {}

    /// Cheatcode: roll.
    fn cheatcode_roll(&self, ccx: &mut CheatsCtxt, new_height: U256) -> Result {
        ccx.ecx.env.block.number = new_height;
        Ok(Default::default())
    }

    /// Cheatcode: warp.
    fn cheatcode_warp(&self, ccx: &mut CheatsCtxt, new_timestamp: U256) -> Result {
        ccx.ecx.env.block.timestamp = new_timestamp;
        Ok(Default::default())
    }

    /// Cheatcode: deal.
    fn cheatcode_deal(&self, ccx: &mut CheatsCtxt, address: Address, new_balance: U256) -> Result {
        let account = journaled_account(ccx.ecx, address)?;
        let old_balance = std::mem::replace(&mut account.info.balance, new_balance);
        let record = DealRecord { address, old_balance, new_balance };
        ccx.state.eth_deals.push(record);
        Ok(Default::default())
    }

    /// Cheatcode: etch.
    fn cheatcode_etch(
        &self,
        ccx: &mut CheatsCtxt,
        target: Address,
        new_runtime_bytecode: &Bytes,
    ) -> Result {
        ensure_not_precompile!(&target, ccx);
        ccx.ecx.load_account(target)?;
        let bytecode = Bytecode::new_raw(Bytes::copy_from_slice(new_runtime_bytecode));
        ccx.ecx.journaled_state.set_code(target, bytecode);
        Ok(Default::default())
    }

    /// Cheatcode: getNonce.
    fn cheatcode_get_nonce(&self, ccx: &mut CheatsCtxt, address: Address) -> Result {
        evm::get_nonce(ccx, &address)
    }

    /// Cheatcode: resetNonce.
    fn cheatcode_reset_nonce(&self, ccx: &mut CheatsCtxt, account: Address) -> Result {
        let account = journaled_account(ccx.ecx, account)?;
        // Per EIP-161, EOA nonces start at 0, but contract nonces
        // start at 1. Comparing by code_hash instead of code
        // to avoid hitting the case where account's code is None.
        let empty = account.info.code_hash == KECCAK_EMPTY;
        let nonce = if empty { 0 } else { 1 };
        account.info.nonce = nonce;
        debug!(target: "cheatcodes", nonce, "reset");
        Ok(Default::default())
    }

    /// Cheatcode: setNonce.
    fn cheatcode_set_nonce(
        &self,
        ccx: &mut CheatsCtxt,
        account: Address,
        new_nonce: u64,
    ) -> Result {
        let account = journaled_account(ccx.ecx, account)?;
        // nonce must increment only
        let current = account.info.nonce;
        ensure!(
            new_nonce >= current,
            "new nonce ({new_nonce}) must be strictly equal to or higher than the \
             account's current nonce ({current})"
        );
        account.info.nonce = new_nonce;
        Ok(Default::default())
    }

    /// Cheatcode: setNonceUnsafe.
    fn cheatcode_set_nonce_unsafe(
        &self,
        ccx: &mut CheatsCtxt,
        account: Address,
        new_nonce: u64,
    ) -> Result {
        let account = journaled_account(ccx.ecx, account)?;
        account.info.nonce = new_nonce;
        Ok(Default::default())
    }

    /// Mocks a call to return with a value.
    fn cheatcode_mock_call(
        &self,
        ccx: &mut CheatsCtxt,
        callee: Address,
        data: &Bytes,
        return_data: &Bytes,
    ) -> Result {
        let _ = make_acc_non_empty(&callee, ccx.ecx)?;
        mock_call(ccx.state, &callee, data, None, return_data, InstructionResult::Return);
        Ok(Default::default())
    }

    /// Mocks a call to revert with a value.
    fn cheatcode_mock_call_revert(
        &self,
        ccx: &mut CheatsCtxt,
        callee: Address,
        data: &Bytes,
        revert_data: &Bytes,
    ) -> Result {
        let _ = make_acc_non_empty(&callee, ccx.ecx)?;
        mock_call(ccx.state, &callee, data, None, revert_data, InstructionResult::Revert);
        Ok(Default::default())
    }

    /// Retrieve artifact code.
    fn get_artifact_code(&self, state: &Cheatcodes, path: &str, deployed: bool) -> Result {
        Ok(crate::fs::get_artifact_code(state, path, deployed)?.abi_encode())
    }

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

impl Clone for Box<dyn CheatcodesStrategyRunner> {
    fn clone(&self) -> Self {
        self.new_cloned()
    }
}

/// Implements [CheatcodesStrategyRunner] for EVM.
#[derive(Debug, Default, Clone)]
pub struct EvmCheatcodesStrategyRunner;

impl CheatcodesStrategyRunner for EvmCheatcodesStrategyRunner {
    fn name(&self) -> &'static str {
        "evm"
    }

    fn new_cloned(&self) -> Box<dyn CheatcodesStrategyRunner> {
        Box::new(self.clone())
    }

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
    pub runner: Box<dyn CheatcodesStrategyRunner>,
    /// Strategy context.
    pub context: Box<dyn CheatcodesStrategyContext>,
}

impl CheatcodesStrategy {
    /// Creates a new EVM strategy for the [Cheatcodes].
    pub fn new_evm() -> Self {
        Self { runner: Box::new(EvmCheatcodesStrategyRunner), context: Box::new(()) }
    }
}

impl Clone for CheatcodesStrategy {
    fn clone(&self) -> Self {
        Self { runner: self.runner.new_cloned(), context: self.context.new_cloned() }
    }
}
