use super::{abi::*, runtime::*, *};

mod calls;
mod cheatcodes;
mod constraints;
mod create;
mod invariant;
mod opcodes;
mod run;

#[derive(Debug)]
struct CallOutcome {
    status: CallStatus,
    state: PathState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallStatus {
    Success,
    Revert,
    Failure,
}

#[derive(Debug)]
struct SequencePath {
    state: PathState,
    steps: Vec<SequenceStepTemplate>,
}

#[derive(Clone, Debug)]
struct SequenceStepTemplate {
    sender: Address,
    address: Address,
    contract_name: Option<String>,
    function: Function,
    calldata: SymbolicCalldata,
}

#[derive(Debug)]
struct InvariantCheckOutcome {
    failed: bool,
    state: PathState,
}

impl SymbolicExecutor {
    pub(super) fn pop_next_path(&self, paths: &mut VecDeque<PathState>) -> Option<PathState> {
        match self.config.exploration_order {
            SymbolicExplorationOrder::Bfs => paths.pop_front(),
            SymbolicExplorationOrder::Dfs => paths.pop_back(),
        }
    }

    pub(super) fn pop_next_feasible_path(
        &mut self,
        paths: &mut VecDeque<PathState>,
        deferred_paths: &mut VecDeque<PathState>,
    ) -> Result<Option<PathState>, SymbolicError> {
        loop {
            while let Some(mut state) = self.pop_next_path(paths) {
                if state.take_deferred_feasibility_check() {
                    let replayable_storage = state.world.replay_storage_symbols();
                    match self.solver.branch_feasibility_with_replayable_storage(
                        &mut self.cx,
                        &state.constraints,
                        &replayable_storage,
                    ) {
                        Ok(BranchFeasibility::Sat) => {}
                        Ok(BranchFeasibility::Unsat) => continue,
                        Ok(BranchFeasibility::NeedsSolver) => {
                            trace!("queued hard arithmetic branch for deferred SMT solving");
                            deferred_paths.push_back(state);
                            continue;
                        }
                        Err(SymbolicError::SolverUnknown) => {
                            self.defer_solver_unknown();
                            continue;
                        }
                        Err(err) => return Err(err),
                    }
                }
                return Ok(Some(state));
            }

            let Some(state) = self.pop_next_path(deferred_paths) else {
                return Ok(None);
            };
            if self.deadline.is_none() {
                self.deadline = self
                    .config
                    .timeout
                    .filter(|seconds| *seconds > 0)
                    .map(|seconds| Instant::now() + Duration::from_secs(seconds.into()));
            }
            self.check_timeout()?;
            trace!("escalating deferred hard arithmetic branch to SMT solver");
            match self.is_sat_with_state(&state, &state.constraints) {
                Ok(true) => return Ok(Some(state)),
                Ok(false) => {}
                Err(SymbolicError::SolverUnknown) => self.defer_solver_unknown(),
                Err(err) => return Err(err),
            }
        }
    }
}
