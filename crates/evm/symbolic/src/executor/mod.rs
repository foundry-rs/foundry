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

enum JoinedCallOutcome {
    Rejected,
    ExpectedRevert { parent: PathState, child: PathState },
    Success { parent: PathState, child: PathState },
    Revert { parent: PathState, child: PathState },
    Failure(PathState),
}

#[derive(Clone, Copy)]
enum CallPathKind {
    External,
    Sequence,
}

enum CallPathOpcode {
    Execute(u8),
    Halt,
    Discard,
}

pub(super) struct FeasiblePath {
    state: PathState,
    from_deferred: bool,
}

#[derive(Debug)]
struct SequencePath {
    state: PathState,
    steps: Vec<SequenceStepTemplate>,
}

#[derive(Debug)]
struct SequenceCall {
    code: SymCode,
    worklist: VecDeque<PathState>,
    deferred_worklist: VecDeque<PathState>,
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
        deferred_mode: DeferredPathMode,
    ) -> Result<Option<FeasiblePath>, SymbolicError> {
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
                            if matches!(deferred_mode, DeferredPathMode::Skip) {
                                self.defer_hard_arithmetic();
                                continue;
                            }
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
                return Ok(Some(FeasiblePath { state, from_deferred: false }));
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
                Ok(true) => return Ok(Some(FeasiblePath { state, from_deferred: true })),
                Ok(false) => {}
                Err(SymbolicError::SolverUnknown) => self.defer_solver_unknown(),
                Err(err) => return Err(err),
            }
        }
    }

    fn join_call_outcome(
        &mut self,
        state: &PathState,
        mut outcome: CallOutcome,
        reverter: Address,
    ) -> Result<JoinedCallOutcome, SymbolicError> {
        let mut parent = state.clone();
        parent.take_call_outcome_state(&mut outcome.state);

        if let Some(assumption) = parent.assume_no_revert_next_call.take()
            && matches!(outcome.status, CallStatus::Revert)
            && self.assume_no_revert_rejects(
                &mut parent,
                &assumption,
                reverter,
                &outcome.state.frame.return_data,
            )?
        {
            return Ok(JoinedCallOutcome::Rejected);
        }

        if let Some(mut expected) = parent.expected_revert.clone() {
            match outcome.status {
                CallStatus::Success => return Ok(JoinedCallOutcome::Failure(parent)),
                CallStatus::Revert | CallStatus::Failure => {
                    if !self.expected_revert_matches(
                        &mut parent,
                        &expected,
                        reverter,
                        &outcome.state.frame.return_data,
                    )? {
                        return Ok(JoinedCallOutcome::Failure(parent));
                    }
                    if expected.consume_one() {
                        parent.expected_revert = None;
                    } else {
                        parent.expected_revert = Some(expected);
                    }
                    return Ok(JoinedCallOutcome::ExpectedRevert { parent, child: outcome.state });
                }
            }
        }

        Ok(match outcome.status {
            CallStatus::Success => JoinedCallOutcome::Success { parent, child: outcome.state },
            CallStatus::Revert => JoinedCallOutcome::Revert { parent, child: outcome.state },
            CallStatus::Failure => JoinedCallOutcome::Failure(parent),
        })
    }

    fn resume_parent_paths(
        &self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        mut parents: VecDeque<PathState>,
    ) -> StepOutcome {
        let Some(first) = self.pop_next_path(&mut parents) else {
            return StepOutcome::AssumeRejected;
        };
        *state = first;
        worklist.extend(parents);
        StepOutcome::Continue
    }

    fn execute_call_path_batch<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        code: &SymCode,
        worklist: &mut VecDeque<PathState>,
        deferred_worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallPathKind,
    ) -> Result<Vec<CallOutcome>, SymbolicError> {
        let mut outcomes = Vec::new();
        let mut outcomes_before_deferred = None;
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        loop {
            // Let callers inspect a completed outcome before spending the remaining budget on a
            // deferred sibling. A later proof pass reconstructs and drains all nested paths.
            if matches!(kind, CallPathKind::Sequence) && !outcomes.is_empty() {
                break;
            }
            if matches!(kind, CallPathKind::External)
                && matches!(self.nested_deferred_mode, DeferredPathMode::Yield)
                && outcomes_before_deferred.is_some_and(|count| outcomes.len() > count)
            {
                if !worklist.is_empty() || !deferred_worklist.is_empty() {
                    self.defer_hard_arithmetic();
                }
                break;
            }
            let deferred_mode = match kind {
                CallPathKind::Sequence => DeferredPathMode::Drain,
                CallPathKind::External => self.nested_deferred_mode,
            };
            let Some(next) =
                self.pop_next_feasible_path(worklist, deferred_worklist, deferred_mode)?
            else {
                break;
            };
            if next.from_deferred
                && matches!(kind, CallPathKind::External)
                && matches!(deferred_mode, DeferredPathMode::Yield)
            {
                outcomes_before_deferred = Some(outcomes.len());
            }
            let mut state = next.state;
            if *completed_paths >= path_limit {
                return Err(SymbolicError::Unsupported("symbolic path limit exceeded"));
            }
            if std::mem::take(&mut state.pending_storage_hook_revert) {
                *completed_paths += 1;
                outcomes.push(CallOutcome { status: CallStatus::Revert, state });
                continue;
            }
            let _path_span = matches!(kind, CallPathKind::Sequence).then(|| {
                trace_span!("symbolic_path", completed_paths, worklist_size = worklist.len())
                    .entered()
            });
            if matches!(kind, CallPathKind::Sequence) {
                trace!(completed_paths, worklist_size = worklist.len(), "exploring symbolic path");
            }

            loop {
                self.check_timeout()?;
                if state.depth >= depth_limit {
                    return Err(SymbolicError::Unsupported("symbolic depth limit exceeded"));
                }
                state.depth += 1;

                let op = match kind {
                    CallPathKind::Sequence => match code.opcode(&mut self.cx, state.pc)? {
                        Some(op) => CallPathOpcode::Execute(op),
                        None => CallPathOpcode::Halt,
                    },
                    CallPathKind::External => match code.guarded_opcode(&mut self.cx, state.pc)? {
                        GuardedOpcode::End => CallPathOpcode::Halt,
                        GuardedOpcode::Concrete(op) => CallPathOpcode::Execute(op),
                        GuardedOpcode::SymbolicSize { condition, opcode } => {
                            let mut in_bounds_constraints = state.constraints.clone();
                            in_bounds_constraints.push(condition.clone());
                            let in_bounds_sat =
                                self.is_sat_with_state(&state, &in_bounds_constraints)?;

                            let mut out_of_bounds_constraints = state.constraints.clone();
                            out_of_bounds_constraints.push(condition.not(&mut self.cx));
                            if self.is_sat_with_state(&state, &out_of_bounds_constraints)? {
                                if *completed_paths >= path_limit {
                                    return Err(SymbolicError::Unsupported(
                                        "symbolic path limit exceeded",
                                    ));
                                }
                                let mut halted = state.clone();
                                halted.constraints = out_of_bounds_constraints;
                                *completed_paths += 1;
                                let status = self.successful_call_status(kind, &halted);
                                outcomes.push(CallOutcome { status, state: halted });
                            }

                            if in_bounds_sat {
                                state.constraints = in_bounds_constraints;
                                CallPathOpcode::Execute(opcode)
                            } else {
                                CallPathOpcode::Discard
                            }
                        }
                    },
                };
                let op = match op {
                    CallPathOpcode::Execute(op) => op,
                    CallPathOpcode::Halt => {
                        *completed_paths += 1;
                        let status = self.successful_call_status(kind, &state);
                        outcomes.push(CallOutcome { status, state });
                        break;
                    }
                    CallPathOpcode::Discard => break,
                };

                let _step_span = matches!(kind, CallPathKind::Sequence)
                    .then(|| trace_span!("symbolic_step", pc = state.pc, op).entered());
                match self.step(
                    executor,
                    code,
                    code.jump_table(),
                    &mut state,
                    &mut *worklist,
                    completed_paths,
                    op,
                )? {
                    StepOutcome::Continue => {}
                    StepOutcome::Halt => {
                        *completed_paths += 1;
                        let status = self.successful_call_status(kind, &state);
                        outcomes.push(CallOutcome { status, state });
                        break;
                    }
                    StepOutcome::Revert => {
                        *completed_paths += 1;
                        outcomes.push(CallOutcome { status: CallStatus::Revert, state });
                        break;
                    }
                    StepOutcome::Failure => {
                        *completed_paths += 1;
                        outcomes.push(CallOutcome { status: CallStatus::Failure, state });
                        break;
                    }
                    StepOutcome::AssumeRejected | StepOutcome::Forked => break,
                }
            }
        }

        Ok(outcomes)
    }

    fn successful_call_status(&self, kind: CallPathKind, state: &PathState) -> CallStatus {
        if matches!(kind, CallPathKind::External) || state.expectations_satisfied() {
            CallStatus::Success
        } else {
            CallStatus::Failure
        }
    }
}
