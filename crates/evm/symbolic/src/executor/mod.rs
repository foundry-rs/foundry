use super::{abi::*, runtime::*, *};

mod calls;
mod cheatcodes;
mod constraints;
mod create;
mod invariant;
mod opcodes;
mod run;

impl SymbolicExecutor {
    /// Pops the next pending path according to the configured exploration order.
    pub(crate) fn pop_batch<T>(&self, batch: &mut VecDeque<T>) -> Option<T> {
        match self.config.exploration_order {
            SymbolicExplorationOrder::Bfs => batch.pop_front(),
            SymbolicExplorationOrder::Dfs => batch.pop_back(),
        }
    }

    /// Spills the remaining local batch onto the global worklist in scheduler order.
    pub(crate) fn spill_batch<T>(&self, batch: VecDeque<T>, worklist: &mut VecDeque<T>) {
        match self.config.exploration_order {
            SymbolicExplorationOrder::Bfs => worklist.extend(batch),
            SymbolicExplorationOrder::Dfs => {
                worklist.reserve(batch.len());
                for path in batch {
                    worklist.push_back(path);
                }
            }
        }
    }
}
