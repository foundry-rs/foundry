use super::{abi::*, runtime::*, *};

mod calls;
mod cheatcodes;
mod constraints;
mod create;
mod invariant;
mod opcodes;
mod run;

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
                && !self.branch_is_sat_or_defer(&state.constraints)?
            {
                continue;
            }
            return Ok(Some(state));
        }
        Ok(None)
    }
}
