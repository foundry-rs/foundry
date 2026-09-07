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

        let outcomes = self.execute_external_call(executor, child, &initcode, completed_paths)?;
        if outcomes.is_empty() {
            return Ok(StepOutcome::AssumeRejected);
        }

        let mut parents = VecDeque::with_capacity(outcomes.len());
        for outcome in outcomes {
            match self.join_call_outcome(state, outcome, created)? {
                JoinedCallOutcome::Rejected => {}
                JoinedCallOutcome::Failure(mut parent) => {
                    parent.return_data = SymReturnData::empty(&mut self.cx);
                    *state = parent;
                    return Ok(StepOutcome::Failure);
                }
                JoinedCallOutcome::ExpectedRevert { mut parent, child } => {
                    parent.return_data = SymReturnData::empty(&mut self.cx);
                    parent.expected_calls = child.expected_calls;
                    parent.expected_creates = pending_expected_creates.clone();
                    parent.call_mocks = child.call_mocks;
                    parent.function_mocks = child.function_mocks;
                    parent.world = failure_world.clone();
                    parent.stack.push(created_word.clone())?;
                    parents.push_back(parent);
                }
                JoinedCallOutcome::Success { mut parent, child } => {
                    parent.return_data = SymReturnData::empty(&mut self.cx);
                    parent.world = child.world;
                    parent.block = child.block;
                    parent.expected_emit = child.expected_emit;
                    parent.expected_calls = child.expected_calls;
                    parent.expected_creates = pending_expected_creates.clone();
                    parent.call_mocks = child.call_mocks;
                    parent.function_mocks = child.function_mocks;
                    self.observe_expected_create(
                        &mut parent,
                        state.address,
                        kind,
                        &child.frame.return_data,
                    )?;
                    if !parent.world.is_destroyed(created) {
                        parent
                            .world
                            .install_code(created, child.frame.return_data.to_code(&mut self.cx)?);
                        parent.world.set_nonce(created, 1);
                    }
                    parent.stack.push(created_word.clone())?;
                    parents.push_back(parent);
                }
                JoinedCallOutcome::Revert { mut parent, child } => {
                    parent.return_data = SymReturnData::empty(&mut self.cx);
                    parent.world = failure_world.clone();
                    parent.return_data = child.frame.return_data;
                    parent.stack.push(SymExpr::zero(&mut self.cx))?;
                    parents.push_back(parent);
                }
            }
        }

        Ok(self.resume_parent_paths(state, worklist, parents))
    }

    pub(super) fn execute_external_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        initial: PathState,
        code: &SymCode,
        completed_paths: &mut usize,
    ) -> Result<Vec<CallOutcome>, SymbolicError> {
        self.execute_call_paths(executor, initial, code, completed_paths, CallPathKind::External)
    }
}
