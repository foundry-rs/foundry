//! Synthetic Ethereum transactions over a suspended parent's state.

use super::{ApplyError, Cheatcodes};
use alloy_consensus::{TxLegacy, transaction::Recovered};
use alloy_primitives::TxKind;
use evm2::{
    Evm, EvmFeatures, ExecutionConfig, Precompiles,
    ethereum::{TxEnvelope, ethereum_tx_registry, floor_gas, intrinsic_gas},
    evm::{EmptyDB, registry::HandlerError},
    interpreter::{GasTracker, InstrStop, Interpreter, Message, MessageResult},
};
use foundry_evm_core::ethereum::FoundryEvmTypes;
use std::{mem, sync::Arc};

impl Cheatcodes {
    pub(super) fn should_isolate(&self, message: &Message<FoundryEvmTypes>) -> bool {
        self.config.isolate
            && !self.storage_hooks.active
            && self.isolation_depth.is_none()
            && (message.depth == 1
                || self.broadcast.as_ref().is_some_and(|broadcast| {
                    usize::from(message.depth.saturating_sub(1)) == broadcast.depth
                })
                || self
                    .deploy_code_depth
                    .is_some_and(|depth| depth.checked_add(1) == Some(message.depth)))
    }

    pub(super) fn isolate(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
    ) -> Result<MessageResult<FoundryEvmTypes>, ApplyError> {
        let host = interp.host();
        let spec = host.spec_id();
        let mut version = *host.version();
        let to = if message.kind.is_create() {
            TxKind::Create
        } else {
            TxKind::Call(message.destination)
        };
        let intrinsic =
            intrinsic_gas(&version, message.caller, to, &message.input, 0, 0, message.value);
        let floor = floor_gas(&version, message.caller, to, &message.input, 0, 0, message.value);
        let regular_limit = message.gas_limit.saturating_add(intrinsic);
        let precharged = if version.feature(EvmFeatures::EIP8037) {
            let empty = host
                .state_mut()
                .account_info_untracked(&message.destination)?
                .is_none_or(|info| info.is_empty());
            if empty && message.kind.is_create() {
                version.gas_params.create_state_gas()
            } else if empty && !message.value.is_zero() && version.feature(EvmFeatures::EIP2780) {
                version.gas_params.new_account_state_gas()
            } else {
                0
            }
        } else {
            0
        };
        version.tx_gas_limit_cap = regular_limit;
        version.features.remove(
            EvmFeatures::FEE_CHARGE
                | EvmFeatures::BASE_FEE_CHECK
                | EvmFeatures::NONCE_CHECK
                | EvmFeatures::EIP3607,
        );
        // Keep the forwarded frame budget even when calldata's floor exceeds the synthetic limit.
        // Apply the floor to the returned gas instead of giving the callee extra execution gas.
        if floor > regular_limit {
            version.features.remove(EvmFeatures::EIP7623);
        }
        let nonce =
            host.state_mut().account_info_untracked(&message.caller)?.map_or(0, |info| info.nonce);
        let tx = Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                nonce,
                to,
                input: message.input.clone(),
                value: message.value,
                gas_limit: regular_limit
                    .saturating_add(message.reservoir)
                    .saturating_add(precharged),
                ..Default::default()
            }),
            message.caller,
        );
        let mut child = Evm::<FoundryEvmTypes>::new_with_execution_config_and_ext(
            ExecutionConfig::for_spec_and_version(spec, version),
            spec,
            *host.block(),
            ethereum_tx_registry(spec),
            EmptyDB::default(),
            Precompiles::base(spec),
            *host.ext(),
        );
        let mut state = host.state().clone_with(EmptyDB::default());
        let pending = host.state().prepare_isolated_state();
        state.clear_transaction_state();
        state.set_pending_state(pending);
        state.overlay_db_mut().db =
            mem::replace(&mut host.state_mut().overlay_db_mut().db, Box::new(EmptyDB::default()));
        *child.state_mut() = state;
        let frames = mem::take(&mut self.snapshot_frames);
        let source_revision = self.forks.revision;
        self.isolated_root = Some(message.clone());
        self.isolation_depth = Some(message.depth);
        self.isolated_snapshot_restored = false;
        child.set_inspector(mem::take(self));
        let executed = child.transact(&tx).map(|executed| executed.detach());
        let inspector = child.clear_inspector().expect("isolated inspector");
        // SAFETY: The child was given an owned Self, which cannot be replaced while executing.
        // Self borrows no host state; the checked engine extraction requires a static DB lifetime.
        *self = *unsafe { Box::from_raw(Box::into_raw(inspector).cast::<Self>()) };
        let restored = self.isolated_snapshot_restored;
        let switched = self.forks.revision != source_revision;
        self.snapshot_frames = frames;
        self.isolated_root = None;
        self.isolation_depth = None;
        self.isolated_snapshot_restored = false;
        host.state_mut().overlay_db_mut().db =
            mem::replace(&mut child.state_mut().overlay_db_mut().db, Box::new(EmptyDB::default()));
        host.set_block(*child.block());
        *host.ext_mut() = *child.ext();
        let executed = match executed {
            Ok(executed) => executed,
            Err(HandlerError::Database(error)) => return Err(error.into()),
            Err(HandlerError::Fatal(error)) => {
                return Err(ApplyError::Execution(evm2::ExecutionError::Fatal(error)));
            }
            Err(HandlerError::InsufficientFunds) => {
                return Ok(Self::result(message, InstrStop::OutOfFunds, Default::default()));
            }
            Err(HandlerError::NonceOverflow) => {
                return Ok(Self::result(message, InstrStop::NonceOverflow, Default::default()));
            }
            Err(HandlerError::CreateInitCodeSizeLimit { .. }) => {
                return Ok(Self::result(
                    message,
                    InstrStop::CreateInitCodeSizeLimit,
                    Default::default(),
                ));
            }
            Err(HandlerError::External(error)) => {
                return Err(ApplyError::Execution(evm2::ExecutionError::Fatal(error)));
            }
            Err(error) => {
                // Other validation/registry failures cannot describe the synthetic frame we
                // constructed. Report the adapter/engine failure instead of fabricating REVERT.
                return Err(ApplyError::Execution(evm2::ExecutionError::Fatal(
                    evm2::AnyError::new(error),
                )));
            }
        };
        if restored && executed.result.status {
            let replaced = Arc::new(host.state().snapshot());
            for frame in &mut self.snapshot_frames {
                frame.replaced.get_or_insert_with(|| Arc::clone(&replaced));
            }
        }
        if (restored && executed.result.status) || switched {
            // Snapshot restoration and fork selection replace the parent's active state.
            // Replacement resets transaction scratch, but parent receipt logs precede this
            // synthetic execution and must survive its fork/snapshot changes and failures.
            let logs = mem::take(host.state_mut().logs_mut());
            host.state_mut().clear_transaction_state();
            host.state_mut().set_pending_state(executed.pending_state);
            *host.state_mut().logs_mut() = logs;
        } else {
            host.state_mut().merge_isolated_state(executed.pending_state);
        }
        host.state_mut().overlay_db_mut().cache =
            mem::take(&mut child.state_mut().overlay_db_mut().cache);
        let mut result = executed.result;
        host.state_mut().logs_mut().extend(mem::take(&mut result.logs));
        let state_gas = result.state_gas_spent.saturating_sub(precharged);
        let mut gas =
            GasTracker::new_with_execution_gas_and_reservoir(message.gas_limit, message.reservoir);
        let regular_gas = result.total_gas_spent.max(floor).saturating_sub(result.state_gas_spent);
        if gas.spend_state(state_gas).is_err() || gas.spend(regular_gas).is_err() {
            gas.spend_all();
        }
        gas.set_refunded(result.refunded as i64);
        Ok(MessageResult::<FoundryEvmTypes> {
            stop: result.stop,
            gas,
            output: result.output,
            created_address: result.created_address,
            ..Default::default()
        })
    }
}
