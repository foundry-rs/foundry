use super::*;

impl SymbolicExecutor {
    pub(super) fn create<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CreateKind,
    ) -> Result<StepOutcome, SymbolicError> {
        if state.is_static {
            state.return_data = SymReturnData::empty(&mut self.cx);
            return Ok(StepOutcome::Revert);
        }

        let offset = state.stack.peek(1)?.clone();
        let size = state.stack.peek(2)?.clone();
        if let Some(outcome) = self.guard_memory_range(executor, state, worklist, &offset, &size)? {
            return Ok(outcome);
        }

        let value = state.stack.pop()?;
        let offset = state.stack.pop()?;
        let size = state.stack.pop()?;
        let size = match state.constrained_usize_checked(&mut self.cx, &size) {
            Some(Ok(size)) => BoundedCopySize::Concrete(size),
            Some(Err(_)) => {
                state.return_data = SymReturnData::empty(&mut self.cx);
                return Ok(StepOutcome::Revert);
            }
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&mut self.cx, &size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &size,
                            max_limit,
                            "symbolic CREATE initcode size",
                        )
                    })?;
                BoundedCopySize::Symbolic { size, max_size }
            }
        };
        let salt =
            if matches!(kind, CreateKind::Create2) { Some(state.stack.pop()?) } else { None };

        size.expand_memory(&mut self.cx, &mut state.memory, offset.clone());

        let initcode = match &size {
            BoundedCopySize::Concrete(size) => {
                if let Some(offset) = state.constrained_usize(&mut self.cx, &offset) {
                    let bytes = state.memory.read_bytes(&mut self.cx, offset, *size);
                    SymCode::from_bytes(&mut self.cx, bytes)
                } else {
                    SymCode::from_memory_offset(&mut self.cx, &state.memory, offset, *size)
                }
            }
            BoundedCopySize::Symbolic { size, max_size } => SymCode::from_memory_symbolic_size(
                &mut self.cx,
                &state.memory,
                offset,
                size.clone(),
                *max_size,
            ),
        };
        let (created_word, created) = match kind {
            CreateKind::Create => {
                let nonce = state.world.nonce(executor, state.address)?;
                let address = state.address.create(nonce);
                (SymExpr::constant(&mut self.cx, address_word(address)), address)
            }
            CreateKind::Create2 => create2_address_word(
                &mut self.cx,
                state,
                state.address,
                salt.expect("CREATE2 salt exists"),
                &initcode,
            )?,
        };

        if !self.prepare_create_value_transfer(executor, state, worklist, value.clone())? {
            return Ok(StepOutcome::Continue);
        }

        let mut failure_world = state.world.clone();
        failure_world.increment_nonce(executor, state.address)?;

        if failure_world.has_code_or_nonce(&mut self.cx, executor, created)? {
            state.world = failure_world;
            state.return_data = SymReturnData::empty(&mut self.cx);
            state.stack.push(SymExpr::zero(&mut self.cx))?;
            return Ok(StepOutcome::Continue);
        }

        let calldata = SymBytes::empty(&mut self.cx);
        let calldata = SymCalldata::from_bytes(&mut self.cx, calldata);
        let mut frame = CallFrame::new(
            &mut self.cx,
            created,
            created,
            state.address,
            value.clone(),
            false,
            calldata,
        );
        frame.address_word = created_word.clone();
        frame.caller_word = state.address_word.clone();
        let mut child = state.child(frame);
        let pending_expected_creates = std::mem::take(&mut child.expected_creates);
        child.world = failure_world.clone();
        child.world.mark_current_transaction_created(created);
        child.world.set_nonce(created, 1);
        child.world.transfer(&mut self.cx, executor, state.address, created, value);
        child.expected_revert = None;
        child.assume_no_revert_next_call = None;

        let outcomes = self.execute_external_call(executor, child, &initcode, completed_paths)?;
        if outcomes.is_empty() {
            return Ok(StepOutcome::AssumeRejected);
        }

        let mut parents = VecDeque::with_capacity(outcomes.len());
        for mut outcome in outcomes {
            let mut parent = state.clone();
            parent.take_call_outcome_state(&mut outcome.state);
            parent.return_data = SymReturnData::empty(&mut self.cx);

            if let Some(assumption) = parent.assume_no_revert_next_call.take()
                && matches!(outcome.status, CallStatus::Revert)
                && self.assume_no_revert_rejects(
                    &mut parent,
                    &assumption,
                    created,
                    &outcome.state.frame.return_data,
                )?
            {
                continue;
            }

            if let Some(mut expected) = parent.expected_revert.clone() {
                match outcome.status {
                    CallStatus::Success => {
                        *state = parent;
                        return Ok(StepOutcome::Failure);
                    }
                    CallStatus::Revert | CallStatus::Failure => {
                        if !self.expected_revert_matches(
                            &mut parent,
                            &expected,
                            created,
                            &outcome.state.frame.return_data,
                        )? {
                            *state = parent;
                            return Ok(StepOutcome::Failure);
                        }
                        if expected.consume_one() {
                            parent.expected_revert = None;
                        } else {
                            parent.expected_revert = Some(expected);
                        }
                        parent.expected_calls = outcome.state.expected_calls;
                        parent.expected_creates = pending_expected_creates.clone();
                        parent.call_mocks = outcome.state.call_mocks;
                        parent.function_mocks = outcome.state.function_mocks;
                        parent.world = failure_world.clone();
                        parent.stack.push(created_word.clone())?;
                        parents.push_back(parent);
                        continue;
                    }
                }
            }

            match outcome.status {
                CallStatus::Success => {
                    parent.world = outcome.state.world;
                    parent.block = outcome.state.block;
                    parent.expected_emit = outcome.state.expected_emit;
                    parent.expected_calls = outcome.state.expected_calls;
                    parent.expected_creates = pending_expected_creates.clone();
                    parent.call_mocks = outcome.state.call_mocks;
                    parent.function_mocks = outcome.state.function_mocks;
                    self.observe_expected_create(
                        &mut parent,
                        state.address,
                        kind,
                        &outcome.state.frame.return_data,
                    )?;
                    if !parent.world.is_destroyed(created) {
                        parent.world.install_code(
                            created,
                            outcome.state.frame.return_data.to_code(&mut self.cx)?,
                        );
                        parent.world.set_nonce(created, 1);
                    }
                    parent.stack.push(created_word.clone())?;
                }
                CallStatus::Revert => {
                    parent.world = failure_world.clone();
                    parent.return_data = outcome.state.frame.return_data;
                    parent.stack.push(SymExpr::zero(&mut self.cx))?;
                }
                CallStatus::Failure => {
                    *state = parent;
                    return Ok(StepOutcome::Failure);
                }
            }

            parents.push_back(parent);
        }

        let Some(first) = self.pop_next_path(&mut parents) else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(parents);
        Ok(StepOutcome::Continue)
    }

    pub(super) fn execute_external_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        initial: PathState,
        code: &SymCode,
        completed_paths: &mut usize,
    ) -> Result<Vec<CallOutcome>, SymbolicError> {
        let mut worklist = VecDeque::from([initial]);
        let mut outcomes = Vec::new();
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        while let Some(mut state) = self.pop_next_feasible_path(&mut worklist)? {
            if *completed_paths >= path_limit {
                return Err(SymbolicError::Unsupported("symbolic path limit exceeded"));
            }
            if std::mem::take(&mut state.pending_storage_hook_revert) {
                *completed_paths += 1;
                outcomes.push(CallOutcome { status: CallStatus::Revert, state });
                continue;
            }

            loop {
                self.check_timeout()?;
                if state.depth >= depth_limit {
                    return Err(SymbolicError::Unsupported("symbolic depth limit exceeded"));
                }
                state.depth += 1;

                let op = match code.guarded_opcode(&mut self.cx, state.pc)? {
                    GuardedOpcode::End => {
                        *completed_paths += 1;
                        outcomes.push(CallOutcome {
                            status: if state.storage_hook_active || state.expectations_satisfied() {
                                CallStatus::Success
                            } else {
                                CallStatus::Failure
                            },
                            state,
                        });
                        break;
                    }
                    GuardedOpcode::Concrete(op) => op,
                    GuardedOpcode::SymbolicSize { condition, opcode } => {
                        let mut in_bounds_constraints = state.constraints.clone();
                        in_bounds_constraints.push(condition.clone());
                        let in_bounds_sat =
                            self.is_sat_with_state(&state, &in_bounds_constraints)?;

                        let mut out_of_bounds_constraints = state.constraints.clone();
                        out_of_bounds_constraints.push(condition.not(&mut self.cx));
                        if self.is_sat_with_state(&state, &out_of_bounds_constraints)? {
                            let mut halted = state.clone();
                            halted.constraints = out_of_bounds_constraints;
                            *completed_paths += 1;
                            outcomes.push(CallOutcome {
                                status: if halted.storage_hook_active
                                    || halted.expectations_satisfied()
                                {
                                    CallStatus::Success
                                } else {
                                    CallStatus::Failure
                                },
                                state: halted,
                            });
                        }

                        if in_bounds_sat {
                            state.constraints = in_bounds_constraints;
                            opcode
                        } else {
                            break;
                        }
                    }
                };

                match self.step(
                    executor,
                    code,
                    code.jump_table(),
                    &mut state,
                    &mut worklist,
                    completed_paths,
                    op,
                )? {
                    StepOutcome::Continue => {}
                    StepOutcome::Halt => {
                        *completed_paths += 1;
                        outcomes.push(CallOutcome {
                            status: if state.storage_hook_active || state.expectations_satisfied() {
                                CallStatus::Success
                            } else {
                                CallStatus::Failure
                            },
                            state,
                        });
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
}
