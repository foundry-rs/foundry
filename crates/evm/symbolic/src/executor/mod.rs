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

#[cfg(test)]
mod tests {
    use super::*;

    struct BranchOnlySolver;

    impl SymbolicSolver for BranchOnlySolver {
        fn stats(&self) -> SymbolicStats {
            SymbolicStats::default()
        }

        fn set_query_observer(&mut self, _observer: Option<QueryObserver>) {}

        fn portfolio_diagnostics(&self) -> Option<&PortfolioDiagnostics> {
            None
        }

        fn capture_diagnostics(&mut self) {}

        fn take_diagnostics(&mut self) -> Option<String> {
            None
        }

        fn check_available(&self) -> Result<(), SymbolicError> {
            Ok(())
        }

        fn is_sat(&mut self, _constraints: &[SymBoolExpr]) -> Result<bool, SymbolicError> {
            panic!("constraints_with_condition should use branch satisfiability")
        }

        fn is_sat_branch(&mut self, _constraints: &[SymBoolExpr]) -> Result<bool, SymbolicError> {
            Ok(true)
        }

        fn model(&mut self, _constraints: &[SymBoolExpr]) -> Result<SymbolicModel, SymbolicError> {
            Err(SymbolicError::Solver("model not implemented".to_string()))
        }
    }

    #[test]
    fn constraints_with_condition_uses_branch_solver_path() {
        let mut executor = SymbolicExecutor {
            config: SymbolicConfig::default(),
            solver: Box::new(BranchOnlySolver),
            deferred_incomplete: None,
        };
        let state = PathState::new(
            Address::ZERO,
            Address::ZERO,
            U256::ZERO,
            SymbolicCalldata::selector_only(&Function::parse("empty()").unwrap()).unwrap(),
            false,
        );
        let condition = SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(1)));

        let (constraints, sat) =
            executor.constraints_with_condition(&state, condition.clone()).unwrap();

        assert!(sat);
        assert_eq!(constraints, vec![condition]);
    }
}
