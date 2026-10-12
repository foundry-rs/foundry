//! Signed transaction replay over a fork's current owned state.

use super::{ApplyError, Cheatcodes};
use crate::Error;
use alloy_primitives::{B256, U256};
use evm2::{
    Evm, ExecutionConfig, Precompiles,
    ethereum::ethereum_tx_registry,
    evm::{EmptyDB, State, registry::HandlerError},
    interpreter::Interpreter,
};
use foundry_common::block_on;
use foundry_evm_core::ethereum::{EthereumFork, FoundryEvmTypes, SavedState};
use std::{mem, sync::Arc};

impl Cheatcodes {
    pub(super) fn transact(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        id: Option<U256>,
        hash: B256,
        depth: u16,
    ) -> Result<(), ApplyError> {
        let id = id.or(self.forks.active).ok_or_else(|| Error::from("no active fork"))?;
        let source =
            self.forks.forks.get(&id).cloned().ok_or_else(|| {
                Error::from(format!("Requested fork with id `{id}` does not exist"))
            })?;
        let (position, _, tx) = block_on(EthereumFork::transaction_position(
            &foundry_config::Config::default(),
            &source.opts,
            hash,
        ))
        .map_err(|error| Error::from(error.to_string()))?;
        let host = interp.host();
        SavedState::materialize_initial(host.state_mut());
        let active = self.forks.active;
        let affects_active = active == Some(id);
        let spec = host.spec_id();
        let database = source.database.with_spec(spec);
        let mut state = if affects_active {
            host.state().clone_with(database.clone())
        } else {
            match source.state {
                Some(state) => state.as_ref().clone().into_state(database.clone()),
                None => State::new(database.clone()),
            }
        };
        if !affects_active {
            self.forks.merge_persistent(host.state(), &mut state);
        }
        let pending = state.prepare_isolated_state();
        state.clear_transaction_state();
        state.set_pending_state(pending);
        let mut version = *host.version();
        version.chain_id = position.env.version.chain_id;
        let mut context = *host.ext();
        context.chain_id_override = Some(version.chain_id);
        context.basefee_override = Some(position.env.block.basefee);
        context.gas_price_override = None;
        context.origin_override = None;
        context.transaction_origin = Some(tx.signer());
        let mut replay = Evm::<FoundryEvmTypes>::new_with_execution_config_and_ext(
            ExecutionConfig::for_spec_and_version(spec, version),
            spec,
            position.env.block,
            ethereum_tx_registry(spec),
            database,
            Precompiles::base(spec),
            context,
        );
        *replay.state_mut() = state;
        let original_forks = self.forks.clone();
        if !affects_active && let Some(active) = active {
            self.forks.forks.get_mut(&active).expect("active fork").state =
                Some(Arc::new(host.state().snapshot()));
        }
        let frames = mem::take(&mut self.snapshot_frames);
        let isolation_depth = self.isolation_depth;
        let replay_depth = self.replay_depth;
        let isolated_root = self.isolated_root.take();
        let restored = self.isolated_snapshot_restored;
        self.replay_depth = Some(depth);
        self.isolation_depth = Some(depth.saturating_sub(1));
        self.isolated_snapshot_restored = false;
        self.forks.active = Some(id);
        replay.set_inspector(mem::take(self));
        let executed = replay.transact(&tx).map(|executed| executed.detach());
        let inspector = replay.clear_inspector().expect("transaction replay inspector");
        // SAFETY: The engine owns a Self with no borrowed host state, just as in isolated calls.
        *self = *unsafe { Box::from_raw(Box::into_raw(inspector).cast::<Self>()) };
        self.snapshot_frames = frames;
        self.isolation_depth = isolation_depth;
        self.replay_depth = replay_depth;
        self.isolated_root = isolated_root;
        self.isolated_snapshot_restored = restored;
        let selected = self.forks.active.expect("replay fork remains selected");
        self.forks.active = active;
        let executed = match executed {
            Ok(executed) => executed,
            Err(HandlerError::Database(error)) => {
                self.forks = original_forks;
                return Err(error.into());
            }
            Err(HandlerError::Fatal(error)) => {
                self.forks = original_forks;
                return Err(ApplyError::Execution(evm2::ExecutionError::Fatal(error)));
            }
            Err(error) => {
                self.forks = original_forks;
                return Err(Error::from(error.to_string()).into());
            }
        };
        // A mined revert still commits transaction-level state. It is not a cheatcode failure.
        if affects_active {
            let pending = executed.pending_state;
            if selected == id {
                host.state_mut().merge_isolated_state(pending);
            } else {
                // Replay may select another fork, but its receipt is appended after the
                // suspended caller's log prefix rather than replacing that prefix.
                let logs = mem::take(host.state_mut().logs_mut());
                host.state_mut().clear_transaction_state();
                host.state_mut().set_pending_state(pending);
                *host.state_mut().logs_mut() = logs;
                host.state_mut().overlay_db_mut().db = mem::replace(
                    &mut replay.state_mut().overlay_db_mut().db,
                    Box::new(EmptyDB::default()),
                );
                let origin = host.ext().transaction_origin;
                let override_origin = host.ext().origin_override;
                host.set_block(*replay.block());
                *host.ext_mut() = *replay.ext();
                host.ext_mut().transaction_origin = origin;
                host.ext_mut().origin_override = override_origin;
                self.forks.active = Some(selected);
            }
            host.state_mut().overlay_db_mut().cache =
                mem::take(&mut replay.state_mut().overlay_db_mut().cache);
        } else {
            replay.state_mut().set_pending_state(executed.pending_state);
            self.forks.merge_persistent(replay.state(), host.state_mut());
            let fork = self.forks.forks.get_mut(&selected).expect("replayed fork");
            fork.state = Some(Arc::new(replay.state().snapshot()));
            fork.needs_finalization = true;
        }
        host.state_mut().logs_mut().extend(executed.result.logs);
        Ok(())
    }
}
