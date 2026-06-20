use super::{abi::*, runtime::*, *};

/// Pops the next pending path according to the configured exploration order.
pub(crate) fn pop_worklist<T>(
    worklist: &mut VecDeque<T>,
    order: SymbolicExplorationOrder,
) -> Option<T> {
    pop_batch(worklist, order)
}

/// Pops the current path from a local batch according to the configured exploration order.
pub(crate) fn pop_batch<T>(batch: &mut VecDeque<T>, order: SymbolicExplorationOrder) -> Option<T> {
    match order {
        SymbolicExplorationOrder::Bfs => batch.pop_front(),
        SymbolicExplorationOrder::Dfs => batch.pop_back(),
    }
}

/// Spills the remaining local batch onto the global worklist in scheduler order.
pub(crate) fn spill_batch<T>(
    batch: VecDeque<T>,
    worklist: &mut VecDeque<T>,
    order: SymbolicExplorationOrder,
) {
    match order {
        SymbolicExplorationOrder::Bfs => worklist.extend(batch),
        SymbolicExplorationOrder::Dfs => {
            worklist.reserve(batch.len());
            for path in batch {
                worklist.push_back(path);
            }
        }
    }
}

mod calls;
mod cheatcodes;
mod constraints;
mod create;
mod invariant;
mod opcodes;
mod run;
