use super::{abi::*, runtime::*, *};

impl SymbolicExecutor {
    /// Creates a symbolic executor from Foundry's symbolic configuration.
    ///
    /// The configured solver command is not executed here. Solver availability is
    /// checked by [`Self::run`] so construction remains cheap and side-effect free.
    ///
    /// The executor owns an isolated solver backend and symbolic world overlay. Create
    /// a fresh executor when a caller needs independent solver query accounting.
    pub fn new(config: SymbolicConfig) -> Self {
        let solver = SmtLibSubprocessSolver::from_config(&config);
        Self { config, solver: Box::new(solver) }
    }

    /// Returns staged solver portfolio diagnostics collected by this executor.
    pub fn portfolio_diagnostics(&self) -> Option<PortfolioDiagnostics> {
        self.solver.portfolio_diagnostics().cloned()
    }

    /// Defers verbose solver diagnostics until the caller explicitly takes them.
    pub fn capture_diagnostics(&mut self) {
        self.solver.capture_diagnostics();
    }

    /// Returns and clears deferred verbose solver diagnostics.
    pub fn take_diagnostics(&mut self) -> Option<String> {
        self.solver.take_diagnostics()
    }

    /// Executes one function symbolically against an already-deployed test contract.
    ///
    /// The input executor supplies the deployed bytecode, storage backend, caller, and
    /// target address established by the normal forge test setup flow. This method
    /// does not mutate the concrete executor and does not replay failures itself; when
    /// it returns [`SymbolicRunResult::Counterexample`], callers should replay the
    /// returned arguments through the concrete executor before reporting the failure.
    ///
    /// Unsupported opcodes, unsupported ABI types, missing solver support, and resource
    /// limit exhaustion are reported as [`SymbolicRunResult::Incomplete`].
    ///
    /// Ordinary Solidity `require` reverts prune the current path. Assertion failures,
    /// forge-std assertion reverts, and DSTest failure signals are reported as
    /// counterexample candidates when the failing path is satisfiable.
    pub fn run<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicRunInput<'_, FEN>,
    ) -> SymbolicRunResult {
        if let Err(err) = self.solver.check_available() {
            return SymbolicRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: SymbolicStats::default(),
            };
        }

        match self.run_inner(input) {
            Ok(result) => result,
            Err(err) => SymbolicRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: self.solver.stats(),
            },
        }
    }

    /// Executes a bounded symbolic invariant call sequence.
    ///
    /// Each sequence step chooses from the concrete target functions and senders supplied by
    /// Foundry's invariant target discovery. Arguments are generated through the same symbolic ABI
    /// model used by stateless symbolic tests, and the symbolic world state is preserved between
    /// steps. Returned counterexamples must still be replayed by the caller before reporting.
    ///
    /// The configured invariant depth limits the number of target calls explored before the
    /// invariant is checked. A depth of zero checks only the invariant against setup state.
    pub fn run_invariant<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicInvariantRunInput<'_, FEN>,
    ) -> SymbolicInvariantRunResult {
        if let Err(err) = self.solver.check_available() {
            return SymbolicInvariantRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: SymbolicStats::default(),
            };
        }

        match self.run_invariant_inner(input) {
            Ok(result) => result,
            Err(err) => SymbolicInvariantRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: self.solver.stats(),
            },
        }
    }

    /// Runs the `run_inner` symbolic executor helper.
    fn run_inner<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicRunInput<'_, FEN>,
    ) -> Result<SymbolicRunResult, SymbolicError> {
        let account = input
            .executor
            .backend()
            .basic_ref(input.target)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?
            .ok_or(SymbolicError::MissingAccount(input.target))?;
        let code =
            account.code.ok_or(SymbolicError::MissingCode(input.target))?.original_bytes().to_vec();
        let code = SymCode::concrete(code);
        let jumpdests = analyze_jumpdests(&code);
        let mut worklist = VecDeque::new();
        for calldata in SymbolicCalldata::variants(input.function, &self.config)? {
            let mut root = PathState::new(
                input.target,
                input.sender,
                input.value,
                calldata,
                input.ffi_enabled,
            );
            root.apply_executor_env(input.executor);
            root.world.set_storage_layout(self.config.storage_layout);
            worklist.push_back(root);
        }
        let mut completed_paths = 0usize;
        let mut reverted_paths = 0usize;
        let mut normal_paths = 0usize;
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        while let Some(mut state) = worklist.pop_front() {
            if completed_paths >= path_limit {
                return Ok(SymbolicRunResult::Incomplete {
                    kind: SymbolicStopReason::Stuck,
                    reason: format!("symbolic path limit exceeded ({path_limit})"),
                    stats: self.stats_with_paths(completed_paths),
                });
            }

            loop {
                if state.depth >= depth_limit {
                    return Ok(SymbolicRunResult::Incomplete {
                        kind: SymbolicStopReason::Stuck,
                        reason: format!("symbolic depth limit exceeded ({depth_limit})"),
                        stats: self.stats_with_paths(completed_paths),
                    });
                }
                state.depth += 1;

                let Some(op) = code.opcode(state.pc)? else {
                    if !state.expectations_satisfied() {
                        let (args, calldata_bytes) = self.materialize_stateless_counterexample(
                            state.root_calldata.as_ref().ok_or_else(|| {
                                SymbolicError::Unsupported("missing root symbolic calldata")
                            })?,
                            input.function,
                            &state,
                        )?;
                        return Ok(SymbolicRunResult::Counterexample {
                            args,
                            calldata: calldata_bytes,
                            stats: self.stats_with_paths(completed_paths + 1),
                        });
                    }
                    completed_paths += 1;
                    break;
                };

                match self.step(
                    input.executor,
                    &code,
                    &jumpdests,
                    &mut state,
                    &mut worklist,
                    &mut completed_paths,
                    op,
                )? {
                    StepOutcome::Continue => {}
                    StepOutcome::Halt => {
                        if !state.expectations_satisfied() {
                            let (args, calldata_bytes) = self
                                .materialize_stateless_counterexample(
                                    state.root_calldata.as_ref().ok_or_else(|| {
                                        SymbolicError::Unsupported("missing root symbolic calldata")
                                    })?,
                                    input.function,
                                    &state,
                                )?;
                            return Ok(SymbolicRunResult::Counterexample {
                                args,
                                calldata: calldata_bytes,
                                stats: self.stats_with_paths(completed_paths + 1),
                            });
                        }
                        completed_paths += 1;
                        normal_paths += 1;
                        break;
                    }
                    StepOutcome::Revert => {
                        completed_paths += 1;
                        reverted_paths += 1;
                        break;
                    }
                    StepOutcome::AssumeRejected => break,
                    StepOutcome::Forked => break,
                    StepOutcome::Failure => {
                        let (args, calldata_bytes) = self.materialize_stateless_counterexample(
                            state.root_calldata.as_ref().ok_or_else(|| {
                                SymbolicError::Unsupported("missing root symbolic calldata")
                            })?,
                            input.function,
                            &state,
                        )?;
                        return Ok(SymbolicRunResult::Counterexample {
                            args,
                            calldata: calldata_bytes,
                            stats: self.stats_with_paths(completed_paths + 1),
                        });
                    }
                }
            }
        }

        if normal_paths == 0 && reverted_paths > 0 {
            return Ok(SymbolicRunResult::Incomplete {
                kind: SymbolicStopReason::RevertAll,
                reason: "all symbolic paths reverted".to_string(),
                stats: self.stats_with_paths(completed_paths),
            });
        }

        Ok(SymbolicRunResult::Safe(self.stats_with_paths(completed_paths)))
    }

    /// Runs the `materialize_stateless_counterexample` symbolic executor helper.
    fn materialize_stateless_counterexample(
        &mut self,
        calldata: &SymbolicCalldata,
        function: &Function,
        state: &PathState,
    ) -> Result<(Vec<DynSolValue>, Bytes), SymbolicError> {
        let model = self.solver.model(&state.constraints)?;
        let args = calldata.model_to_args(&model)?;
        let calldata_bytes = Bytes::from(function.abi_encode_input(&args)?);
        Ok((args, calldata_bytes))
    }

    /// Runs the `run_invariant_inner` symbolic executor helper.
    fn run_invariant_inner<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicInvariantRunInput<'_, FEN>,
    ) -> Result<SymbolicInvariantRunResult, SymbolicError> {
        if input.targets.is_empty() {
            return Err(SymbolicError::Unsupported("symbolic invariant has no targets"));
        }

        let senders =
            if input.senders.is_empty() { vec![input.sender] } else { input.senders.clone() };
        let mut completed_paths = 0usize;
        let mut initial_state =
            PathState::empty(input.invariant_address, input.sender, input.ffi_enabled);
        initial_state.apply_executor_env(input.executor);
        initial_state.world.set_storage_layout(self.config.storage_layout);
        let initial = SequencePath { state: initial_state, steps: Vec::new() };

        for outcome in self.execute_invariant_check(
            input.executor,
            initial.state.clone(),
            input.invariant_address,
            input.sender,
            input.invariant,
            input.after_invariant,
            &mut completed_paths,
        )? {
            if outcome.failed {
                let sequence = self.materialize_sequence(&initial.steps, &outcome.state)?;
                return Ok(SymbolicInvariantRunResult::Counterexample {
                    sequence,
                    stats: self.stats_with_paths(completed_paths),
                });
            }
        }

        let path_limit = self.config.path_width() as usize;
        let mut frontier = vec![initial];
        for depth in 0..input.depth {
            let mut next_frontier = Vec::new();
            for sequence in frontier {
                for (target_idx, target) in input.targets.iter().enumerate() {
                    for (sender_idx, sender) in senders.iter().copied().enumerate() {
                        let prefix = format!("sequence_{depth}_{target_idx}_{sender_idx}");
                        let calldatas = SymbolicCalldata::variants_with_prefix(
                            &target.function,
                            &self.config,
                            prefix,
                        )?;
                        for calldata in calldatas {
                            let step = SequenceStepTemplate {
                                sender,
                                address: target.address,
                                contract_name: target.contract_name.clone(),
                                function: target.function.clone(),
                                calldata,
                            };
                            let outcomes = self.execute_sequence_call(
                                input.executor,
                                sequence.state.clone(),
                                target.address,
                                sender,
                                &target.function,
                                step.calldata.call_data(),
                                step.calldata.constraints.clone(),
                                &mut completed_paths,
                            )?;

                            for outcome in outcomes {
                                let mut steps = sequence.steps.clone();
                                steps.push(step.clone());

                                match outcome.status {
                                    TopLevelCallStatus::Failure => {
                                        let sequence =
                                            self.materialize_sequence(&steps, &outcome.state)?;
                                        return Ok(SymbolicInvariantRunResult::Counterexample {
                                            sequence,
                                            stats: self.stats_with_paths(completed_paths),
                                        });
                                    }
                                    TopLevelCallStatus::Revert => {
                                        if input.fail_on_revert {
                                            let sequence =
                                                self.materialize_sequence(&steps, &outcome.state)?;
                                            return Ok(
                                                SymbolicInvariantRunResult::Counterexample {
                                                    sequence,
                                                    stats: self.stats_with_paths(completed_paths),
                                                },
                                            );
                                        }
                                    }
                                    TopLevelCallStatus::Success => {
                                        for invariant_outcome in self.execute_invariant_check(
                                            input.executor,
                                            outcome.state.clone(),
                                            input.invariant_address,
                                            input.sender,
                                            input.invariant,
                                            input.after_invariant,
                                            &mut completed_paths,
                                        )? {
                                            if invariant_outcome.failed {
                                                let sequence = self.materialize_sequence(
                                                    &steps,
                                                    &invariant_outcome.state,
                                                )?;
                                                return Ok(
                                                    SymbolicInvariantRunResult::Counterexample {
                                                        sequence,
                                                        stats: self
                                                            .stats_with_paths(completed_paths),
                                                    },
                                                );
                                            }
                                            next_frontier.push(SequencePath {
                                                state: invariant_outcome.state,
                                                steps: steps.clone(),
                                            });
                                        }
                                    }
                                }

                                if completed_paths >= path_limit {
                                    return Ok(SymbolicInvariantRunResult::Incomplete {
                                        kind: SymbolicStopReason::Stuck,
                                        reason: format!(
                                            "symbolic path limit exceeded ({path_limit})"
                                        ),
                                        stats: self.stats_with_paths(completed_paths),
                                    });
                                }
                            }
                        }
                    }
                }
            }

            if next_frontier.is_empty() {
                break;
            }
            frontier = next_frontier;
        }

        Ok(SymbolicInvariantRunResult::Safe(self.stats_with_paths(completed_paths)))
    }

    /// Implements the `stats_with_paths` symbolic executor helper.
    fn stats_with_paths(&self, paths: usize) -> SymbolicStats {
        let mut stats = self.solver.stats();
        stats.paths = paths;
        stats
    }

    #[expect(clippy::too_many_arguments)]
    /// Computes the `execute_invariant_check` symbolic executor helper result.
    fn execute_invariant_check<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: PathState,
        invariant_address: Address,
        sender: Address,
        invariant: &Function,
        after_invariant: Option<&Function>,
        completed_paths: &mut usize,
    ) -> Result<Vec<InvariantCheckOutcome>, SymbolicError> {
        let calldata = SymbolicCalldata::selector_only(invariant)?;
        let outcomes = self.execute_sequence_call(
            executor,
            state,
            invariant_address,
            sender,
            invariant,
            calldata.call_data(),
            calldata.constraints,
            completed_paths,
        )?;

        let mut checked = Vec::new();
        for mut outcome in outcomes {
            if !matches!(outcome.status, TopLevelCallStatus::Success) {
                outcome.status = TopLevelCallStatus::Failure;
                checked.push(InvariantCheckOutcome { failed: true, state: outcome.state });
                continue;
            }

            if self.invariant_return_failed(invariant, &outcome.return_data, &mut outcome.state)? {
                checked.push(InvariantCheckOutcome { failed: true, state: outcome.state });
                continue;
            }

            let Some(after_invariant) = after_invariant else {
                checked.push(InvariantCheckOutcome { failed: false, state: outcome.state });
                continue;
            };

            let after_calldata = SymbolicCalldata::selector_only(after_invariant)?;
            for after_outcome in self.execute_sequence_call(
                executor,
                outcome.state.clone(),
                invariant_address,
                sender,
                after_invariant,
                after_calldata.call_data(),
                after_calldata.constraints.clone(),
                completed_paths,
            )? {
                checked.push(InvariantCheckOutcome {
                    failed: !matches!(after_outcome.status, TopLevelCallStatus::Success),
                    state: after_outcome.state,
                });
            }
        }
        Ok(checked)
    }

    /// Implements the `invariant_return_failed` symbolic executor helper.
    fn invariant_return_failed(
        &mut self,
        invariant: &Function,
        return_data: &SymReturnData,
        state: &mut PathState,
    ) -> Result<bool, SymbolicError> {
        if invariant.outputs.is_empty() {
            return Ok(false);
        }
        if invariant.outputs.len() != 1 || invariant.outputs[0].selector_type().as_ref() != "bool" {
            return Ok(false);
        }
        if return_data.len < 32 {
            return Ok(true);
        }

        let pass = return_data.load_word(0)?.nonzero_bool();
        let fail = pass.clone().not();
        match fail {
            BoolExpr::Const(true) => Ok(true),
            BoolExpr::Const(false) => Ok(false),
            fail => {
                let mut constraints = state.constraints.clone();
                constraints.push(fail);
                if self.solver.is_sat(&constraints)? {
                    state.constraints = constraints;
                    Ok(true)
                } else {
                    state.constraints.push(pass);
                    Ok(false)
                }
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    /// Computes the `execute_sequence_call` symbolic executor helper result.
    fn execute_sequence_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        mut state: PathState,
        target: Address,
        sender: Address,
        _function: &Function,
        calldata: SymCalldata,
        constraints: Vec<BoolExpr>,
        completed_paths: &mut usize,
    ) -> Result<Vec<TopLevelCallOutcome>, SymbolicError> {
        let code = self.account_code(executor, target)?;
        let code = SymCode::concrete(code);
        let jumpdests = analyze_jumpdests(&code);
        state.call_depth = 0;
        state.origin = sender;
        state.origin_word = SymWord::Concrete(address_word(sender));
        state.frame =
            CallFrame::new(target, target, target, sender, SymWord::zero(), false, calldata);
        state.constraints.extend(constraints);

        let mut worklist = VecDeque::from([state]);
        let mut outcomes = Vec::new();
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        while let Some(mut state) = worklist.pop_front() {
            if *completed_paths >= path_limit {
                return Err(SymbolicError::Unsupported("symbolic path limit exceeded"));
            }

            loop {
                if state.depth >= depth_limit {
                    return Err(SymbolicError::Unsupported("symbolic depth limit exceeded"));
                }
                state.depth += 1;

                let Some(op) = code.opcode(state.pc)? else {
                    *completed_paths += 1;
                    outcomes.push(TopLevelCallOutcome {
                        status: if state.expectations_satisfied() {
                            TopLevelCallStatus::Success
                        } else {
                            TopLevelCallStatus::Failure
                        },
                        return_data: state.return_data.clone(),
                        state,
                    });
                    break;
                };

                match self.step(
                    executor,
                    &code,
                    &jumpdests,
                    &mut state,
                    &mut worklist,
                    completed_paths,
                    op,
                )? {
                    StepOutcome::Continue => {}
                    StepOutcome::Halt => {
                        *completed_paths += 1;
                        outcomes.push(TopLevelCallOutcome {
                            status: if state.expectations_satisfied() {
                                TopLevelCallStatus::Success
                            } else {
                                TopLevelCallStatus::Failure
                            },
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::Revert => {
                        *completed_paths += 1;
                        outcomes.push(TopLevelCallOutcome {
                            status: TopLevelCallStatus::Revert,
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::Failure => {
                        *completed_paths += 1;
                        outcomes.push(TopLevelCallOutcome {
                            status: TopLevelCallStatus::Failure,
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::AssumeRejected | StepOutcome::Forked => break,
                }
            }
        }

        Ok(outcomes)
    }

    /// Implements the `account_code` symbolic executor helper.
    fn account_code<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<Vec<u8>, SymbolicError> {
        executor
            .backend()
            .basic_ref(address)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?
            .ok_or(SymbolicError::MissingAccount(address))?
            .code
            .ok_or(SymbolicError::MissingCode(address))
            .map(|code| code.original_bytes().to_vec())
    }

    /// Runs the `materialize_sequence` symbolic executor helper.
    fn materialize_sequence(
        &mut self,
        steps: &[SequenceStepTemplate],
        state: &PathState,
    ) -> Result<Vec<SymbolicInvariantStep>, SymbolicError> {
        let model = self.solver.model(&state.constraints)?;
        steps
            .iter()
            .map(|step| {
                let args = step.calldata.model_to_args(&model)?;
                let calldata = Bytes::from(step.function.abi_encode_input(&args)?);
                Ok(SymbolicInvariantStep {
                    sender: step.sender,
                    address: step.address,
                    contract_name: step.contract_name.clone(),
                    function_name: step.function.name.clone(),
                    signature: step.function.signature(),
                    args,
                    calldata,
                })
            })
            .collect()
    }

    #[expect(clippy::too_many_arguments)]
    /// Runs the `step` symbolic executor helper.
    fn step<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        code: &SymCode,
        jumpdests: &BTreeSet<usize>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        op: u8,
    ) -> Result<StepOutcome, SymbolicError> {
        state.pc += 1;

        if op == opcode::PUSH0 {
            state.stack.push(SymWord::zero())?;
            return Ok(StepOutcome::Continue);
        }
        if (opcode::PUSH1..=opcode::PUSH32).contains(&op) {
            let n = (op - opcode::PUSH1 + 1) as usize;
            let end = state.pc.saturating_add(n);
            if end > code.len() {
                return Err(SymbolicError::InvalidBytecode("truncated PUSH data"));
            }
            let bytes = std::iter::repeat_with(SymWord::zero)
                .take(32 - n)
                .chain(code.read_bytes(state.pc, n))
                .collect::<Vec<_>>();
            state.pc = end;
            state.stack.push(word_from_bytes(bytes))?;
            return Ok(StepOutcome::Continue);
        }
        if (opcode::DUP1..=opcode::DUP16).contains(&op) {
            let n = (op - opcode::DUP1 + 1) as usize;
            let value = state.stack.peek(n - 1)?.clone();
            state.stack.push(value)?;
            return Ok(StepOutcome::Continue);
        }
        if (opcode::SWAP1..=opcode::SWAP16).contains(&op) {
            let n = (op - opcode::SWAP1 + 1) as usize;
            state.stack.swap(n)?;
            return Ok(StepOutcome::Continue);
        }

        match op {
            opcode::STOP => Ok(StepOutcome::Halt),
            opcode::ADD => state.bin_word(|a, b| a.wrapping_add(b), ExprOp::Add),
            opcode::SUB => state.bin_word(|a, b| a.wrapping_sub(b), ExprOp::Sub),
            opcode::MUL => state.bin_word(|a, b| a.wrapping_mul(b), ExprOp::Mul),
            opcode::EXP => state.exp_word(),
            opcode::DIV => state.bin_word_div_zero_guard(
                |a, b| if b.is_zero() { U256::ZERO } else { a / b },
                ExprOp::UDiv,
            ),
            opcode::SDIV => state.bin_word_div_zero_guard(sdiv, ExprOp::SDiv),
            opcode::MOD => state.bin_word_div_zero_guard(
                |a, b| if b.is_zero() { U256::ZERO } else { a % b },
                ExprOp::URem,
            ),
            opcode::SMOD => state.bin_word_div_zero_guard(smod, ExprOp::SRem),
            opcode::ADDMOD => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let n = state.stack.pop()?;
                match (a, b, n) {
                    (SymWord::Concrete(a), SymWord::Concrete(b), SymWord::Concrete(n)) => {
                        state.stack.push(SymWord::Concrete(if n.is_zero() {
                            U256::ZERO
                        } else {
                            a.wrapping_add(b) % n
                        }))?;
                    }
                    (a, b, n) => {
                        let n = n.into_expr();
                        state.stack.push(SymWord::Expr(Expr::Ite(
                            Box::new(BoolExpr::eq(n.clone(), Expr::Const(U256::ZERO))),
                            Box::new(Expr::Const(U256::ZERO)),
                            Box::new(Expr::op(
                                ExprOp::URem,
                                Expr::op(ExprOp::Add, a.into_expr(), b.into_expr()),
                                n,
                            )),
                        )))?;
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::MULMOD => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let n = state.stack.pop()?;
                match (a, b, n) {
                    (SymWord::Concrete(a), SymWord::Concrete(b), SymWord::Concrete(n)) => {
                        state.stack.push(SymWord::Concrete(if n.is_zero() {
                            U256::ZERO
                        } else {
                            a.wrapping_mul(b) % n
                        }))?;
                    }
                    (a, b, n) => {
                        let n = n.into_expr();
                        state.stack.push(SymWord::Expr(Expr::Ite(
                            Box::new(BoolExpr::eq(n.clone(), Expr::Const(U256::ZERO))),
                            Box::new(Expr::Const(U256::ZERO)),
                            Box::new(Expr::op(
                                ExprOp::URem,
                                Expr::op(ExprOp::Mul, a.into_expr(), b.into_expr()),
                                n,
                            )),
                        )))?;
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::LT => state.cmp_word(|a, b| a < b, BoolExprOp::Ult),
            opcode::GT => state.cmp_word(|a, b| a > b, BoolExprOp::Ugt),
            opcode::SLT => state.cmp_word(slt, BoolExprOp::Slt),
            opcode::SGT => state.cmp_word(|a, b| slt(b, a), BoolExprOp::Sgt),
            opcode::EQ => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                state.stack.push(SymWord::from_bool(BoolExpr::eq(b.into_expr(), a.into_expr())))?;
                Ok(StepOutcome::Continue)
            }
            opcode::ISZERO => {
                let value = state.stack.pop()?;
                state.stack.push(SymWord::from_bool(value.into_zero_bool()))?;
                Ok(StepOutcome::Continue)
            }
            opcode::AND => state.bin_word(|a, b| a & b, ExprOp::And),
            opcode::OR => state.bin_word(|a, b| a | b, ExprOp::Or),
            opcode::XOR => state.bin_word(|a, b| a ^ b, ExprOp::Xor),
            opcode::NOT => {
                let value = state.stack.pop()?;
                state.stack.push(match value {
                    SymWord::Concrete(value) => SymWord::Concrete(!value),
                    value => SymWord::Expr(Expr::Not(Box::new(value.into_expr()))),
                })?;
                Ok(StepOutcome::Continue)
            }
            opcode::SIGNEXTEND => {
                let byte_index = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.stack.push(signextend_word_dynamic(byte_index, value))?;
                Ok(StepOutcome::Continue)
            }
            opcode::BYTE => {
                let index = state.stack.pop()?;
                let word = state.stack.pop()?;
                state.stack.push(byte_word_dynamic(index, word))?;
                Ok(StepOutcome::Continue)
            }
            opcode::SHL => state.shift_word(ShiftKind::Shl),
            opcode::SHR => state.shift_word(ShiftKind::Shr),
            opcode::SAR => state.shift_word(ShiftKind::Sar),
            opcode::KECCAK256 => {
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        let bytes = state.memory.read_bytes_offset(offset, size);
                        state.stack.push(keccak_word(bytes))?;
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic SHA3 size",
                                )
                            })?;
                        let bytes =
                            state.memory.read_bytes_symbolic_size(offset, size.clone(), max_size);
                        state.stack.push(keccak_word_with_len(bytes, size))?;
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::ADDRESS => {
                let address = state.address_word.clone();
                state.stack.push(address)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLER => {
                let caller = state.caller_word.clone();
                state.stack.push(caller)?;
                Ok(StepOutcome::Continue)
            }
            opcode::ORIGIN => {
                let origin = state.origin_word.clone();
                state.stack.push(origin)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLVALUE => {
                let callvalue = state.callvalue.clone();
                state.stack.push(callvalue)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BLOCKHASH => {
                let number = state.stack.pop()?;
                let hash = state.block.block_hash_word(executor, number)?;
                state.stack.push(hash)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BALANCE => {
                let target = state.stack.pop()?;
                let balance = state.balance_word(executor, target)?;
                state.stack.push(balance)?;
                Ok(StepOutcome::Continue)
            }
            opcode::SELFBALANCE => {
                let balance = state.balance(executor, state.address);
                state.stack.push(balance)?;
                Ok(StepOutcome::Continue)
            }
            opcode::EXTCODESIZE => {
                let target = state.stack.pop()?;
                let size = state.extcode_size_word(executor, target)?;
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::EXTCODEHASH => {
                let target = state.stack.pop()?;
                let hash = state.extcode_hash_word(executor, target)?;
                state.stack.push(hash)?;
                Ok(StepOutcome::Continue)
            }
            opcode::EXTCODECOPY => {
                let target = state.stack.pop()?;
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        let bytes = state.extcode_bytes_word(executor, target, offset, size)?;
                        state.memory.copy_symbolic_offset(dest, bytes);
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic EXTCODECOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            let bytes =
                                state.extcode_bytes_word(executor, target, offset, max_size)?;
                            state.memory.copy_symbolic_size_offset(dest, size, bytes)?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::CALLDATALOAD => {
                let offset = state.stack.pop()?;
                let value = state.calldata.load_word(offset)?;
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLDATASIZE => {
                let size = state.calldata.size_word.clone();
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLDATACOPY => {
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        if size != 0 {
                            let calldata = state.calldata.clone();
                            state.memory.copy_calldata_to_offset(dest, offset, size, &calldata)?;
                        }
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic CALLDATACOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            let calldata = state.calldata.clone();
                            state.memory.copy_calldata_symbolic_size(
                                dest, offset, size, max_size, &calldata,
                            )?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::CODESIZE => {
                state.stack.push(SymWord::Concrete(U256::from(code.len())))?;
                Ok(StepOutcome::Continue)
            }
            opcode::CODECOPY => {
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        state
                            .memory
                            .copy_symbolic_offset(dest, code.read_bytes_offset(offset, size));
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic CODECOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            state.memory.copy_symbolic_size_offset(
                                dest,
                                size,
                                code.read_bytes_offset(offset, max_size),
                            )?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::RETURNDATASIZE => {
                let size = state.return_data.len_word();
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::RETURNDATACOPY => {
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        if !self.assume_returndata_copy_in_bounds(
                            state,
                            offset.clone(),
                            SymWord::Concrete(U256::from(size)),
                        )? {
                            return Ok(StepOutcome::Revert);
                        }
                        let return_data = state.return_data.clone();
                        state.memory.copy_return_data_to_offset(
                            dest,
                            offset,
                            size,
                            &return_data,
                        )?;
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let available = state
                            .constrained_usize(&offset)
                            .map(|offset| state.return_data.len.saturating_sub(offset))
                            .unwrap_or(state.return_data.len);
                        let max_limit = available.min(self.config.max_calldata_bytes as usize);
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic RETURNDATACOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            let return_data = state.return_data.clone();
                            if !self.assume_returndata_copy_in_bounds(
                                state,
                                offset.clone(),
                                size.clone(),
                            )? {
                                return Ok(StepOutcome::Revert);
                            }
                            state.memory.copy_return_data_symbolic_size(
                                dest,
                                offset,
                                size,
                                max_size,
                                &return_data,
                            )?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::POP => {
                state.stack.pop()?;
                Ok(StepOutcome::Continue)
            }
            opcode::MLOAD => {
                let offset = state.stack.pop()?;
                let value = state.memory.load_word_offset(offset)?;
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::MSTORE => {
                let offset = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.memory.store_word_offset(offset, value);
                Ok(StepOutcome::Continue)
            }
            opcode::MSTORE8 => {
                let offset = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.memory.store_byte_offset(offset, value);
                Ok(StepOutcome::Continue)
            }
            opcode::SLOAD => {
                let key = state.stack.pop()?;
                state.record_sload(state.storage_address, key.clone());
                let value = state.world.sload(executor, state.storage_address, key)?;
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::SSTORE => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let key = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.record_sstore(state.storage_address, key.clone());
                state.world.sstore(state.storage_address, key, value);
                Ok(StepOutcome::Continue)
            }
            opcode::TLOAD => {
                let key = state.stack.pop()?;
                let value = state.world.tload(state.storage_address, key);
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::TSTORE => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let key = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.world.tstore(state.storage_address, key, value);
                Ok(StepOutcome::Continue)
            }
            opcode::JUMP => {
                let dest = state.stack.pop()?;
                let dest = state.expect_constrained_usize(dest, "symbolic JUMP destination")?;
                ensure_jumpdest(dest, jumpdests)?;
                if !self.take_loop_jump(state, state.pc, dest) {
                    return Ok(StepOutcome::AssumeRejected);
                }
                state.pc = dest;
                Ok(StepOutcome::Continue)
            }
            opcode::JUMPI => {
                let dest = state.stack.pop()?;
                let dest = state.expect_constrained_usize(dest, "symbolic JUMPI destination")?;
                ensure_jumpdest(dest, jumpdests)?;
                let cond = state.stack.pop()?;
                match cond.truth() {
                    Some(true) => {
                        if !self.take_loop_jump(state, state.pc, dest) {
                            return Ok(StepOutcome::AssumeRejected);
                        }
                        state.pc = dest;
                        Ok(StepOutcome::Continue)
                    }
                    Some(false) => Ok(StepOutcome::Continue),
                    None => {
                        let true_cond = cond.nonzero_bool();
                        let false_cond = true_cond.clone().not();
                        let fallthrough = state.pc;
                        let mut true_state = state.clone();
                        true_state.constraints.push(true_cond);
                        true_state.pc = dest;
                        let mut false_state = state.clone();
                        false_state.constraints.push(false_cond);
                        false_state.pc = fallthrough;

                        if self.take_loop_jump(&mut true_state, fallthrough, dest)
                            && self.solver.is_sat(&true_state.constraints)?
                        {
                            worklist.push_back(true_state);
                        }
                        if self.solver.is_sat(&false_state.constraints)? {
                            worklist.push_back(false_state);
                        }
                        Ok(StepOutcome::Forked)
                    }
                }
            }
            opcode::PC => {
                let pc = state.pc - 1;
                state.stack.push(SymWord::Concrete(U256::from(pc)))?;
                Ok(StepOutcome::Continue)
            }
            opcode::MSIZE => {
                let size = state.memory.size_word();
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::GAS => {
                state.stack.push(SymWord::Concrete(U256::MAX))?;
                Ok(StepOutcome::Continue)
            }
            opcode::JUMPDEST => Ok(StepOutcome::Continue),
            opcode::MCOPY => {
                let dest = state.stack.pop()?;
                let src = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        state.memory.copy_memory_to_offset(dest, src, size)?;
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic MCOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            state.memory.copy_memory_symbolic_size(dest, src, size, max_size)?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::RETURN => self.return_or_revert(state, false),
            opcode::REVERT => self.return_or_revert(state, true),
            opcode::INVALID => Ok(StepOutcome::Failure),
            opcode::CALL => self.call(executor, state, worklist, completed_paths, CallKind::Call),
            opcode::CALLCODE => {
                self.call(executor, state, worklist, completed_paths, CallKind::CallCode)
            }
            opcode::DELEGATECALL => {
                self.call(executor, state, worklist, completed_paths, CallKind::DelegateCall)
            }
            opcode::STATICCALL => {
                self.call(executor, state, worklist, completed_paths, CallKind::StaticCall)
            }
            opcode::CREATE => {
                self.create(executor, state, worklist, completed_paths, CreateKind::Create)
            }
            opcode::CREATE2 => {
                self.create(executor, state, worklist, completed_paths, CreateKind::Create2)
            }
            opcode::SELFDESTRUCT => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let beneficiary = state.pop_address_or_symbolic_slot()?;
                state.world.selfdestruct(executor, state.address, beneficiary)?;
                state.return_data = SymReturnData::default();
                Ok(StepOutcome::Halt)
            }
            opcode::CHAINID => {
                let value = state.block.chain_id.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BASEFEE => {
                let value = state.block.basefee.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::GASPRICE => {
                let gas_price = state.gas_price.clone();
                state.stack.push(gas_price)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BLOBHASH => {
                let index = state.stack.pop()?;
                let index = state.expect_constrained_usize(index, "symbolic BLOBHASH index")?;
                let hash = state.block.blob_hash(index);
                state.stack.push(SymWord::Concrete(U256::from_be_slice(hash.as_slice())))?;
                Ok(StepOutcome::Continue)
            }
            opcode::COINBASE => {
                let coinbase = state.block.coinbase;
                state.stack.push(SymWord::Concrete(address_word(coinbase)))?;
                Ok(StepOutcome::Continue)
            }
            opcode::TIMESTAMP => {
                let value = state.block.timestamp.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::NUMBER => {
                let value = state.block.number.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::DIFFICULTY => {
                let value = state.block.difficulty.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::GASLIMIT => {
                let value = state.block.gaslimit.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BLOBBASEFEE => {
                let value = state.block.blob_basefee.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::LOG0 | opcode::LOG1 | opcode::LOG2 | opcode::LOG3 | opcode::LOG4 => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let topics = (op - opcode::LOG0) as usize;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                let (data_len, data) = match state.constrained_usize(&size) {
                    Some(size) => (
                        SymWord::Concrete(U256::from(size)),
                        state.memory.read_bytes_offset(offset, size),
                    ),
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic LOG size",
                                )
                            })?;
                        let data =
                            state.memory.read_bytes_symbolic_size(offset, size.clone(), max_size);
                        (size, data)
                    }
                };
                let mut log_topics = Vec::with_capacity(topics);
                for _ in 0..topics {
                    log_topics.push(state.stack.pop()?);
                }
                self.handle_log(
                    state,
                    SymbolicLog { topics: log_topics, data_len, data, emitter: state.address },
                )
            }
            _ => Err(SymbolicError::UnsupportedOpcode(op)),
        }
    }

    /// Implements the `assume_returndata_copy_in_bounds` symbolic executor helper.
    fn assume_returndata_copy_in_bounds(
        &mut self,
        state: &mut PathState,
        offset: SymWord,
        size: SymWord,
    ) -> Result<bool, SymbolicError> {
        let end = Expr::op(ExprOp::Add, offset.into_expr(), size.into_expr());
        let in_bounds = BoolExpr::cmp(BoolExprOp::Ule, end, state.return_data.len_expr());
        match in_bounds {
            BoolExpr::Const(value) => Ok(value),
            in_bounds => {
                let mut constraints = state.constraints.clone();
                constraints.push(in_bounds);
                if self.solver.is_sat(&constraints)? {
                    state.constraints = constraints;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Implements the `return_or_revert` symbolic executor helper.
    fn return_or_revert(
        &mut self,
        state: &mut PathState,
        is_revert: bool,
    ) -> Result<StepOutcome, SymbolicError> {
        let offset = state.stack.pop()?;
        let size = state.stack.pop()?;
        match state.constrained_usize(&size) {
            Some(size) => {
                state.return_data = state.memory.return_data(offset.clone(), size)?;
                if is_revert {
                    Ok(self.classify_revert(state, offset, size))
                } else {
                    Ok(StepOutcome::Halt)
                }
            }
            None if state.constrained_word(&size).is_some() => Ok(StepOutcome::Revert),
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &size,
                            max_limit,
                            if is_revert { "symbolic REVERT size" } else { "symbolic RETURN size" },
                        )
                    })?;
                state.return_data =
                    state.memory.return_data_symbolic_size(offset, size, max_size)?;
                Ok(if is_revert { StepOutcome::Revert } else { StepOutcome::Halt })
            }
        }
    }

    /// Runs the `classify_revert` symbolic executor helper.
    fn classify_revert(&self, state: &PathState, offset: SymWord, size: usize) -> StepOutcome {
        if state.call_depth == 0
            && let SymWord::Concrete(offset) = offset
            && offset <= U256::from(usize::MAX)
            && let Ok(data) = state.memory.read_concrete(offset.to::<usize>(), size)
            && is_assertion_revert(&data)
        {
            StepOutcome::Failure
        } else {
            StepOutcome::Revert
        }
    }

    /// Implements the `call` symbolic executor helper.
    fn call(
        &mut self,
        executor: &Executor<impl FoundryEvmNetwork>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallKind,
    ) -> Result<StepOutcome, SymbolicError> {
        let pre_call_state = state.clone();
        let call_pc = state.pc.saturating_sub(1);
        let gas = state.stack.pop()?;
        let target = state.stack.pop()?;
        let target_address = state.world.resolve_address(&target);
        let value = match (kind, target_address) {
            (CallKind::Call, Some(to)) if is_known_cheatcode(to) => {
                let value = state.stack.pop()?;
                let value = state.expect_constrained_word(value, "symbolic CALL value")?;
                SymWord::Concrete(value)
            }
            (CallKind::Call, _) => state.stack.pop()?,
            (CallKind::CallCode, _) => state.stack.pop()?,
            (CallKind::StaticCall | CallKind::DelegateCall, _) => SymWord::zero(),
        };
        let in_offset = state.stack.pop()?;
        let in_size = state.stack.pop()?;
        let in_size = match state.constrained_usize(&in_size) {
            Some(size) => BoundedCopySize::Concrete(size),
            None if state.constrained_word(&in_size).is_some() => {
                return Ok(StepOutcome::Revert);
            }
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&in_size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &in_size,
                            max_limit,
                            "symbolic CALL input size",
                        )
                    })?;
                BoundedCopySize::Symbolic { size: in_size, max_size }
            }
        };
        let out_offset = state.stack.pop()?;
        let out_size = state.stack.pop()?;
        let out_size = match state.constrained_usize(&out_size) {
            Some(size) => BoundedCopySize::Concrete(size),
            None if state.constrained_word(&out_size).is_some() => {
                return Ok(StepOutcome::Revert);
            }
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&out_size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &out_size,
                            max_limit,
                            "symbolic CALL output size",
                        )
                    })?;
                BoundedCopySize::Symbolic { size: out_size, max_size }
            }
        };

        if state.is_static && !state.constrained_word(&value).is_some_and(|value| value.is_zero()) {
            state.return_data = SymReturnData::default();
            return Ok(StepOutcome::Revert);
        }

        if let Some(to) = target_address {
            let call_input = call_input_from_memory(&state.memory, in_offset.clone(), &in_size);
            if self.branch_symbolic_function_mock_if_needed(
                state,
                worklist,
                &pre_call_state,
                call_pc,
                to,
                &call_input,
            )? {
                return Ok(StepOutcome::Forked);
            }
            let code_address = self.function_mock_target(state, to, &call_input)?.unwrap_or(to);
            if self.branch_symbolic_call_value_if_needed(
                state,
                worklist,
                &pre_call_state,
                call_pc,
                to,
                code_address,
                &value,
                &gas,
                &call_input,
            )? {
                return Ok(StepOutcome::Forked);
            }
            let concrete_value = state.constrained_word(&value);
            if self.branch_symbolic_call_match_if_needed(
                state,
                worklist,
                &pre_call_state,
                call_pc,
                to,
                code_address,
                concrete_value,
                &gas,
                &call_input,
            )? {
                return Ok(StepOutcome::Forked);
            }
            return self.call_concrete_target(
                executor,
                state,
                worklist,
                completed_paths,
                kind,
                to,
                Some(target),
                value,
                gas,
                in_offset,
                in_size,
                out_offset,
                out_size,
            );
        }

        self.call_symbolic_target(
            executor,
            state,
            worklist,
            completed_paths,
            kind,
            target,
            value,
            gas,
            in_offset,
            in_size,
            out_offset,
            out_size,
        )
    }

    #[expect(clippy::too_many_arguments)]
    /// Implements the `branch_symbolic_call_value_if_needed` symbolic executor helper.
    fn branch_symbolic_call_value_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        to: Address,
        code_address: Address,
        value: &SymWord,
        gas: &SymWord,
        call_input: &[SymWord],
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(value).is_some() {
            return Ok(false);
        }

        let mut candidates = BTreeSet::new();
        for expected in &state.expected_calls {
            let Some(expected_value) = expected.value else { continue };
            if self
                .expected_call_match_constraints(
                    state,
                    expected,
                    to,
                    Some(expected_value),
                    gas,
                    call_input,
                )?
                .is_some()
            {
                candidates.insert(expected_value);
            }
        }
        for mock in &state.call_mocks {
            let Some(mock_value) = mock.value else { continue };
            if self
                .call_mock_match_constraints(
                    state,
                    mock,
                    code_address,
                    Some(mock_value),
                    call_input,
                )?
                .is_some()
            {
                candidates.insert(mock_value);
            }
        }

        for candidate in candidates {
            let eq = BoolExpr::eq(value.clone().into_expr(), Expr::Const(candidate));
            let (eq_constraints, eq_sat) = self.constraints_with_condition(state, eq.clone())?;
            let (neq_constraints, neq_sat) = self.constraints_with_condition(state, eq.not())?;

            match (eq_sat, neq_sat) {
                (true, true) => {
                    let mut eq_state = pre_call_state.clone();
                    eq_state.pc = call_pc;
                    eq_state.constraints = eq_constraints;
                    worklist.push_back(eq_state);

                    let mut neq_state = pre_call_state.clone();
                    neq_state.pc = call_pc;
                    neq_state.constraints = neq_constraints;
                    worklist.push_back(neq_state);
                    return Ok(true);
                }
                (true, false) => {
                    state.constraints = eq_constraints;
                    return Ok(false);
                }
                (false, true) => {
                    state.constraints = neq_constraints;
                }
                (false, false) => return Ok(false),
            }
        }

        Ok(false)
    }

    /// Implements the `branch_symbolic_function_mock_if_needed` symbolic executor helper.
    fn branch_symbolic_function_mock_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        callee: Address,
        calldata: &[SymWord],
    ) -> Result<bool, SymbolicError> {
        let function_mocks = state.function_mocks.clone();
        for mock in function_mocks.iter().rev().cloned() {
            if mock.data.len() != calldata.len() {
                continue;
            }
            let Some(condition) = function_mock_match_condition(
                &mock,
                callee,
                calldata,
                "symbolic vm.mockFunction calldata",
            )?
            else {
                continue;
            };
            if self.branch_symbolic_match_condition_if_needed(
                state,
                worklist,
                pre_call_state,
                call_pc,
                condition,
            )? {
                return Ok(true);
            }
        }

        for mock in function_mocks.iter().rev().cloned() {
            if mock.data.len() != 4 {
                continue;
            }
            let Some(condition) = function_mock_match_condition(
                &mock,
                callee,
                calldata,
                "symbolic vm.mockFunction selector",
            )?
            else {
                continue;
            };
            if self.branch_symbolic_match_condition_if_needed(
                state,
                worklist,
                pre_call_state,
                call_pc,
                condition,
            )? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Applies the `observe_expected_call` symbolic executor helper.
    fn observe_expected_call(
        &mut self,
        state: &mut PathState,
        callee: Address,
        value: Option<U256>,
        gas: &SymWord,
        calldata: &[SymWord],
    ) -> Result<bool, SymbolicError> {
        if state.expected_calls.is_empty() {
            return Ok(true);
        }
        for idx in 0..state.expected_calls.len() {
            let expected = state.expected_calls[idx].clone();
            if let Some(constraints) = self
                .expected_call_match_constraints(state, &expected, callee, value, gas, calldata)?
            {
                state.constraints = constraints;
                return Ok(state.expected_calls[idx].observe());
            }
        }
        Ok(true)
    }

    #[expect(clippy::too_many_arguments)]
    /// Implements the `branch_symbolic_call_match_if_needed` symbolic executor helper.
    fn branch_symbolic_call_match_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        callee: Address,
        code_address: Address,
        value: Option<U256>,
        gas: &SymWord,
        calldata: &[SymWord],
    ) -> Result<bool, SymbolicError> {
        let expected_calls = state.expected_calls.clone();
        for expected in expected_calls {
            let Some(condition) =
                self.expected_call_match_condition(&expected, callee, value, gas, calldata)?
            else {
                continue;
            };
            if self.branch_symbolic_match_condition_if_needed(
                state,
                worklist,
                pre_call_state,
                call_pc,
                condition,
            )? {
                return Ok(true);
            }
        }

        let mut mocks = state.call_mocks.iter().cloned().enumerate().collect::<Vec<_>>();
        mocks.sort_by_key(|(idx, mock)| {
            (std::cmp::Reverse(mock.data.len()), std::cmp::Reverse(mock.value.is_some()), *idx)
        });

        for (_, mock) in mocks {
            let Some(condition) =
                self.call_mock_match_condition(&mock, code_address, value, calldata)?
            else {
                continue;
            };
            if self.branch_symbolic_match_condition_if_needed(
                state,
                worklist,
                pre_call_state,
                call_pc,
                condition,
            )? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Implements the `take_call_mock` symbolic executor helper.
    fn take_call_mock(
        &mut self,
        state: &mut PathState,
        callee: Address,
        value: Option<U256>,
        calldata: &[SymWord],
    ) -> Result<Option<CallMockOutcome>, SymbolicError> {
        if state.call_mocks.is_empty() {
            return Ok(None);
        }
        let mut best = None;
        for idx in 0..state.call_mocks.len() {
            let mock = state.call_mocks[idx].clone();
            let Some(constraints) =
                self.call_mock_match_constraints(state, &mock, callee, value, calldata)?
            else {
                continue;
            };
            let specificity = (mock.data.len(), mock.value.is_some());
            if best.as_ref().is_none_or(
                |(_, best_specificity, _): &(usize, (usize, bool), Vec<BoolExpr>)| {
                    specificity > *best_specificity
                },
            ) {
                best = Some((idx, specificity, constraints));
            }
        }
        let Some((idx, _, constraints)) = best else {
            return Ok(None);
        };
        state.constraints = constraints;
        Ok(Some(state.call_mocks[idx].next_outcome()))
    }

    /// Implements the `branch_symbolic_match_condition_if_needed` symbolic executor helper.
    fn branch_symbolic_match_condition_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        condition: BoolExpr,
    ) -> Result<bool, SymbolicError> {
        let (match_constraints, match_sat) =
            self.constraints_with_condition(state, condition.clone())?;
        let (mismatch_constraints, mismatch_sat) =
            self.constraints_with_condition(state, condition.not())?;

        match (match_sat, mismatch_sat) {
            (true, true) => {
                let mut match_state = pre_call_state.clone();
                match_state.pc = call_pc;
                match_state.constraints = match_constraints;
                worklist.push_back(match_state);

                let mut mismatch_state = pre_call_state.clone();
                mismatch_state.pc = call_pc;
                mismatch_state.constraints = mismatch_constraints;
                worklist.push_back(mismatch_state);
                Ok(true)
            }
            (true, false) => {
                state.constraints = match_constraints;
                Ok(false)
            }
            (false, true) => {
                state.constraints = mismatch_constraints;
                Ok(false)
            }
            (false, false) => Ok(false),
        }
    }

    /// Implements the `function_mock_target` symbolic executor helper.
    fn function_mock_target(
        &mut self,
        state: &mut PathState,
        callee: Address,
        calldata: &[SymWord],
    ) -> Result<Option<Address>, SymbolicError> {
        for mock in state.function_mocks.iter().rev().cloned() {
            if mock.data.len() != calldata.len() {
                continue;
            }
            let Some(condition) = function_mock_match_condition(
                &mock,
                callee,
                calldata,
                "symbolic vm.mockFunction calldata",
            )?
            else {
                continue;
            };
            if let Some(constraints) = self.constraints_for_condition(state, condition)? {
                state.constraints = constraints;
                return Ok(Some(mock.target));
            }
        }
        for mock in state.function_mocks.iter().rev().cloned() {
            if mock.data.len() != 4 {
                continue;
            }
            let Some(condition) = function_mock_match_condition(
                &mock,
                callee,
                calldata,
                "symbolic vm.mockFunction selector",
            )?
            else {
                continue;
            };
            if let Some(constraints) = self.constraints_for_condition(state, condition)? {
                state.constraints = constraints;
                return Ok(Some(mock.target));
            }
        }
        Ok(None)
    }

    /// Implements the `expected_call_match_constraints` symbolic executor helper.
    fn expected_call_match_constraints(
        &mut self,
        state: &PathState,
        expected: &ExpectedCall,
        callee: Address,
        value: Option<U256>,
        gas: &SymWord,
        calldata: &[SymWord],
    ) -> Result<Option<Vec<BoolExpr>>, SymbolicError> {
        let Some(condition) =
            self.expected_call_match_condition(expected, callee, value, gas, calldata)?
        else {
            return Ok(None);
        };
        self.constraints_for_condition(state, condition)
    }

    /// Implements the `call_mock_match_constraints` symbolic executor helper.
    fn call_mock_match_constraints(
        &mut self,
        state: &PathState,
        mock: &CallMock,
        callee: Address,
        value: Option<U256>,
        calldata: &[SymWord],
    ) -> Result<Option<Vec<BoolExpr>>, SymbolicError> {
        let Some(condition) = self.call_mock_match_condition(mock, callee, value, calldata)? else {
            return Ok(None);
        };
        self.constraints_for_condition(state, condition)
    }

    /// Implements the `expected_call_match_condition` symbolic executor helper.
    fn expected_call_match_condition(
        &self,
        expected: &ExpectedCall,
        callee: Address,
        value: Option<U256>,
        gas: &SymWord,
        calldata: &[SymWord],
    ) -> Result<Option<BoolExpr>, SymbolicError> {
        if !expected.static_parts_match(value, gas)? {
            return Ok(None);
        }
        let Some(data_condition) =
            calldata_prefix_condition(calldata, &expected.data, "symbolic expected call calldata")?
        else {
            return Ok(None);
        };
        Ok(Some(BoolExpr::and(vec![
            address_match_condition(&expected.callee, callee),
            data_condition,
        ])))
    }

    /// Implements the `call_mock_match_condition` symbolic executor helper.
    fn call_mock_match_condition(
        &self,
        mock: &CallMock,
        callee: Address,
        value: Option<U256>,
        calldata: &[SymWord],
    ) -> Result<Option<BoolExpr>, SymbolicError> {
        if !mock.static_parts_match(value) {
            return Ok(None);
        }
        let Some(data_condition) =
            calldata_prefix_condition(calldata, &mock.data, "symbolic mocked call calldata")?
        else {
            return Ok(None);
        };
        Ok(Some(BoolExpr::and(vec![address_match_condition(&mock.callee, callee), data_condition])))
    }

    /// Returns whether `expected_revert_matches` holds.
    fn expected_revert_matches(
        &mut self,
        state: &mut PathState,
        expected: &ExpectedRevert,
        reverter: Address,
        return_data: &SymReturnData,
    ) -> Result<bool, SymbolicError> {
        let Some(condition) = expected_revert_match_condition(expected, reverter, return_data)
        else {
            return Ok(false);
        };

        let (match_constraints, match_sat) =
            self.constraints_with_condition(state, condition.clone())?;
        if !match_sat {
            return Ok(false);
        }

        let (mismatch_constraints, mismatch_sat) =
            self.constraints_with_condition(state, condition.not())?;
        if mismatch_sat {
            state.constraints = mismatch_constraints;
            return Ok(false);
        }

        state.constraints = match_constraints;
        Ok(true)
    }

    /// Implements the `assume_no_revert_rejects` symbolic executor helper.
    fn assume_no_revert_rejects(
        &mut self,
        state: &mut PathState,
        assumption: &AssumeNoRevert,
        reverter: Address,
        return_data: &SymReturnData,
    ) -> Result<bool, SymbolicError> {
        let AssumeNoRevert::Filtered(filters) = assumption else {
            return Ok(true);
        };

        let conditions = filters
            .iter()
            .filter_map(|filter| expected_revert_match_condition(filter, reverter, return_data))
            .collect::<Vec<_>>();
        if conditions.is_empty() {
            return Ok(false);
        }

        let condition = BoolExpr::or(conditions);
        let (_match_constraints, match_sat) =
            self.constraints_with_condition(state, condition.clone())?;
        if !match_sat {
            return Ok(false);
        }

        let (mismatch_constraints, mismatch_sat) =
            self.constraints_with_condition(state, condition.not())?;
        if mismatch_sat {
            state.constraints = mismatch_constraints;
            return Ok(false);
        }

        Ok(true)
    }

    /// Implements the `constraints_for_condition` symbolic executor helper.
    fn constraints_for_condition(
        &mut self,
        state: &PathState,
        condition: BoolExpr,
    ) -> Result<Option<Vec<BoolExpr>>, SymbolicError> {
        let (constraints, sat) = self.constraints_with_condition(state, condition)?;
        Ok(sat.then_some(constraints))
    }

    /// Implements the `constraints_with_condition` symbolic executor helper.
    fn constraints_with_condition(
        &mut self,
        state: &PathState,
        condition: BoolExpr,
    ) -> Result<(Vec<BoolExpr>, bool), SymbolicError> {
        match condition {
            BoolExpr::Const(true) => Ok((state.constraints.clone(), true)),
            BoolExpr::Const(false) => Ok((state.constraints.clone(), false)),
            condition => {
                let mut constraints = state.constraints.clone();
                constraints.push(condition);
                let sat = self.solver.is_sat(&constraints)?;
                Ok((constraints, sat))
            }
        }
    }

    /// Implements the `take_loop_jump` symbolic executor helper.
    fn take_loop_jump(&self, state: &mut PathState, source_pc: usize, dest: usize) -> bool {
        let Some(bound) = self.config.loop_bound else {
            return true;
        };
        if dest >= source_pc {
            return true;
        }
        let count = state.loop_jumps.entry(dest).or_default();
        if *count >= bound {
            return false;
        }
        *count += 1;
        true
    }

    /// Runs the `handle_log` symbolic executor helper.
    fn handle_log(
        &mut self,
        state: &mut PathState,
        log: SymbolicLog,
    ) -> Result<StepOutcome, SymbolicError> {
        let Some(mut expected) = state.expected_emit.take() else {
            state.record_log(log);
            return Ok(StepOutcome::Continue);
        };

        if let Some(template) = expected.template.clone() {
            if !self.expected_emit_matches(state, &expected, &template, &log)? {
                state.expected_emit = Some(expected);
                state.record_log(log);
                return Ok(StepOutcome::Failure);
            }
            expected.consume_one();
            if !expected.is_satisfied() {
                state.expected_emit = Some(expected);
            }
        } else {
            expected.template = Some(log.clone());
            state.expected_emit = Some(expected);
        }

        state.record_log(log);
        Ok(StepOutcome::Continue)
    }

    /// Returns whether `expected_emit_matches` holds.
    fn expected_emit_matches(
        &mut self,
        state: &mut PathState,
        expected: &ExpectedEmit,
        template: &SymbolicLog,
        actual: &SymbolicLog,
    ) -> Result<bool, SymbolicError> {
        let mut conditions = Vec::new();
        if let Some(expected_emitter) = &expected.emitter {
            conditions.push(address_match_condition(expected_emitter, actual.emitter));
        }
        for idx in 0..expected.checks.topics.len() {
            if !expected.checks.topics[idx] {
                continue;
            }
            match (template.topics.get(idx), actual.topics.get(idx)) {
                (Some(left), Some(right)) => {
                    conditions
                        .push(BoolExpr::eq(left.clone().into_expr(), right.clone().into_expr()));
                }
                (None, None) => {}
                _ => return Ok(false),
            }
        }

        if expected.checks.data {
            conditions.push(BoolExpr::eq(
                template.data_len.clone().into_expr(),
                actual.data_len.clone().into_expr(),
            ));
            if template.data.len() != actual.data.len() {
                return Ok(false);
            }
            conditions.extend(
                template
                    .data
                    .iter()
                    .cloned()
                    .zip(actual.data.iter().cloned())
                    .map(|(left, right)| BoolExpr::eq(left.into_expr(), right.into_expr())),
            );
        }

        let condition = BoolExpr::and(conditions);
        let (match_constraints, match_sat) =
            self.constraints_with_condition(state, condition.clone())?;
        if !match_sat {
            return Ok(false);
        }

        let (mismatch_constraints, mismatch_sat) =
            self.constraints_with_condition(state, condition.not())?;
        if mismatch_sat {
            state.constraints = mismatch_constraints;
            return Ok(false);
        }

        state.constraints = match_constraints;
        Ok(true)
    }

    #[expect(clippy::too_many_arguments)]
    /// Implements the `call_concrete_target` symbolic executor helper.
    fn call_concrete_target<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallKind,
        to: Address,
        target_word: Option<SymWord>,
        value: SymWord,
        gas: SymWord,
        in_offset: SymWord,
        in_size: BoundedCopySize,
        out_offset: SymWord,
        out_size: BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        if is_known_cheatcode(to) {
            if !state.constrained_word(&value).is_some_and(|value| value.is_zero()) {
                return Err(SymbolicError::Unsupported("value-bearing cheatcode CALL"));
            }
            let (in_size_word, in_size, has_symbolic_in_size) = bounded_copy_size_parts(&in_size);
            if in_size < 4 {
                return Err(SymbolicError::Unsupported("short cheatcode CALL"));
            }
            let in_offset = in_offset.into_usize("symbolic cheatcode CALL input offset")?;
            if !self.assume_word_at_least(state, &in_size_word, 4)? {
                return Ok(StepOutcome::AssumeRejected);
            }

            let selector = state
                .memory
                .read_concrete(in_offset, 4)?
                .try_into()
                .map_err(|_| SymbolicError::Unsupported("symbolic cheatcode selector"))?;
            if has_symbolic_in_size {
                let min_size = if to == CHEATCODE_ADDRESS {
                    foundry_cheatcode_min_input_size(selector)
                } else if to == SYMBOLIC_VM_COMPAT_ADDRESS {
                    symbolic_vm_cheatcode_min_input_size(selector)
                } else {
                    None
                }
                .ok_or(SymbolicError::Unsupported("symbolic cheatcode CALL input size"))?;
                if min_size > in_size {
                    return Err(SymbolicError::Unsupported("symbolic cheatcode CALL input size"));
                }
                if !self.assume_word_at_least(state, &in_size_word, min_size)? {
                    return Ok(StepOutcome::AssumeRejected);
                }
            }

            if to == CHEATCODE_ADDRESS
                && let Some(outcome) = self.branch_accesses_cheatcode_if_needed(
                    state,
                    worklist,
                    selector,
                    in_offset,
                    out_offset.clone(),
                    &out_size,
                )?
            {
                return Ok(outcome);
            }

            let return_data = if to == CHEATCODE_ADDRESS {
                match self
                    .handle_foundry_cheatcode(executor, state, selector, in_offset, in_size)?
                {
                    CheatcodeOutcome::Continue(ret) => SymReturnData::from_words(ret),
                    CheatcodeOutcome::ContinueData(ret) => ret,
                    CheatcodeOutcome::AssumeRejected => return Ok(StepOutcome::AssumeRejected),
                    CheatcodeOutcome::Failure => return Ok(StepOutcome::Failure),
                }
            } else if to == SYMBOLIC_VM_COMPAT_ADDRESS {
                self.handle_symbolic_vm_cheatcode(state, selector, in_offset)?
            } else {
                return Err(SymbolicError::Unsupported("symbolic cheatcode address"));
            };

            state.return_data = return_data;
            let return_data = state.return_data.clone();
            state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
            state.stack.push(SymWord::Concrete(U256::from(1)))?;
            return Ok(StepOutcome::Continue);
        }

        if is_console(to) {
            state.return_data = SymReturnData::default();
            let return_data = state.return_data.clone();
            state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
            state.stack.push(SymWord::Concrete(U256::from(1)))?;
            return Ok(StepOutcome::Continue);
        }

        let call_input = call_input_from_memory(&state.memory, in_offset.clone(), &in_size);
        if !state.expected_calls.is_empty() {
            let concrete_value = state.constrained_word(&value);
            if !self.observe_expected_call(state, to, concrete_value, &gas, &call_input)? {
                return Ok(StepOutcome::Failure);
            }
        }
        let code_address = self.function_mock_target(state, to, &call_input)?.unwrap_or(to);
        if !state.call_mocks.is_empty() {
            let concrete_value = state.constrained_word(&value);
            if let Some(mock) =
                self.take_call_mock(state, code_address, concrete_value, &call_input)?
            {
                if !matches!(kind, CallKind::DelegateCall) {
                    let _ = state.prank_for_next_call();
                }
                state.return_data = mock.return_data;
                let return_data = state.return_data.clone();
                state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
                state.stack.push(SymWord::Concrete(U256::from(!mock.reverts)))?;
                return Ok(StepOutcome::Continue);
            }
        }

        if matches!(kind, CallKind::Call)
            && !self.prepare_value_transfer(
                executor,
                state,
                worklist,
                value.clone(),
                out_offset.clone(),
                &out_size,
            )?
        {
            return Ok(StepOutcome::Continue);
        }

        if is_supported_precompile(code_address) {
            let input_len = bounded_copy_size_word(&in_size);
            let input = call_input_from_memory(&state.memory, in_offset, &in_size);
            match execute_symbolic_precompile(code_address, input, input_len)? {
                Some(return_data) => {
                    state.return_data = return_data;
                    if matches!(kind, CallKind::Call) {
                        state.world.transfer(executor, state.address, to, value);
                    }
                    let return_data = state.return_data.clone();
                    state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
                    state.stack.push(SymWord::Concrete(U256::from(1)))?;
                }
                None => {
                    state.return_data = SymReturnData::default();
                    let return_data = state.return_data.clone();
                    state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
                    state.stack.push(SymWord::zero())?;
                }
            }
            return Ok(StepOutcome::Continue);
        }

        let child_code = state.world.extcode(executor, code_address)?;
        if child_code.is_empty() {
            if matches!(kind, CallKind::Call) {
                state.world.transfer(executor, state.address, to, value);
            }
            state.return_data = SymReturnData::default();
            let return_data = state.return_data.clone();
            state.memory.copy_call_output_offset(out_offset, &out_size, &return_data)?;
            state.stack.push(SymWord::Concrete(U256::from(1)))?;
            return Ok(StepOutcome::Continue);
        }

        let calldata = calldata_from_call_input(call_input, &in_size);
        let callee_address_word = state
            .world
            .symbolic_word_for_address(to)
            .or_else(|| {
                target_word
                    .as_ref()
                    .filter(|word| state.world.resolve_address(word) == Some(to))
                    .cloned()
            })
            .unwrap_or_else(|| SymWord::Concrete(address_word(to)));
        let (pranked_caller, pranked_caller_word, pranked_origin) = state.prank_for_next_call();
        let frame = match kind {
            CallKind::Call => {
                let mut frame = CallFrame::new(
                    to,
                    code_address,
                    to,
                    pranked_caller,
                    value.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = callee_address_word;
                frame.caller_word = pranked_caller_word;
                frame
            }
            CallKind::StaticCall => {
                let mut frame = CallFrame::new(
                    to,
                    code_address,
                    to,
                    pranked_caller,
                    SymWord::zero(),
                    true,
                    calldata,
                );
                frame.address_word = callee_address_word;
                frame.caller_word = pranked_caller_word;
                frame
            }
            CallKind::DelegateCall => {
                let mut frame = CallFrame::new(
                    state.address,
                    code_address,
                    state.storage_address,
                    state.caller,
                    state.callvalue.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = state.address_word.clone();
                frame.caller_word = state.caller_word.clone();
                frame
            }
            CallKind::CallCode => {
                let mut frame = CallFrame::new(
                    state.address,
                    code_address,
                    state.storage_address,
                    pranked_caller,
                    value.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = state.address_word.clone();
                frame.caller_word = pranked_caller_word;
                frame
            }
        };

        let original_world = state.world.clone();
        let mut child = state.child(frame);
        if let Some((origin, origin_word)) = pranked_origin {
            child.origin = origin;
            child.origin_word = origin_word;
        }
        if matches!(kind, CallKind::Call) {
            child.world.transfer(executor, state.address, to, value);
        }
        child.expected_revert = None;
        child.assume_no_revert_next_call = None;
        let outcomes = self.execute_external_call(executor, child, &child_code, completed_paths)?;
        let Some((first, rest)) = outcomes.split_first() else {
            return Ok(StepOutcome::AssumeRejected);
        };

        let mut parents = Vec::with_capacity(outcomes.len());
        for outcome in std::iter::once(first).chain(rest.iter()) {
            let mut parent = state.clone();
            parent.constraints = outcome.state.constraints.clone();
            parent.next_symbol = outcome.state.next_symbol;

            if let Some(assumption) = parent.assume_no_revert_next_call.take()
                && matches!(outcome.status, TopLevelCallStatus::Revert)
                && self.assume_no_revert_rejects(
                    &mut parent,
                    &assumption,
                    to,
                    &outcome.return_data,
                )?
            {
                continue;
            }

            if let Some(mut expected) = parent.expected_revert.clone() {
                match outcome.status {
                    TopLevelCallStatus::Success => {
                        *state = parent;
                        return Ok(StepOutcome::Failure);
                    }
                    TopLevelCallStatus::Revert | TopLevelCallStatus::Failure => {
                        if !self.expected_revert_matches(
                            &mut parent,
                            &expected,
                            to,
                            &outcome.return_data,
                        )? {
                            *state = parent;
                            return Ok(StepOutcome::Failure);
                        }
                        if expected.consume_one() {
                            parent.expected_revert = None;
                        } else {
                            parent.expected_revert = Some(expected);
                        }
                        parent.access_record = outcome.state.access_record.clone();
                        parent.expected_calls = outcome.state.expected_calls.clone();
                        parent.expected_creates = outcome.state.expected_creates.clone();
                        parent.call_mocks = outcome.state.call_mocks.clone();
                        parent.function_mocks = outcome.state.function_mocks.clone();
                        parent.world = original_world.clone();
                        parent.return_data = SymReturnData::default();
                        let return_data = parent.return_data.clone();
                        parent.memory.copy_call_output_offset(
                            out_offset.clone(),
                            &out_size,
                            &return_data,
                        )?;
                        parent.stack.push(SymWord::Concrete(U256::from(1)))?;
                        parents.push(parent);
                        continue;
                    }
                }
            }

            parent.world = if matches!(outcome.status, TopLevelCallStatus::Success) {
                outcome.state.world.clone()
            } else {
                original_world.clone()
            };
            match outcome.status {
                TopLevelCallStatus::Success => {
                    parent.block = outcome.state.block.clone();
                    parent.recorded_logs = outcome.state.recorded_logs.clone();
                    parent.access_record = outcome.state.access_record.clone();
                    parent.expected_emit = outcome.state.expected_emit.clone();
                    parent.expected_calls = outcome.state.expected_calls.clone();
                    parent.expected_creates = outcome.state.expected_creates.clone();
                    parent.call_mocks = outcome.state.call_mocks.clone();
                    parent.function_mocks = outcome.state.function_mocks.clone();
                }
                TopLevelCallStatus::Failure => {
                    *state = parent;
                    return Ok(StepOutcome::Failure);
                }
                TopLevelCallStatus::Revert => {}
            }
            parent.return_data = outcome.return_data.clone();
            let return_data = parent.return_data.clone();
            parent.memory.copy_call_output_offset(out_offset.clone(), &out_size, &return_data)?;
            parent.stack.push(SymWord::Concrete(U256::from(matches!(
                outcome.status,
                TopLevelCallStatus::Success
            ))))?;
            parents.push(parent);
        }

        let mut iter = parents.into_iter();
        let Some(first) = iter.next() else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(iter);
        Ok(StepOutcome::Continue)
    }

    /// Implements the `prepare_value_transfer` symbolic executor helper.
    fn prepare_value_transfer<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        value: SymWord,
        out_offset: SymWord,
        out_size: &BoundedCopySize,
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(&value).is_some_and(|value| value.is_zero()) {
            return Ok(true);
        }

        let balance = state.world.balance_word_for_address(executor, state.address);
        let can_pay = BoolExpr::cmp(BoolExprOp::Uge, balance.into_expr(), value.into_expr());
        match can_pay {
            BoolExpr::Const(true) => Ok(true),
            BoolExpr::Const(false) => {
                state.return_data = SymReturnData::default();
                let return_data = state.return_data.clone();
                state.memory.copy_call_output_offset(out_offset, out_size, &return_data)?;
                state.stack.push(SymWord::zero())?;
                Ok(false)
            }
            can_pay => {
                let mut success_constraints = state.constraints.clone();
                success_constraints.push(can_pay.clone());
                let success_sat = self.solver.is_sat(&success_constraints)?;

                let mut failure_constraints = state.constraints.clone();
                failure_constraints.push(can_pay.not());
                let failure_sat = self.solver.is_sat(&failure_constraints)?;

                match (success_sat, failure_sat) {
                    (true, true) => {
                        let mut failure = state.clone();
                        failure.constraints = failure_constraints;
                        failure.return_data = SymReturnData::default();
                        let return_data = failure.return_data.clone();
                        failure.memory.copy_call_output_offset(
                            out_offset,
                            out_size,
                            &return_data,
                        )?;
                        failure.stack.push(SymWord::zero())?;
                        worklist.push_back(failure);

                        state.constraints = success_constraints;
                        Ok(true)
                    }
                    (true, false) => {
                        state.constraints = success_constraints;
                        Ok(true)
                    }
                    (false, true) => {
                        state.constraints = failure_constraints;
                        state.return_data = SymReturnData::default();
                        let return_data = state.return_data.clone();
                        state.memory.copy_call_output_offset(out_offset, out_size, &return_data)?;
                        state.stack.push(SymWord::zero())?;
                        Ok(false)
                    }
                    (false, false) => Ok(false),
                }
            }
        }
    }

    /// Implements the `prepare_create_value_transfer` symbolic executor helper.
    fn prepare_create_value_transfer<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        value: SymWord,
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(&value).is_some_and(|value| value.is_zero()) {
            return Ok(true);
        }

        let balance = state.world.balance_word_for_address(executor, state.address);
        let can_pay = BoolExpr::cmp(BoolExprOp::Uge, balance.into_expr(), value.into_expr());
        match can_pay {
            BoolExpr::Const(true) => Ok(true),
            BoolExpr::Const(false) => {
                state.return_data = SymReturnData::default();
                state.stack.push(SymWord::zero())?;
                Ok(false)
            }
            can_pay => {
                let mut success_constraints = state.constraints.clone();
                success_constraints.push(can_pay.clone());
                let success_sat = self.solver.is_sat(&success_constraints)?;

                let mut failure_constraints = state.constraints.clone();
                failure_constraints.push(can_pay.not());
                let failure_sat = self.solver.is_sat(&failure_constraints)?;

                match (success_sat, failure_sat) {
                    (true, true) => {
                        let mut failure = state.clone();
                        failure.constraints = failure_constraints;
                        failure.return_data = SymReturnData::default();
                        failure.stack.push(SymWord::zero())?;
                        worklist.push_back(failure);

                        state.constraints = success_constraints;
                        Ok(true)
                    }
                    (true, false) => {
                        state.constraints = success_constraints;
                        Ok(true)
                    }
                    (false, true) => {
                        state.constraints = failure_constraints;
                        state.return_data = SymReturnData::default();
                        state.stack.push(SymWord::zero())?;
                        Ok(false)
                    }
                    (false, false) => Ok(false),
                }
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    /// Implements the `call_symbolic_target` symbolic executor helper.
    fn call_symbolic_target<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallKind,
        target: SymWord,
        value: SymWord,
        gas: SymWord,
        in_offset: SymWord,
        in_size: BoundedCopySize,
        out_offset: SymWord,
        out_size: BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        let target = target.into_expr();
        let mut candidates = state.world.symbolic_call_targets(executor)?;
        candidates.extend((1..=9).map(precompile_address));
        candidates.sort();
        candidates.dedup();
        if candidates.is_empty() {
            return Err(SymbolicError::Unsupported(
                "symbolic CALL target has no known contract candidates",
            ));
        }

        let candidate_constraints = candidates
            .iter()
            .map(|address| BoolExpr::eq(target.clone(), Expr::Const(address_word(*address))))
            .collect::<Vec<_>>();
        let mut outside_constraints = state.constraints.clone();
        outside_constraints.extend(candidate_constraints.iter().cloned().map(BoolExpr::not));
        let outside_sat = self.solver.is_sat(&outside_constraints)?;

        if !self.config.symbolic_call_targets && outside_sat {
            return Err(SymbolicError::Unsupported("symbolic CALL target"));
        }

        let mut parents = VecDeque::new();
        if outside_sat {
            let mut branch = state.clone();
            branch.constraints = outside_constraints;

            if matches!(kind, CallKind::Call) {
                if self.prepare_value_transfer(
                    executor,
                    &mut branch,
                    &mut parents,
                    value.clone(),
                    out_offset.clone(),
                    &out_size,
                )? {
                    let symbolic_target = SymWord::Expr(target);
                    let to = branch.world.symbolic_address_slot(symbolic_target);
                    branch.world.transfer(executor, branch.address, to, value.clone());
                    branch.return_data = SymReturnData::default();
                    let return_data = branch.return_data.clone();
                    branch.memory.copy_call_output_offset(
                        out_offset.clone(),
                        &out_size,
                        &return_data,
                    )?;
                    branch.stack.push(SymWord::Concrete(U256::from(1)))?;
                    parents.push_back(branch);
                }
            } else {
                branch.return_data = SymReturnData::default();
                let return_data = branch.return_data.clone();
                branch.memory.copy_call_output_offset(
                    out_offset.clone(),
                    &out_size,
                    &return_data,
                )?;
                branch.stack.push(SymWord::Concrete(U256::from(1)))?;
                parents.push_back(branch);
            }
        }

        for (to, constraint) in candidates.into_iter().zip(candidate_constraints) {
            let mut branch = state.clone();
            branch.constraints.push(constraint);
            if !self.solver.is_sat(&branch.constraints)? {
                continue;
            }

            let mut branch_worklist = VecDeque::new();
            match self.call_concrete_target(
                executor,
                &mut branch,
                &mut branch_worklist,
                completed_paths,
                kind,
                to,
                None,
                value.clone(),
                gas.clone(),
                in_offset.clone(),
                in_size.clone(),
                out_offset.clone(),
                out_size.clone(),
            )? {
                StepOutcome::Continue => {
                    parents.push_back(branch);
                    parents.extend(branch_worklist);
                }
                StepOutcome::AssumeRejected => {}
                outcome => return Ok(outcome),
            }
        }

        let Some(first) = parents.pop_front() else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(parents);
        Ok(StepOutcome::Continue)
    }

    /// Implements the `create` symbolic executor helper.
    fn create<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CreateKind,
    ) -> Result<StepOutcome, SymbolicError> {
        if state.is_static {
            state.return_data = SymReturnData::default();
            return Ok(StepOutcome::Revert);
        }

        let value = state.stack.pop()?;
        let offset = state.stack.pop()?;
        let size = state.stack.pop()?;
        let size = match state.constrained_usize(&size) {
            Some(size) => BoundedCopySize::Concrete(size),
            None if state.constrained_word(&size).is_some() => {
                state.return_data = SymReturnData::default();
                state.stack.push(SymWord::zero())?;
                return Ok(StepOutcome::Continue);
            }
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&size)
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

        let initcode = match &size {
            BoundedCopySize::Concrete(size) => {
                if let Some(offset) = state.constrained_usize(&offset) {
                    SymCode { bytes: state.memory.read_bytes(offset, *size) }
                } else {
                    SymCode::from_memory_offset(&state.memory, offset, *size)
                }
            }
            BoundedCopySize::Symbolic { size, max_size } => {
                SymCode::from_memory_symbolic_size(&state.memory, offset, size.clone(), *max_size)
            }
        };
        let (created_word, created) = match kind {
            CreateKind::Create => {
                let nonce = state.world.nonce(executor, state.address)?;
                let address = state.address.create(nonce);
                (SymWord::Concrete(address_word(address)), address)
            }
            CreateKind::Create2 => create2_address_word(
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

        if failure_world.has_code_or_nonce(executor, created)? {
            state.world = failure_world;
            state.return_data = SymReturnData::default();
            state.stack.push(SymWord::zero())?;
            return Ok(StepOutcome::Continue);
        }

        let mut frame = CallFrame::new(
            created,
            created,
            created,
            state.address,
            value.clone(),
            false,
            SymCalldata::new(Vec::new()),
        );
        frame.address_word = created_word.clone();
        frame.caller_word = state.address_word.clone();
        let mut child = state.child(frame);
        let pending_expected_creates = std::mem::take(&mut child.expected_creates);
        child.world = failure_world.clone();
        child.world.set_nonce(created, 1);
        child.world.transfer(executor, state.address, created, value);
        child.expected_revert = None;
        child.assume_no_revert_next_call = None;

        let outcomes = self.execute_external_call(executor, child, &initcode, completed_paths)?;
        let Some((first, rest)) = outcomes.split_first() else {
            return Ok(StepOutcome::AssumeRejected);
        };

        let mut parents = Vec::with_capacity(outcomes.len());
        for outcome in std::iter::once(first).chain(rest.iter()) {
            let mut parent = state.clone();
            parent.constraints = outcome.state.constraints.clone();
            parent.next_symbol = outcome.state.next_symbol;
            parent.return_data = SymReturnData::default();

            if let Some(assumption) = parent.assume_no_revert_next_call.take()
                && matches!(outcome.status, TopLevelCallStatus::Revert)
                && self.assume_no_revert_rejects(
                    &mut parent,
                    &assumption,
                    created,
                    &outcome.return_data,
                )?
            {
                continue;
            }

            if let Some(mut expected) = parent.expected_revert.clone() {
                match outcome.status {
                    TopLevelCallStatus::Success => {
                        *state = parent;
                        return Ok(StepOutcome::Failure);
                    }
                    TopLevelCallStatus::Revert | TopLevelCallStatus::Failure => {
                        if !self.expected_revert_matches(
                            &mut parent,
                            &expected,
                            created,
                            &outcome.return_data,
                        )? {
                            *state = parent;
                            return Ok(StepOutcome::Failure);
                        }
                        if expected.consume_one() {
                            parent.expected_revert = None;
                        } else {
                            parent.expected_revert = Some(expected);
                        }
                        parent.access_record = outcome.state.access_record.clone();
                        parent.expected_calls = outcome.state.expected_calls.clone();
                        parent.expected_creates = pending_expected_creates.clone();
                        parent.call_mocks = outcome.state.call_mocks.clone();
                        parent.function_mocks = outcome.state.function_mocks.clone();
                        parent.world = failure_world.clone();
                        parent.stack.push(created_word.clone())?;
                        parents.push(parent);
                        continue;
                    }
                }
            }

            match outcome.status {
                TopLevelCallStatus::Success => {
                    parent.world = outcome.state.world.clone();
                    parent.block = outcome.state.block.clone();
                    parent.recorded_logs = outcome.state.recorded_logs.clone();
                    parent.access_record = outcome.state.access_record.clone();
                    parent.expected_emit = outcome.state.expected_emit.clone();
                    parent.expected_calls = outcome.state.expected_calls.clone();
                    parent.expected_creates = pending_expected_creates.clone();
                    parent.call_mocks = outcome.state.call_mocks.clone();
                    parent.function_mocks = outcome.state.function_mocks.clone();
                    self.observe_expected_create(
                        &mut parent,
                        state.address,
                        kind,
                        &outcome.return_data,
                    )?;
                    parent.world.install_code(created, outcome.return_data.to_code());
                    parent.world.set_nonce(created, 1);
                    parent.stack.push(created_word.clone())?;
                }
                TopLevelCallStatus::Revert => {
                    parent.world = failure_world.clone();
                    parent.stack.push(SymWord::zero())?;
                }
                TopLevelCallStatus::Failure => {
                    *state = parent;
                    return Ok(StepOutcome::Failure);
                }
            }

            parents.push(parent);
        }

        let mut iter = parents.into_iter();
        let Some(first) = iter.next() else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(iter);
        Ok(StepOutcome::Continue)
    }

    /// Computes the `execute_external_call` symbolic executor helper result.
    fn execute_external_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        initial: PathState,
        code: &SymCode,
        completed_paths: &mut usize,
    ) -> Result<Vec<ExternalCallOutcome>, SymbolicError> {
        let jumpdests = analyze_jumpdests(code);
        let mut worklist = VecDeque::from([initial]);
        let mut outcomes = Vec::new();
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        while let Some(mut state) = worklist.pop_front() {
            if *completed_paths >= path_limit {
                return Err(SymbolicError::Unsupported("symbolic path limit exceeded"));
            }

            loop {
                if state.depth >= depth_limit {
                    return Err(SymbolicError::Unsupported("symbolic depth limit exceeded"));
                }
                state.depth += 1;

                let op = match code.guarded_opcode(state.pc)? {
                    GuardedOpcode::End => {
                        *completed_paths += 1;
                        outcomes.push(ExternalCallOutcome {
                            status: if state.expectations_satisfied() {
                                TopLevelCallStatus::Success
                            } else {
                                TopLevelCallStatus::Failure
                            },
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    GuardedOpcode::Concrete(op) => op,
                    GuardedOpcode::SymbolicSize { condition, opcode } => {
                        let mut in_bounds_constraints = state.constraints.clone();
                        in_bounds_constraints.push(condition.clone());
                        let in_bounds_sat = self.solver.is_sat(&in_bounds_constraints)?;

                        let mut out_of_bounds_constraints = state.constraints.clone();
                        out_of_bounds_constraints.push(condition.not());
                        if self.solver.is_sat(&out_of_bounds_constraints)? {
                            let mut halted = state.clone();
                            halted.constraints = out_of_bounds_constraints;
                            *completed_paths += 1;
                            outcomes.push(ExternalCallOutcome {
                                status: if halted.expectations_satisfied() {
                                    TopLevelCallStatus::Success
                                } else {
                                    TopLevelCallStatus::Failure
                                },
                                return_data: halted.return_data.clone(),
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
                    &jumpdests,
                    &mut state,
                    &mut worklist,
                    completed_paths,
                    op,
                )? {
                    StepOutcome::Continue => {}
                    StepOutcome::Halt => {
                        *completed_paths += 1;
                        outcomes.push(ExternalCallOutcome {
                            status: if state.expectations_satisfied() {
                                TopLevelCallStatus::Success
                            } else {
                                TopLevelCallStatus::Failure
                            },
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::Revert => {
                        *completed_paths += 1;
                        outcomes.push(ExternalCallOutcome {
                            status: TopLevelCallStatus::Revert,
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::Failure => {
                        *completed_paths += 1;
                        outcomes.push(ExternalCallOutcome {
                            status: TopLevelCallStatus::Failure,
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::AssumeRejected | StepOutcome::Forked => break,
                }
            }
        }

        Ok(outcomes)
    }

    /// Runs the `handle_assertion` symbolic executor helper.
    fn handle_assertion(
        &mut self,
        state: &mut PathState,
        pass: BoolExpr,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let fail = pass.clone().not();
        match fail {
            BoolExpr::Const(true) => return Ok(CheatcodeOutcome::Failure),
            BoolExpr::Const(false) => return Ok(CheatcodeOutcome::Continue(Vec::new())),
            _ => {}
        }

        let mut fail_constraints = state.constraints.clone();
        fail_constraints.push(fail);
        if self.solver.is_sat(&fail_constraints)? {
            state.constraints = fail_constraints;
            return Ok(CheatcodeOutcome::Failure);
        }

        state.constraints.push(pass);
        Ok(CheatcodeOutcome::Continue(Vec::new()))
    }

    /// Applies the `set_expected_revert` symbolic executor helper.
    fn set_expected_revert(
        &mut self,
        state: &mut PathState,
        data: ExpectedRevertData,
        reverter: Option<SymWord>,
        remaining: u64,
    ) -> CheatcodeOutcome {
        state.expected_revert =
            Some(ExpectedRevert { data, reverter, remaining: remaining.max(1) });
        CheatcodeOutcome::Continue(Vec::new())
    }

    /// Applies the `set_expected_emit` symbolic executor helper.
    fn set_expected_emit(
        &mut self,
        state: &mut PathState,
        checks: ExpectedEmitChecks,
        emitter: Option<SymWord>,
        remaining: u64,
    ) -> CheatcodeOutcome {
        state.expected_emit =
            Some(ExpectedEmit { checks, emitter, remaining: remaining.max(1), template: None });
        CheatcodeOutcome::Continue(Vec::new())
    }

    #[expect(clippy::too_many_arguments)]
    /// Applies the `set_expected_call` symbolic executor helper.
    fn set_expected_call(
        &mut self,
        state: &mut PathState,
        callee: SymWord,
        value: Option<U256>,
        gas: Option<u64>,
        min_gas: Option<u64>,
        data: Vec<SymWord>,
        count: Option<u64>,
    ) -> CheatcodeOutcome {
        let (gas, min_gas) = adjust_expected_call_gas_for_value(value, gas, min_gas);
        state.expected_calls.push(ExpectedCall {
            callee,
            value,
            gas,
            min_gas,
            data,
            expected: count.unwrap_or(1).max(1),
            observed: 0,
            exact: count.is_some(),
        });
        CheatcodeOutcome::Continue(Vec::new())
    }

    /// Applies the `set_expected_create` symbolic executor helper.
    fn set_expected_create(
        &mut self,
        state: &mut PathState,
        bytecode: Vec<u8>,
        deployer: SymWord,
        kind: CreateKind,
    ) -> CheatcodeOutcome {
        state.expected_creates.push(ExpectedCreate { bytecode, deployer, kind });
        CheatcodeOutcome::Continue(Vec::new())
    }

    /// Applies the `observe_expected_create` symbolic executor helper.
    fn observe_expected_create(
        &mut self,
        state: &mut PathState,
        deployer: Address,
        kind: CreateKind,
        runtime: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        if state.expected_creates.is_empty() {
            return Ok(());
        }
        let bytecode = runtime.read_concrete("symbolic expected create bytecode")?;
        let mut mismatch_constraints = None;
        for idx in 0..state.expected_creates.len() {
            let expected = state.expected_creates[idx].clone();
            if expected.kind != kind || expected.bytecode != bytecode {
                continue;
            }

            let condition = address_match_condition(&expected.deployer, deployer);
            let (match_constraints, match_sat) =
                self.constraints_with_condition(state, condition.clone())?;
            let (candidate_mismatch_constraints, mismatch_sat) =
                self.constraints_with_condition(state, condition.not())?;

            if match_sat && !mismatch_sat {
                state.constraints = match_constraints;
                state.expected_creates.swap_remove(idx);
                return Ok(());
            }

            if mismatch_sat {
                mismatch_constraints.get_or_insert(candidate_mismatch_constraints);
            }
        }

        if let Some(constraints) = mismatch_constraints {
            state.constraints = constraints;
        }
        Ok(())
    }

    /// Implements the `branch_accesses_cheatcode_if_needed` symbolic executor helper.
    fn branch_accesses_cheatcode_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        selector: [u8; 4],
        in_offset: usize,
        out_offset: SymWord,
        out_size: &BoundedCopySize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        if selector != selector!("accesses(address)") {
            return Ok(None);
        }

        let Some(record) = state.access_record.clone() else {
            return Ok(None);
        };
        let target = read_abi_word_arg(&state.memory, in_offset + 4, 0)?;
        if matches!(target, SymWord::Concrete(_)) {
            return Ok(None);
        }

        let addresses =
            record.reads.keys().chain(record.writes.keys()).copied().collect::<BTreeSet<_>>();
        if addresses.is_empty() {
            return Ok(None);
        }

        let mut branches = VecDeque::new();
        let mut matched_conditions = Vec::new();
        for address in addresses {
            let condition = address_match_condition(&target, address);
            matched_conditions.push(condition.clone());
            if let Some(constraints) = self.constraints_for_condition(state, condition)? {
                let mut branch = state.clone();
                branch.constraints = constraints;
                complete_cheatcode_call(
                    &mut branch,
                    out_offset.clone(),
                    out_size,
                    accesses_return_data(Some(&record), address),
                )?;
                branches.push_back(branch);
            }
        }

        let unmatched_condition =
            BoolExpr::and(matched_conditions.into_iter().map(BoolExpr::not).collect());
        if let Some(constraints) = self.constraints_for_condition(state, unmatched_condition)? {
            let mut branch = state.clone();
            branch.constraints = constraints;
            complete_cheatcode_call(
                &mut branch,
                out_offset,
                out_size,
                accesses_return_data(Some(&record), Address::ZERO),
            )?;
            branches.push_back(branch);
        }

        let Some(first_branch) = branches.pop_front() else {
            return Ok(Some(StepOutcome::AssumeRejected));
        };
        *state = first_branch;
        worklist.extend(branches);
        Ok(Some(StepOutcome::Continue))
    }

    /// Implements the `accesses_return_data_for_target` symbolic executor helper.
    fn accesses_return_data_for_target(
        &mut self,
        state: &mut PathState,
        target: SymWord,
    ) -> Result<SymReturnData, SymbolicError> {
        let Some(record) = state.access_record.clone() else {
            return Ok(accesses_return_data(None, Address::ZERO));
        };

        if let SymWord::Concrete(target) = target {
            return Ok(accesses_return_data(Some(&record), word_to_address(target)));
        }

        let addresses =
            record.reads.keys().chain(record.writes.keys()).copied().collect::<BTreeSet<_>>();
        if addresses.is_empty() {
            return Ok(accesses_return_data(Some(&record), Address::ZERO));
        }

        for address in addresses {
            let condition = address_match_condition(&target, address);
            let (match_constraints, match_sat) =
                self.constraints_with_condition(state, condition.clone())?;
            let (_, mismatch_sat) = self.constraints_with_condition(state, condition.not())?;

            match (match_sat, mismatch_sat) {
                (true, false) => {
                    state.constraints = match_constraints;
                    return Ok(accesses_return_data(Some(&record), address));
                }
                (true, true) => {
                    return Err(SymbolicError::Unsupported("symbolic vm.accesses address"));
                }
                (false, _) => {}
            }
        }

        Ok(accesses_return_data(Some(&record), Address::ZERO))
    }

    /// Implements the `add_call_mock` symbolic executor helper.
    fn add_call_mock(
        &mut self,
        state: &mut PathState,
        callee: SymWord,
        value: Option<U256>,
        data: Vec<SymWord>,
        returns: Vec<SymReturnData>,
        reverts: bool,
    ) -> CheatcodeOutcome {
        state.call_mocks.push(CallMock { callee, value, data, returns, reverts, calls: 0 });
        CheatcodeOutcome::Continue(Vec::new())
    }

    /// Applies the `set_function_mock` symbolic executor helper.
    fn set_function_mock(
        &mut self,
        state: &mut PathState,
        callee: SymWord,
        target: Address,
        data: Vec<SymWord>,
    ) -> CheatcodeOutcome {
        if let Some(mock) =
            state.function_mocks.iter_mut().find(|mock| mock.callee == callee && mock.data == data)
        {
            mock.target = target;
        } else {
            state.function_mocks.push(FunctionMock { callee, target, data });
        }
        CheatcodeOutcome::Continue(Vec::new())
    }

    /// Runs the `handle_foundry_cheatcode` symbolic executor helper.
    fn handle_foundry_cheatcode<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        selector: [u8; 4],
        in_offset: usize,
        in_size: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let args_offset = in_offset + 4;
        if selector == selector!("assume(bool)") {
            return self.handle_assume(state, in_offset + 4);
        }
        if selector == selector!("assumeNoRevert()") {
            if state.assume_no_revert_next_call.is_some() {
                return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
            }
            state.assume_no_revert_next_call = Some(AssumeNoRevert::Any);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("assumeNoRevert((address,bool,bytes))") {
            if state.assume_no_revert_next_call.is_some() {
                return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
            }
            let mut values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Tuple(vec![
                    DynSolType::Address,
                    DynSolType::Bool,
                    DynSolType::Bytes,
                ])],
            )?;
            let value = values
                .pop()
                .ok_or(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"))?;
            state.assume_no_revert_next_call =
                Some(AssumeNoRevert::Filtered(vec![dyn_potential_revert(&value)?]));
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("assumeNoRevert((address,bool,bytes)[])") {
            if state.assume_no_revert_next_call.is_some() {
                return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
            }
            let mut values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::Tuple(vec![
                    DynSolType::Address,
                    DynSolType::Bool,
                    DynSolType::Bytes,
                ])))],
            )?;
            let value = values
                .pop()
                .ok_or(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"))?;
            state.assume_no_revert_next_call =
                Some(AssumeNoRevert::Filtered(dyn_potential_reverts(&value)?));
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("skip(bool)") || selector == selector!("skip(bool,string)") {
            return self.handle_skip(state, in_offset + 4);
        }
        if selector == selector!("recordLogs()") {
            state.recorded_logs = Some(Vec::new());
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("record()") {
            state.access_record = Some(AccessRecord::default());
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("stopRecord()") {
            state.access_record = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("accesses(address)") {
            let target = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(CheatcodeOutcome::ContinueData(
                self.accesses_return_data_for_target(state, target)?,
            ));
        }
        if selector == selector!("getRecordedLogs()") {
            let logs = state.recorded_logs.replace(Vec::new()).unwrap_or_default();
            return Ok(CheatcodeOutcome::ContinueData(recorded_logs_return_data(logs)));
        }
        if selector == selector!("getRecordedLogsJson()") {
            let logs = state.recorded_logs.replace(Vec::new()).unwrap_or_default();
            return Ok(CheatcodeOutcome::ContinueData(recorded_logs_json_return_data(logs)?));
        }
        if selector == selector!("expectRevert()") {
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, None, 1));
        }
        if selector == selector!("expectRevert(bytes4)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                None,
                1,
            ));
        }
        if selector == selector!("expectRevert(bytes)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                0,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectRevert",
            )?;
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Exact(data), None, 1));
        }
        if selector == selector!("expectRevert(address)") {
            let reverter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, Some(reverter), 1));
        }
        if selector == selector!("expectRevert(bytes4,address)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                Some(reverter),
                1,
            ));
        }
        if selector == selector!("expectRevert(bytes,address)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                0,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectRevert",
            )?;
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Exact(data),
                Some(reverter),
                1,
            ));
        }
        if selector == selector!("expectRevert(uint64)") {
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 0, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, None, count));
        }
        if selector == selector!("expectRevert(bytes4,uint64)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                None,
                count,
            ));
        }
        if selector == selector!("expectRevert(bytes,uint64)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                0,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectRevert",
            )?;
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Exact(data),
                None,
                count,
            ));
        }
        if selector == selector!("expectRevert(address,uint64)") {
            let reverter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Any,
                Some(reverter),
                count,
            ));
        }
        if selector == selector!("expectRevert(bytes4,address,uint64)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                Some(reverter),
                count,
            ));
        }
        if selector == selector!("expectRevert(bytes,address,uint64)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                0,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectRevert",
            )?;
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Exact(data),
                Some(reverter),
                count,
            ));
        }
        if selector == selector!("expectPartialRevert(bytes4)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                None,
                1,
            ));
        }
        if selector == selector!("expectPartialRevert(bytes4,address)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                Some(reverter),
                1,
            ));
        }
        if selector == selector!("expectEmit()") {
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                None,
                1,
            ));
        }
        if selector == selector!("expectEmit(address)") {
            let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                Some(emitter),
                1,
            ));
        }
        if selector == selector!("expectEmit(uint64)") {
            let count = read_abi_u64_arg(&state.memory, args_offset, 0, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                None,
                count,
            ));
        }
        if selector == selector!("expectEmit(address,uint64)") {
            let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                Some(emitter),
                count,
            ));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            return Ok(self.set_expected_emit(state, checks, None, 1));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool,address)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            let emitter = read_abi_word_arg(&state.memory, args_offset, 4)?;
            return Ok(self.set_expected_emit(state, checks, Some(emitter), 1));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool,uint64)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(state, checks, None, count));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool,address,uint64)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            let emitter = read_abi_word_arg(&state.memory, args_offset, 4)?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 5, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(state, checks, Some(emitter), count));
        }
        if selector == selector!("expectEmitAnonymous()") {
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_anonymous(),
                None,
                1,
            ));
        }
        if selector == selector!("expectEmitAnonymous(address)") {
            let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_anonymous(),
                Some(emitter),
                1,
            ));
        }
        if selector == selector!("expectEmitAnonymous(bool,bool,bool,bool,bool)") {
            let checks = ExpectedEmitChecks::from_anonymous_args(&state.memory, args_offset)?;
            return Ok(self.set_expected_emit(state, checks, None, 1));
        }
        if selector == selector!("expectEmitAnonymous(bool,bool,bool,bool,bool,address)") {
            let checks = ExpectedEmitChecks::from_anonymous_args(&state.memory, args_offset)?;
            let emitter = read_abi_word_arg(&state.memory, args_offset, 5)?;
            return Ok(self.set_expected_emit(state, checks, Some(emitter), 1));
        }
        if selector == selector!("expectCall(address,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            return Ok(self.set_expected_call(state, callee, None, None, None, data, None));
        }
        if selector == selector!("expectCall(address,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(state, callee, None, None, None, data, Some(count)));
        }
        if selector == selector!("expectCall(address,uint256,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            return Ok(self.set_expected_call(state, callee, Some(value), None, None, data, None));
        }
        if selector == selector!("expectCall(address,uint256,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 3, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                None,
                None,
                data,
                Some(count),
            ));
        }
        if selector == selector!("expectCall(address,uint256,uint64,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let gas = read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                Some(gas),
                None,
                data,
                None,
            ));
        }
        if selector == selector!("expectCall(address,uint256,uint64,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let gas = read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                Some(gas),
                None,
                data,
                Some(count),
            ));
        }
        if selector == selector!("expectCallMinGas(address,uint256,uint64,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let min_gas =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                None,
                Some(min_gas),
                data,
                None,
            ));
        }
        if selector == selector!("expectCallMinGas(address,uint256,uint64,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let min_gas =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                None,
                Some(min_gas),
                data,
                Some(count),
            ));
        }
        if selector == selector!("expectCreate(bytes,address)")
            || selector == selector!("expectCreate2(bytes,address)")
        {
            let bytecode = read_abi_dynamic_bytes_arg(
                &state.memory,
                args_offset,
                0,
                "symbolic vm.expectCreate bytecode",
            )?;
            let deployer = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let kind = if selector == selector!("expectCreate(bytes,address)") {
                CreateKind::Create
            } else {
                CreateKind::Create2
            };
            return Ok(self.set_expected_create(state, bytecode, deployer, kind));
        }
        if selector == selector!("clearMockedCalls()") {
            state.call_mocks.clear();
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("mockCall(address,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], false));
        }
        if selector == selector!("mockCall(address,uint256,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.mockCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], false));
        }
        if selector == selector!("mockCall(address,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 1);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], false));
        }
        if selector == selector!("mockCall(address,uint256,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.mockCall")?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 2);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], false));
        }
        if selector == selector!("mockCalls(address,bytes,bytes[])")
            || selector == selector!("mockCalls(address,uint256,bytes,bytes[])")
        {
            let has_value = selector == selector!("mockCalls(address,uint256,bytes,bytes[])");
            let (value, data_idx, ret_idx) = if has_value {
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.mockCalls",
                )?;
                (Some(value), 2, 3)
            } else {
                (None, 1, 2)
            };
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                data_idx,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCalls data",
            )?;
            let returns = read_abi_symbolic_dynamic_bytes_array_arg(
                state,
                args_offset,
                ret_idx,
                self.config.max_dynamic_length as usize,
                self.config.max_calldata_bytes as usize,
            )?;
            return Ok(self.add_call_mock(state, callee, value, data, returns, false));
        }
        if selector == selector!("mockCallRevert(address,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], true));
        }
        if selector == selector!("mockCallRevert(address,uint256,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.mockCallRevert",
            )?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], true));
        }
        if selector == selector!("mockCallRevert(address,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 1);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], true));
        }
        if selector == selector!("mockCallRevert(address,uint256,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.mockCallRevert",
            )?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 2);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], true));
        }
        if selector == selector!("mockFunction(address,address,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let target =
                read_abi_address_arg(&state.memory, args_offset, 1, "symbolic vm.mockFunction")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockFunction",
            )?;
            return Ok(self.set_function_mock(state, callee, target, data));
        }
        if selector == selector!("prank(address)") {
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prank(address,address)") {
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prank(address,bool)") {
            let _delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 1, "symbolic vm.prank")?;
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prank(address,address,bool)") {
            let _delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 2, "symbolic vm.prank")?;
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address)") {
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address,address)") {
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address,bool)") {
            let _delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 1, "symbolic vm.startPrank")?;
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address,address,bool)") {
            let _delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 2, "symbolic vm.startPrank")?;
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("stopPrank()") {
            state.prank = SymbolicPrank::default();
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("readCallers()") {
            return Ok(CheatcodeOutcome::Continue(state.read_callers_words()));
        }
        if selector == selector!("addr(uint256)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.addr")?;
            let address = private_key_address(private_key)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(address))]));
        }
        if selector == selector!("sign(uint256,bytes32)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.sign")?;
            let digest = read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.sign")?;
            return Ok(CheatcodeOutcome::Continue(sign_hash_words(private_key, digest)?));
        }
        if selector == selector!("signCompact(uint256,bytes32)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.signCompact")?;
            let digest =
                read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.signCompact")?;
            return Ok(CheatcodeOutcome::Continue(sign_compact_hash_words(private_key, digest)?));
        }
        if selector == selector!("deriveKey(string,uint32)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let private_key =
                derive_private_key::<English>(&mnemonic, DEFAULT_DERIVATION_PATH_PREFIX, index)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("deriveKey(string,string,uint32)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let path = read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
            let private_key = derive_private_key::<English>(&mnemonic, &path, index)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("deriveKey(string,uint32,string)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let language =
                read_abi_string_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
            let private_key = derive_private_key_with_language(
                &mnemonic,
                DEFAULT_DERIVATION_PATH_PREFIX,
                index,
                &language,
            )?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("deriveKey(string,string,uint32,string)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let path = read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
            let language =
                read_abi_string_arg(&state.memory, args_offset, 3, "symbolic vm.deriveKey")?;
            let private_key = derive_private_key_with_language(&mnemonic, &path, index, &language)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("rememberKey(uint256)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.rememberKey")?;
            let address = private_key_address(private_key)?;
            state.wallets.insert(address);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(address))]));
        }
        if selector == selector!("rememberKeys(string,string,uint32)")
            || selector == selector!("rememberKeys(string,string,string,uint32)")
        {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.rememberKeys")?;
            let path =
                read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.rememberKeys")?;
            let (language, count_index) =
                if selector == selector!("rememberKeys(string,string,string,uint32)") {
                    (
                        Some(read_abi_string_arg(
                            &state.memory,
                            args_offset,
                            2,
                            "symbolic vm.rememberKeys",
                        )?),
                        3,
                    )
                } else {
                    (None, 2)
                };
            let count = read_abi_u32_arg(
                &state.memory,
                args_offset,
                count_index,
                "symbolic vm.rememberKeys",
            )?;
            if count > MAX_REMEMBER_KEYS {
                return Err(SymbolicError::Unsupported("symbolic vm.rememberKeys count"));
            }
            let mut addresses = Vec::with_capacity(count as usize);
            for index in 0..count {
                let private_key = if let Some(language) = &language {
                    derive_private_key_with_language(&mnemonic, &path, index, language)?
                } else {
                    derive_private_key::<English>(&mnemonic, &path, index)?
                };
                let address = private_key_address(private_key)?;
                state.wallets.insert(address);
                addresses.push(DynSolValue::Address(address));
            }
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(
                DynSolValue::Array(addresses),
            )));
        }
        if selector == selector!("getWallets()") {
            let wallets = DynSolValue::Array(
                state.wallets.iter().copied().map(DynSolValue::Address).collect(),
            );
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(wallets)));
        }
        if selector == selector!("store(address,bytes32,bytes32)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let slot = state.memory.load_word(in_offset + 36)?;
            let value = state.memory.load_word(in_offset + 68)?;
            if target == CHEATCODE_ADDRESS
                && slot == SymWord::Concrete(failed_slot())
                && value == SymWord::Concrete(U256::from(1))
            {
                return Ok(CheatcodeOutcome::Failure);
            }
            state.world.sstore(target, slot, value);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("load(address,bytes32)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let slot = state.memory.load_word(in_offset + 36)?;
            let value = state.world.sload(executor, target, slot)?;
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("getNonce(address)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let nonce = state.world.nonce(executor, target)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(nonce))]));
        }
        if selector == selector!("computeCreateAddress(address,uint256)") {
            let deployer = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let nonce = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let address = compute_create_address_word(state, deployer, nonce)?;
            return Ok(CheatcodeOutcome::Continue(vec![address]));
        }
        if selector == selector!("computeCreate2Address(bytes32,bytes32,address)") {
            let salt = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let init_code_hash = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let deployer = read_abi_word_arg(&state.memory, args_offset, 2)?;
            let address = compute_create2_address_word(state, deployer, salt, init_code_hash)?;
            return Ok(CheatcodeOutcome::Continue(vec![address]));
        }
        if selector == selector!("computeCreate2Address(bytes32,bytes32)") {
            let salt = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let init_code_hash = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let address = compute_create2_address_word(
                state,
                SymWord::Concrete(address_word(DEFAULT_CREATE2_DEPLOYER)),
                salt,
                init_code_hash,
            )?;
            return Ok(CheatcodeOutcome::Continue(vec![address]));
        }
        if selector == selector!("etch(address,bytes)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let code = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_dynamic_length as usize,
                "symbolic vm.etch",
            )?;
            state.world.install_code(target, SymCode { bytes: code });
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getCode(string)")
            || selector == selector!("getDeployedCode(string)")
        {
            let artifact =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.getCode")?;
            let code = artifact_code(&artifact, selector == selector!("getDeployedCode(string)"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(code)));
        }
        if selector == selector!("deal(address,uint256)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let value =
                read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.deal value")?;
            state.world.set_balance(target, value);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("setNonce(address,uint64)")
            || selector == selector!("setNonceUnsafe(address,uint64)")
        {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let nonce =
                read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.setNonce")?;
            if nonce > U256::from(u64::MAX) {
                return Err(SymbolicError::Unsupported("symbolic vm.setNonce nonce"));
            }
            let nonce = nonce.to::<u64>();
            if selector == selector!("setNonce(address,uint64)")
                && nonce < state.world.nonce(executor, target)?
            {
                return Ok(CheatcodeOutcome::Failure);
            }
            state.world.set_nonce(target, nonce);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("resetNonce(address)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let nonce = if state.world.extcode(executor, target)?.is_empty() { 0 } else { 1 };
            state.world.set_nonce(target, nonce);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("allowCheatcodes(address)") {
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address)") {
            let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            state.persistent_accounts.insert(account);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address,address)") {
            let account0 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let account1 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 1)?;
            state.persistent_accounts.insert(account0);
            state.persistent_accounts.insert(account1);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address,address,address)") {
            let account0 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let account1 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 1)?;
            let account2 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 2)?;
            state.persistent_accounts.insert(account0);
            state.persistent_accounts.insert(account1);
            state.persistent_accounts.insert(account2);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address[])") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::Address))],
            )?;
            for account in dyn_address_array(&values[0])? {
                state.persistent_accounts.insert(account);
            }
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("revokePersistent(address)") {
            let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            state.persistent_accounts.remove(&account);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("revokePersistent(address[])") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::Address))],
            )?;
            for account in dyn_address_array(&values[0])? {
                state.persistent_accounts.remove(&account);
            }
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("isPersistent(address)") {
            let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                state.persistent_accounts.contains(&account),
            ))]));
        }
        if selector == selector!("activeFork()") {
            let id = executor.backend().active_fork_id().ok_or(SymbolicError::Unsupported(
                "symbolic vm.activeFork requires an active forked executor",
            ))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(id)]));
        }
        if selector == selector!("selectFork(uint256)") {
            let id =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.selectFork id")?;
            if executor.backend().is_active_fork(id) {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            return Err(SymbolicError::Unsupported(
                "symbolic vm.selectFork can only select the already active fork",
            ));
        }
        if selector == selector!("rollFork(uint256)") {
            let block_number = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.rollFork block number",
            )?;
            let current =
                state.block.number.clone().into_concrete("symbolic vm.rollFork current block")?;
            if block_number == current {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            return Err(SymbolicError::Unsupported(
                "symbolic vm.rollFork cannot change the active fork block during symbolic execution",
            ));
        }
        if selector == selector!("rollFork(uint256,uint256)") {
            let id =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.rollFork id")?;
            let block_number = read_abi_constrained_word_arg(
                state,
                args_offset,
                1,
                "symbolic vm.rollFork block number",
            )?;
            let current =
                state.block.number.clone().into_concrete("symbolic vm.rollFork current block")?;
            if executor.backend().is_active_fork(id) && block_number == current {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            return Err(SymbolicError::Unsupported(
                "symbolic vm.rollFork cannot change the active fork block during symbolic execution",
            ));
        }
        if selector == selector!("createFork(string)")
            || selector == selector!("createFork(string,uint256)")
            || selector == selector!("createFork(string,bytes32)")
            || selector == selector!("createSelectFork(string)")
            || selector == selector!("createSelectFork(string,uint256)")
            || selector == selector!("createSelectFork(string,bytes32)")
            || selector == selector!("rollFork(bytes32)")
            || selector == selector!("rollFork(uint256,bytes32)")
        {
            return Err(SymbolicError::Unsupported(
                "symbolic fork creation and fork block mutation must happen before symbolic execution",
            ));
        }
        if selector == selector!("snapshot()") || selector == selector!("snapshotState()") {
            let id = state.world.snapshot_state();
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(id)]));
        }
        if selector == selector!("revertTo(uint256)")
            || selector == selector!("revertToState(uint256)")
            || selector == selector!("revertToAndDelete(uint256)")
            || selector == selector!("revertToStateAndDelete(uint256)")
        {
            let id = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.revertToState snapshot",
            )?;
            let success = state.world.restore_snapshot(id);
            if success
                && (selector == selector!("revertToAndDelete(uint256)")
                    || selector == selector!("revertToStateAndDelete(uint256)"))
            {
                state.world.delete_snapshot(id);
            }
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(success))]));
        }
        if selector == selector!("deleteSnapshot(uint256)")
            || selector == selector!("deleteStateSnapshot(uint256)")
        {
            let id = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.deleteStateSnapshot snapshot",
            )?;
            let success = state.world.delete_snapshot(id);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(success))]));
        }
        if selector == selector!("deleteSnapshots()")
            || selector == selector!("deleteStateSnapshots()")
        {
            state.world.delete_snapshots();
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("warp(uint256)") {
            state.block.timestamp = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("roll(uint256)") {
            state.block.number = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("setBlockhash(uint256,bytes32)") {
            let block_number = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.setBlockhash block number",
            )?;
            let block_hash = state.memory.load_word(in_offset + 36)?;
            state.block.set_block_hash(block_number, block_hash)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prevrandao(bytes32)")
            || selector == selector!("prevrandao(uint256)")
        {
            state.block.difficulty = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("blobhashes(bytes32[])") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::FixedBytes(32)))],
            )?;
            state.block.set_blob_hashes(dyn_bytes32_array(&values[0])?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlobhashes()") {
            let value = DynSolValue::Array(
                state
                    .block
                    .blob_hashes
                    .iter()
                    .copied()
                    .map(|hash| DynSolValue::FixedBytes(hash, 32))
                    .collect(),
            );
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
        }
        if selector == selector!("fee(uint256)") {
            state.block.basefee = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("blobBaseFee(uint256)") {
            state.block.blob_basefee = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlobBaseFee()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.blob_basefee.clone()]));
        }
        if selector == selector!("chainId(uint256)") {
            state.block.chain_id = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getChainId()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.chain_id.clone()]));
        }
        if selector == selector!("difficulty(uint256)") {
            state.block.difficulty = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("coinbase(address)") {
            let coinbase = read_abi_constrained_address_arg(
                state,
                args_offset,
                0,
                "symbolic vm.coinbase value",
            )?;
            state.block.coinbase = coinbase;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlockNumber()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.number.clone()]));
        }
        if selector == selector!("txGasPrice(uint256)") {
            state.gas_price = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlockTimestamp()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.timestamp.clone()]));
        }
        if selector == selector!("label(address,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Address, DynSolType::String],
            )?;
            let account = dyn_address(&values[0])?;
            let label = dyn_string(&values[1])?;
            state.labels.insert(account, label);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getLabel(address)") {
            let account =
                read_abi_address_arg(&state.memory, args_offset, 0, "symbolic vm.getLabel")?;
            let label = state
                .labels
                .get(&account)
                .cloned()
                .unwrap_or_else(|| format!("unlabeled:{account}"));
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(label.bytes())));
        }
        if selector == selector!("pauseGasMetering()")
            || selector == selector!("resumeGasMetering()")
            || selector == selector!("resetGasMetering()")
            || selector == selector!("breakpoint(string)")
            || selector == selector!("breakpoint(string,bool)")
            || selector == selector!("expectSafeMemory(uint64,uint64)")
            || selector == selector!("expectSafeMemoryCall(uint64,uint64)")
            || selector == selector!("stopExpectSafeMemory()")
            || selector == selector!("snapshotValue(string,uint256)")
            || selector == selector!("snapshotValue(string,string,uint256)")
            || selector == selector!("startSnapshotGas(string)")
            || selector == selector!("startSnapshotGas(string,string)")
            || selector == selector!("setEvmVersion(string)")
            || selector == selector!("sleep(uint256)")
            || selector == selector!("cool(address)")
            || selector == selector!("accessList((address,bytes32[])[])")
            || selector == selector!("warmSlot(address,bytes32)")
            || selector == selector!("coolSlot(address,bytes32)")
            || selector == selector!("noAccessList()")
        {
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getEvmVersion()") {
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return("cancun".bytes())));
        }
        if selector == selector!("getFoundryVersion()") {
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                env!("CARGO_PKG_VERSION").bytes(),
            )));
        }
        if selector == selector!("lastCallGas()") {
            return Ok(CheatcodeOutcome::Continue(vec![
                SymWord::zero(),
                SymWord::zero(),
                SymWord::zero(),
                SymWord::zero(),
                SymWord::zero(),
            ]));
        }
        if selector == selector!("snapshotGasLastCall(string)")
            || selector == selector!("snapshotGasLastCall(string,string)")
            || selector == selector!("stopSnapshotGas()")
            || selector == selector!("stopSnapshotGas(string)")
            || selector == selector!("stopSnapshotGas(string,string)")
        {
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::zero()]));
        }
        if selector == selector!("projectRoot()") {
            let root = std::env::current_dir()
                .map_err(|_| SymbolicError::Unsupported("symbolic vm.projectRoot"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                root.display().to_string().bytes(),
            )));
        }
        if selector == selector!("unixTime()") {
            let milliseconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| SymbolicError::Unsupported("symbolic vm.unixTime"))?
                .as_millis();
            let value = U256::try_from(milliseconds)
                .map_err(|_| SymbolicError::Unsupported("symbolic vm.unixTime"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(value)]));
        }
        if selector == selector!("isContext(uint8)") {
            let context =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.isContext")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                context == U256::ZERO || context == U256::from(1),
            ))]));
        }
        if selector == selector!("toString(address)") {
            let address =
                read_abi_address_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                format!("{address:?}").bytes(),
            )));
        }
        if selector == selector!("toString(bytes)") {
            let bytes =
                read_abi_dynamic_bytes_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                format!("0x{}", hex::encode(bytes)).bytes(),
            )));
        }
        if selector == selector!("toString(bytes32)") {
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                format!("0x{}", hex::encode(value.to_be_bytes::<32>())).bytes(),
            )));
        }
        if selector == selector!("toString(bool)") {
            let value = read_abi_bool_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                if value { "true" } else { "false" }.bytes(),
            )));
        }
        if selector == selector!("toString(uint256)") {
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                value.to_string().bytes(),
            )));
        }
        if selector == selector!("toString(int256)") {
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                I256::from_raw(value).to_string().bytes(),
            )));
        }
        if selector == selector!("parseBytes(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBytes")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(parse_env_bytes(
                &value,
            )?)));
        }
        if selector == selector!("parseAddress(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseAddress")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(
                parse_env_address(&value)?,
            ))]));
        }
        if selector == selector!("parseUint(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseUint")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_uint(
                &value,
            )?)]));
        }
        if selector == selector!("parseInt(string)") {
            let value = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseInt")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_int(&value)?)]));
        }
        if selector == selector!("parseBytes32(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBytes32")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_bytes32(
                &value,
            )?)]));
        }
        if selector == selector!("parseBool(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBool")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                parse_env_bool(&value)?,
            ))]));
        }
        if selector == selector!("toLowercase(string)")
            || selector == selector!("toUppercase(string)")
            || selector == selector!("trim(string)")
        {
            let value = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.string")?;
            let output = if selector == selector!("toLowercase(string)") {
                value.to_lowercase()
            } else if selector == selector!("toUppercase(string)") {
                value.to_uppercase()
            } else {
                value.trim().to_string()
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(output.bytes())));
        }
        if selector == selector!("replace(string,string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String, DynSolType::String],
            )?;
            let output =
                dyn_string(&values[0])?.replace(&dyn_string(&values[1])?, &dyn_string(&values[2])?);
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(output.bytes())));
        }
        if selector == selector!("split(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let input = dyn_string(&values[0])?;
            let delimiter = dyn_string(&values[1])?;
            let parts = if delimiter.is_empty() {
                input.chars().map(|ch| DynSolValue::String(ch.to_string())).collect()
            } else {
                input.split(&delimiter).map(|part| DynSolValue::String(part.to_string())).collect()
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(
                DynSolValue::Array(parts),
            )));
        }
        if selector == selector!("indexOf(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let input = dyn_string(&values[0])?;
            let needle = dyn_string(&values[1])?;
            let index = input.find(&needle).map(U256::from).unwrap_or(U256::MAX);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(index)]));
        }
        if selector == selector!("contains(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let contains = dyn_string(&values[0])?.contains(&dyn_string(&values[1])?);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(contains))]));
        }
        if selector == selector!("toBase64(bytes)")
            || selector == selector!("toBase64(string)")
            || selector == selector!("toBase64URL(bytes)")
            || selector == selector!("toBase64URL(string)")
        {
            let data =
                read_abi_dynamic_bytes_arg(&state.memory, args_offset, 0, "symbolic vm.toBase64")?;
            let encoded = if selector == selector!("toBase64URL(bytes)")
                || selector == selector!("toBase64URL(string)")
            {
                BASE64_URL_SAFE.encode(data)
            } else {
                BASE64_STANDARD.encode(data)
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(encoded.bytes())));
        }
        if selector == selector!("bound(uint256,uint256,uint256)") {
            return self.handle_bound_uint(state, args_offset);
        }
        if selector == selector!("bound(int256,int256,int256)") {
            return self.handle_bound_int(state, args_offset);
        }
        if selector == selector!("envExists(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envExists")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                std::env::var_os(name).is_some(),
            ))]));
        }
        if selector == selector!("envBool(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBool")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                parse_env_bool(&value)?,
            ))]));
        }
        if selector == selector!("envUint(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envUint")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_uint(
                &value,
            )?)]));
        }
        if selector == selector!("envInt(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envInt")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_int(&value)?)]));
        }
        if selector == selector!("envAddress(string)") {
            let name =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envAddress")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            let address = parse_env_address(&value)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(address))]));
        }
        if selector == selector!("envBytes32(string)") {
            let name =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBytes32")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_bytes32(
                &value,
            )?)]));
        }
        if selector == selector!("envString(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envString")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(value.bytes())));
        }
        if selector == selector!("envBytes(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBytes")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(parse_env_bytes(
                &value,
            )?)));
        }
        if selector == selector!("envBool(string,string)")
            || selector == selector!("envUint(string,string)")
            || selector == selector!("envInt(string,string)")
            || selector == selector!("envAddress(string,string)")
            || selector == selector!("envBytes32(string,string)")
            || selector == selector!("envString(string,string)")
            || selector == selector!("envBytes(string,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let name = dyn_string(&values[0])?;
            let delimiter = dyn_string(&values[1])?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            let value = if selector == selector!("envBool(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_bool_value)?
            } else if selector == selector!("envUint(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_uint_value)?
            } else if selector == selector!("envInt(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_int_value)?
            } else if selector == selector!("envAddress(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_address_value)?
            } else if selector == selector!("envBytes32(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_bytes32_value)?
            } else if selector == selector!("envString(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_string_value)?
            } else {
                parse_env_array(&value, &delimiter, parse_env_bytes_value)?
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
        }
        if selector == selector!("envOr(string,bool)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envOr")?;
            let value = match std::env::var(name) {
                Ok(value) => U256::from(parse_env_bool(&value)?),
                Err(_) => {
                    read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.envOr")?
                }
            };
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(value)]));
        }
        if selector == selector!("envOr(string,uint256)")
            || selector == selector!("envOr(string,int256)")
            || selector == selector!("envOr(string,address)")
            || selector == selector!("envOr(string,bytes32)")
        {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envOr")?;
            let default =
                read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.envOr")?;
            let value = match std::env::var(name) {
                Ok(value) if selector == selector!("envOr(string,uint256)") => {
                    parse_env_uint(&value)?
                }
                Ok(value) if selector == selector!("envOr(string,int256)") => {
                    parse_env_int(&value)?
                }
                Ok(value) if selector == selector!("envOr(string,address)") => {
                    address_word(parse_env_address(&value)?)
                }
                Ok(value) => parse_env_bytes32(&value)?,
                Err(_) => default,
            };
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(value)]));
        }
        if selector == selector!("envOr(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let name = dyn_string(&values[0])?;
            let value = std::env::var(name).unwrap_or(dyn_string(&values[1])?);
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(value.bytes())));
        }
        if selector == selector!("envOr(string,bytes)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::Bytes],
            )?;
            let name = dyn_string(&values[0])?;
            let value = match std::env::var(name) {
                Ok(value) => parse_env_bytes(&value)?,
                Err(_) => dyn_bytes(&values[1])?,
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(value)));
        }
        if selector == selector!("envOr(string,string,bool[])")
            || selector == selector!("envOr(string,string,uint256[])")
            || selector == selector!("envOr(string,string,int256[])")
            || selector == selector!("envOr(string,string,address[])")
            || selector == selector!("envOr(string,string,bytes32[])")
            || selector == selector!("envOr(string,string,string[])")
            || selector == selector!("envOr(string,string,bytes[])")
        {
            let element_ty = if selector == selector!("envOr(string,string,bool[])") {
                DynSolType::Bool
            } else if selector == selector!("envOr(string,string,uint256[])") {
                DynSolType::Uint(256)
            } else if selector == selector!("envOr(string,string,int256[])") {
                DynSolType::Int(256)
            } else if selector == selector!("envOr(string,string,address[])") {
                DynSolType::Address
            } else if selector == selector!("envOr(string,string,bytes32[])") {
                DynSolType::FixedBytes(32)
            } else if selector == selector!("envOr(string,string,string[])") {
                DynSolType::String
            } else {
                DynSolType::Bytes
            };
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![
                    DynSolType::String,
                    DynSolType::String,
                    DynSolType::Array(Box::new(element_ty)),
                ],
            )?;
            let name = dyn_string(&values[0])?;
            let delimiter = dyn_string(&values[1])?;
            let value = match std::env::var(name) {
                Ok(value) if selector == selector!("envOr(string,string,bool[])") => {
                    parse_env_array(&value, &delimiter, parse_env_bool_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,uint256[])") => {
                    parse_env_array(&value, &delimiter, parse_env_uint_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,int256[])") => {
                    parse_env_array(&value, &delimiter, parse_env_int_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,address[])") => {
                    parse_env_array(&value, &delimiter, parse_env_address_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,bytes32[])") => {
                    parse_env_array(&value, &delimiter, parse_env_bytes32_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,string[])") => {
                    parse_env_array(&value, &delimiter, parse_env_string_value)?
                }
                Ok(value) => parse_env_array(&value, &delimiter, parse_env_bytes_value)?,
                Err(_) => values[2].clone(),
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
        }
        if selector == selector!("ffi(string[])") {
            if !state.ffi_enabled {
                return Err(SymbolicError::Unsupported("symbolic ffi disabled"));
            }
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::String))],
            )?;
            let args = dyn_string_array(&values[0])?;
            if args.is_empty() || args[0].is_empty() {
                return Err(SymbolicError::Unsupported("symbolic ffi empty command"));
            }
            let output = Command::new(&args[0])
                .args(&args[1..])
                .output()
                .map_err(|_| SymbolicError::Unsupported("symbolic ffi command"))?;
            if !output.status.success() {
                return Err(SymbolicError::Unsupported("symbolic ffi command failed"));
            }
            let stdout = String::from_utf8(output.stdout)
                .map_err(|_| SymbolicError::Unsupported("symbolic ffi stdout"))?;
            let trimmed = stdout.trim();
            let bytes = hex::decode(trimmed).unwrap_or_else(|_| trimmed.as_bytes().to_vec());
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(bytes)));
        }
        if selector == selector!("assertTrue(bool)")
            || selector == selector!("assertTrue(bool,string)")
        {
            let condition = read_abi_word_arg(&state.memory, args_offset, 0)?.nonzero_bool();
            return self.handle_assertion(state, condition);
        }
        if selector == selector!("assertFalse(bool)")
            || selector == selector!("assertFalse(bool,string)")
        {
            let condition = read_abi_word_arg(&state.memory, args_offset, 0)?.into_zero_bool();
            return self.handle_assertion(state, condition);
        }
        if selector == selector!("assertEq(uint256,uint256)")
            || selector == selector!("assertEq(uint256,uint256,string)")
            || selector == selector!("assertEq(int256,int256)")
            || selector == selector!("assertEq(int256,int256,string)")
            || selector == selector!("assertEq(address,address)")
            || selector == selector!("assertEq(address,address,string)")
            || selector == selector!("assertEq(bytes32,bytes32)")
            || selector == selector!("assertEq(bytes32,bytes32,string)")
            || selector == selector!("assertEq(bool,bool)")
            || selector == selector!("assertEq(bool,bool,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(state, BoolExpr::eq(left.into_expr(), right.into_expr()));
        }
        if selector == selector!("assertEq(string,string)")
            || selector == selector!("assertEq(string,string,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertEq(string,string)") {
                    vec![DynSolType::String, DynSolType::String]
                } else {
                    vec![DynSolType::String, DynSolType::String, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_string(&values[0])? == dyn_string(&values[1])?),
            );
        }
        if selector == selector!("assertEq(bytes,bytes)")
            || selector == selector!("assertEq(bytes,bytes,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertEq(bytes,bytes)") {
                    vec![DynSolType::Bytes, DynSolType::Bytes]
                } else {
                    vec![DynSolType::Bytes, DynSolType::Bytes, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_bytes(&values[0])? == dyn_bytes(&values[1])?),
            );
        }
        if selector == selector!("assertEq(bool[],bool[])")
            || selector == selector!("assertEq(bool[],bool[],string)")
            || selector == selector!("assertEq(uint256[],uint256[])")
            || selector == selector!("assertEq(uint256[],uint256[],string)")
            || selector == selector!("assertEq(int256[],int256[])")
            || selector == selector!("assertEq(int256[],int256[],string)")
            || selector == selector!("assertEq(address[],address[])")
            || selector == selector!("assertEq(address[],address[],string)")
            || selector == selector!("assertEq(bytes32[],bytes32[])")
            || selector == selector!("assertEq(bytes32[],bytes32[],string)")
            || selector == selector!("assertEq(string[],string[])")
            || selector == selector!("assertEq(string[],string[],string)")
            || selector == selector!("assertEq(bytes[],bytes[])")
            || selector == selector!("assertEq(bytes[],bytes[],string)")
        {
            let element_ty = array_assertion_element_type(selector)?;
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector_has_string_reason(selector) {
                    vec![
                        DynSolType::Array(Box::new(element_ty.clone())),
                        DynSolType::Array(Box::new(element_ty)),
                        DynSolType::String,
                    ]
                } else {
                    vec![
                        DynSolType::Array(Box::new(element_ty.clone())),
                        DynSolType::Array(Box::new(element_ty)),
                    ]
                },
            )?;
            return self.handle_assertion(state, BoolExpr::Const(values[0] == values[1]));
        }
        if selector == selector!("assertEqDecimal(uint256,uint256,uint256)")
            || selector == selector!("assertEqDecimal(uint256,uint256,uint256,string)")
            || selector == selector!("assertEqDecimal(int256,int256,uint256)")
            || selector == selector!("assertEqDecimal(int256,int256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(state, BoolExpr::eq(left.into_expr(), right.into_expr()));
        }
        if selector == selector!("assertNotEq(uint256,uint256)")
            || selector == selector!("assertNotEq(uint256,uint256,string)")
            || selector == selector!("assertNotEq(int256,int256)")
            || selector == selector!("assertNotEq(int256,int256,string)")
            || selector == selector!("assertNotEq(address,address)")
            || selector == selector!("assertNotEq(address,address,string)")
            || selector == selector!("assertNotEq(bytes32,bytes32)")
            || selector == selector!("assertNotEq(bytes32,bytes32,string)")
            || selector == selector!("assertNotEq(bool,bool)")
            || selector == selector!("assertNotEq(bool,bool,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self
                .handle_assertion(state, BoolExpr::eq(left.into_expr(), right.into_expr()).not());
        }
        if selector == selector!("assertNotEq(string,string)")
            || selector == selector!("assertNotEq(string,string,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertNotEq(string,string)") {
                    vec![DynSolType::String, DynSolType::String]
                } else {
                    vec![DynSolType::String, DynSolType::String, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_string(&values[0])? != dyn_string(&values[1])?),
            );
        }
        if selector == selector!("assertNotEq(bytes,bytes)")
            || selector == selector!("assertNotEq(bytes,bytes,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertNotEq(bytes,bytes)") {
                    vec![DynSolType::Bytes, DynSolType::Bytes]
                } else {
                    vec![DynSolType::Bytes, DynSolType::Bytes, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_bytes(&values[0])? != dyn_bytes(&values[1])?),
            );
        }
        if selector == selector!("assertNotEq(bool[],bool[])")
            || selector == selector!("assertNotEq(bool[],bool[],string)")
            || selector == selector!("assertNotEq(uint256[],uint256[])")
            || selector == selector!("assertNotEq(uint256[],uint256[],string)")
            || selector == selector!("assertNotEq(int256[],int256[])")
            || selector == selector!("assertNotEq(int256[],int256[],string)")
            || selector == selector!("assertNotEq(address[],address[])")
            || selector == selector!("assertNotEq(address[],address[],string)")
            || selector == selector!("assertNotEq(bytes32[],bytes32[])")
            || selector == selector!("assertNotEq(bytes32[],bytes32[],string)")
            || selector == selector!("assertNotEq(string[],string[])")
            || selector == selector!("assertNotEq(string[],string[],string)")
            || selector == selector!("assertNotEq(bytes[],bytes[])")
            || selector == selector!("assertNotEq(bytes[],bytes[],string)")
        {
            let element_ty = array_assertion_element_type(selector)?;
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector_has_string_reason(selector) {
                    vec![
                        DynSolType::Array(Box::new(element_ty.clone())),
                        DynSolType::Array(Box::new(element_ty)),
                        DynSolType::String,
                    ]
                } else {
                    vec![
                        DynSolType::Array(Box::new(element_ty.clone())),
                        DynSolType::Array(Box::new(element_ty)),
                    ]
                },
            )?;
            return self.handle_assertion(state, BoolExpr::Const(values[0] != values[1]));
        }
        if selector == selector!("assertLt(uint256,uint256)")
            || selector == selector!("assertLt(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Ult, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertLe(uint256,uint256)")
            || selector == selector!("assertLe(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Ule, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertGt(uint256,uint256)")
            || selector == selector!("assertGt(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Ugt, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertGe(uint256,uint256)")
            || selector == selector!("assertGe(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Uge, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertLt(int256,int256)")
            || selector == selector!("assertLt(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Slt, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertGt(int256,int256)")
            || selector == selector!("assertGt(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Sgt, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertLe(int256,int256)")
            || selector == selector!("assertLe(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Sgt, left.into_expr(), right.into_expr()).not(),
            );
        }
        if selector == selector!("assertGe(int256,int256)")
            || selector == selector!("assertGe(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Slt, left.into_expr(), right.into_expr()).not(),
            );
        }
        if selector == selector!("randomUint()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_word("vmRandomUint")]));
        }
        if selector == selector!("randomUint(uint256)") {
            let bits =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic randomUint bits")?;
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_bounded_uint(bits)]));
        }
        if selector == selector!("randomUint(uint256,uint256)") {
            let min = state.memory.load_word(in_offset + 4)?;
            let max = state.memory.load_word(in_offset + 36)?;
            let value = state.fresh_word("vmRandomUintRange");
            state.constraints.push(BoolExpr::cmp(
                BoolExprOp::Uge,
                value.clone().into_expr(),
                min.into_expr(),
            ));
            state.constraints.push(BoolExpr::cmp(
                BoolExprOp::Ule,
                value.clone().into_expr(),
                max.into_expr(),
            ));
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("randomInt()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_word("vmRandomInt")]));
        }
        if selector == selector!("randomInt(uint256)") {
            let bits =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic randomInt bits")?;
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_bounded_uint(bits)]));
        }
        if selector == selector!("randomAddress()") {
            let value = state.fresh_bounded_uint(U256::from(160));
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("randomBool()") {
            let value = state.fresh_bounded_uint(U256::from(1));
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("randomBytes(uint256)") {
            let len = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let max_limit = self.config.max_dynamic_length as usize;
            let max_len = state
                .upper_bound_usize(&len)
                .filter(|len| *len <= max_limit)
                .map(Ok)
                .unwrap_or_else(|| {
                    self.solver_upper_bound_usize(
                        state,
                        &len,
                        max_limit,
                        "symbolic randomBytes length",
                    )
                })?;
            let bytes = (0..max_len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(CheatcodeOutcome::ContinueData(abi_bytes_return_with_len(len, bytes)));
        }
        if selector == selector!("randomBytes4()") {
            let value = state.fresh_bounded_uint(U256::from(32));
            return Ok(CheatcodeOutcome::Continue(vec![shift_left(value, 224)]));
        }
        if selector == selector!("randomBytes8()") {
            let value = state.fresh_bounded_uint(U256::from(64));
            return Ok(CheatcodeOutcome::Continue(vec![shift_left(value, 192)]));
        }

        Err(SymbolicError::Unsupported("symbolic Foundry cheatcode"))
    }

    /// Runs the `handle_symbolic_vm_cheatcode` symbolic executor helper.
    fn handle_symbolic_vm_cheatcode(
        &mut self,
        state: &mut PathState,
        selector: [u8; 4],
        in_offset: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        if selector == selector!("createUint256(string)")
            || selector == selector!("createInt256(string)")
            || selector == selector!("createBytes32(string)")
        {
            return Ok(SymReturnData::from_words(vec![state.fresh_word("svm")]));
        }
        for bits in (8..=256).step_by(8) {
            if selector == selector_for(&format!("createUint{bits}(string)"))
                || selector == selector_for(&format!("createInt{bits}(string)"))
            {
                if bits == 256 {
                    return Ok(SymReturnData::from_words(vec![state.fresh_word("svm")]));
                }
                return Ok(SymReturnData::from_words(vec![
                    state.fresh_bounded_uint(U256::from(bits)),
                ]));
            }
        }
        for bytes in 1..=32 {
            if selector == selector_for(&format!("createBytes{bytes}(string)")) {
                let value = state.fresh_bounded_uint(U256::from(bytes * 8));
                let value = if bytes == 32 { value } else { shift_left(value, (32 - bytes) * 8) };
                return Ok(SymReturnData::from_words(vec![value]));
            }
        }
        if selector == selector!("createUint(uint256,string)")
            || selector == selector!("createInt(uint256,string)")
        {
            let bits = read_abi_constrained_word_arg(
                state,
                in_offset + 4,
                0,
                "symbolic svm.create integer bits",
            )?;
            return Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(bits)]));
        }
        if selector == selector!("createAddress(string)") {
            return Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(U256::from(160))]));
        }
        if selector == selector!("createBool(string)") {
            return Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(U256::from(1))]));
        }
        if selector == selector!("createBytes(string)") {
            let len = self.config.default_dynamic_length as usize;
            let bytes = (0..len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createBytes(uint256,string)") {
            let len = read_abi_constrained_word_arg(
                state,
                in_offset + 4,
                0,
                "symbolic svm.createBytes length",
            )?;
            let len = u256_to_usize(len)
                .filter(|len| *len <= self.config.max_calldata_bytes as usize)
                .ok_or(SymbolicError::Unsupported("symbolic svm.createBytes length"))?;
            let bytes = (0..len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createString(string)") {
            let len = self.config.default_dynamic_length as usize;
            let bytes = (0..len)
                .map(|_| {
                    let byte = state.fresh_bounded_uint(U256::from(8));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Uge,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x20)),
                    ));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Ule,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x7e)),
                    ));
                    byte
                })
                .collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createString(uint256,string)") {
            let len = read_abi_constrained_word_arg(
                state,
                in_offset + 4,
                0,
                "symbolic svm.createString length",
            )?;
            let len = u256_to_usize(len)
                .filter(|len| *len <= self.config.max_calldata_bytes as usize)
                .ok_or(SymbolicError::Unsupported("symbolic svm.createString length"))?;
            let bytes = (0..len)
                .map(|_| {
                    let byte = state.fresh_bounded_uint(U256::from(8));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Uge,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x20)),
                    ));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Ule,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x7e)),
                    ));
                    byte
                })
                .collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createBytes4(string)") {
            return Ok(SymReturnData::from_words(vec![shift_left(
                state.fresh_bounded_uint(U256::from(32)),
                224,
            )]));
        }
        if selector == selector!("createCalldata(string)") {
            let max = self.config.max_calldata_bytes as usize;
            let len = if max < 4 {
                max
            } else {
                (self.config.default_dynamic_length as usize).max(4).min(max)
            };
            let bytes = (0..len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("enableSymbolicStorage(address)")
            || selector == selector!("setArbitraryStorage(address)")
        {
            let target = read_abi_address_or_symbolic_slot_arg(state, in_offset + 4, 0)?;
            state.world.enable_arbitrary_storage(target);
            return Ok(SymReturnData::default());
        }
        if selector == selector!("snapshotStorage(address)") {
            let _target = read_abi_address_or_symbolic_slot_arg(state, in_offset + 4, 0)?;
            let id = state.world.snapshot_state();
            return Ok(SymReturnData::from_words(vec![SymWord::Concrete(id)]));
        }
        if selector == selector!("snapshotState()") {
            let id = state.world.snapshot_state();
            return Ok(SymReturnData::from_words(vec![SymWord::Concrete(id)]));
        }

        Err(SymbolicError::Unsupported("symbolic VM compatibility cheatcode"))
    }

    /// Runs the `handle_assume` symbolic executor helper.
    fn handle_assume(
        &mut self,
        state: &mut PathState,
        condition_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let cond = state.memory.load_word(condition_offset)?;
        self.assume_condition(state, cond.nonzero_bool())
    }

    /// Runs the `handle_skip` symbolic executor helper.
    fn handle_skip(
        &mut self,
        state: &mut PathState,
        condition_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let cond = state.memory.load_word(condition_offset)?;
        self.assume_condition(state, cond.nonzero_bool().not())
    }

    /// Implements the `assume_condition` symbolic executor helper.
    fn assume_condition(
        &mut self,
        state: &mut PathState,
        condition: BoolExpr,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        match condition {
            BoolExpr::Const(true) => Ok(CheatcodeOutcome::Continue(Vec::new())),
            BoolExpr::Const(false) => Ok(CheatcodeOutcome::AssumeRejected),
            condition => {
                state.constraints.push(condition);
                if self.solver.is_sat(&state.constraints)? {
                    Ok(CheatcodeOutcome::Continue(Vec::new()))
                } else {
                    Ok(CheatcodeOutcome::AssumeRejected)
                }
            }
        }
    }

    /// Implements the `solver_upper_bound_usize` symbolic executor helper.
    fn solver_upper_bound_usize(
        &mut self,
        state: &PathState,
        word: &SymWord,
        max: usize,
        reason: &'static str,
    ) -> Result<usize, SymbolicError> {
        let expr = word.clone().into_expr();
        let mut above_max = state.constraints.clone();
        above_max.push(BoolExpr::cmp(BoolExprOp::Ugt, expr.clone(), Expr::Const(U256::from(max))));
        if self.solver.is_sat(&above_max)? {
            return Err(SymbolicError::Unsupported(reason));
        }

        let mut low = 0usize;
        let mut high = max;
        while low < high {
            let mid = low + (high - low) / 2;
            let mut above_mid = state.constraints.clone();
            above_mid.push(BoolExpr::cmp(
                BoolExprOp::Ugt,
                expr.clone(),
                Expr::Const(U256::from(mid)),
            ));
            if self.solver.is_sat(&above_mid)? {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Ok(low)
    }

    /// Implements the `assume_word_at_least` symbolic executor helper.
    fn assume_word_at_least(
        &mut self,
        state: &mut PathState,
        word: &SymWord,
        min: usize,
    ) -> Result<bool, SymbolicError> {
        let condition =
            BoolExpr::cmp(BoolExprOp::Uge, word.clone().into_expr(), Expr::Const(U256::from(min)));
        match condition {
            BoolExpr::Const(value) => Ok(value),
            condition => {
                let mut constraints = state.constraints.clone();
                constraints.push(condition);
                if self.solver.is_sat(&constraints)? {
                    state.constraints = constraints;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Runs the `handle_bound_uint` symbolic executor helper.
    fn handle_bound_uint(
        &mut self,
        state: &mut PathState,
        args_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let value = read_abi_word_arg(&state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&state.memory, args_offset, 2)?;

        if let (SymWord::Concrete(value), SymWord::Concrete(min), SymWord::Concrete(max)) =
            (&value, &min, &max)
        {
            if min >= max {
                return Ok(CheatcodeOutcome::Failure);
            }
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(bound_uint_concrete(
                *value, *min, *max,
            ))]));
        }

        if let (SymWord::Concrete(min), SymWord::Concrete(max)) = (&min, &max)
            && min >= max
        {
            return Ok(CheatcodeOutcome::Failure);
        }

        let bounded = state.fresh_word("vmBoundUint");
        state.constraints.push(BoolExpr::cmp(
            BoolExprOp::Uge,
            bounded.clone().into_expr(),
            min.into_expr(),
        ));
        state.constraints.push(BoolExpr::cmp(
            BoolExprOp::Ule,
            bounded.clone().into_expr(),
            max.into_expr(),
        ));
        Ok(CheatcodeOutcome::Continue(vec![bounded]))
    }

    /// Runs the `handle_bound_int` symbolic executor helper.
    fn handle_bound_int(
        &mut self,
        state: &mut PathState,
        args_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let value = read_abi_word_arg(&state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&state.memory, args_offset, 2)?;

        if let (SymWord::Concrete(value), SymWord::Concrete(min), SymWord::Concrete(max)) =
            (&value, &min, &max)
        {
            if !slt(*min, *max) {
                return Ok(CheatcodeOutcome::Failure);
            }
            let bounded = if !slt(*value, *min) && !slt(*max, *value) { *value } else { *min };
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(bounded)]));
        }

        if let (SymWord::Concrete(min), SymWord::Concrete(max)) = (&min, &max)
            && !slt(*min, *max)
        {
            return Ok(CheatcodeOutcome::Failure);
        }

        let bounded = state.fresh_word("vmBoundInt");
        state.constraints.push(
            BoolExpr::cmp(BoolExprOp::Slt, bounded.clone().into_expr(), min.into_expr()).not(),
        );
        state.constraints.push(
            BoolExpr::cmp(BoolExprOp::Sgt, bounded.clone().into_expr(), max.into_expr()).not(),
        );
        Ok(CheatcodeOutcome::Continue(vec![bounded]))
    }
}
