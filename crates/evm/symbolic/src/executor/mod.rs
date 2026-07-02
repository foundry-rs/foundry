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
}
