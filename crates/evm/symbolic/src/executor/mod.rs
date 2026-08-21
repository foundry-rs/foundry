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
    ) -> Result<Option<PathState>, SymbolicError> {
        while let Some(mut state) = self.pop_next_path(paths) {
            if state.take_deferred_feasibility_check()
                && !self.branch_is_sat_or_defer(&state, &state.constraints)?
            {
                continue;
            }
            return Ok(Some(state));
        }
        Ok(None)
    }
}
