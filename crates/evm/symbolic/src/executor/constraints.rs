use foundry_cheatcodes_spec::Vm::{assumeCall, bound_0Call, bound_1Call, skip_0Call, skip_1Call};

use super::*;

impl SymbolicExecutor {
    /// Handles `vm.assume` in an invariant predicate, which must cover every reachable state.
    pub(super) fn handle_assume(
        &mut self,
        state: &mut PathState,
        condition_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let cond = state.memory.load_word(&mut self.cx, condition_offset)?;
        let cond = cond.nonzero_bool(&mut self.cx);
        if cond.as_const() != Some(true) {
            // Restricting the input domain would hide rejected states, including when accepted
            // values remain on this same path.
            let rejected = cond.not(&mut self.cx);
            let (_, rejected_sat) = self.constraints_with_condition(state, rejected)?;
            if rejected_sat {
                return Err(SymbolicError::Unsupported(
                    "vm.assume may reject an invariant predicate",
                ));
            }
        }
        Ok(CheatcodeOutcome::Continue(Vec::new()))
    }

    /// Forks `vm.assume` and `vm.skip` as Forge runs them: the accepted branch continues, and the
    /// rejected branch reverts the cheatcode call, so a caller can catch the revert and the run
    /// is discarded only when it reaches the test frame.
    #[expect(clippy::too_many_arguments)]
    pub(super) fn branch_assume_cheatcode_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        selector: [u8; 4],
        in_offset: Option<usize>,
        symbolic_offset_input: Option<&SymBytes>,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        let skip = matches!(selector, skip_0Call::SELECTOR | skip_1Call::SELECTOR);
        if !skip && (selector != assumeCall::SELECTOR || state.invariant_predicate) {
            return Ok(None);
        }
        // An input at a symbolic offset is decoded from a scratch memory that holds it at 0.
        let scratch = symbolic_offset_input.map(|input| {
            let mut scratch = SymMemory::default();
            scratch.store_bytes(&mut self.cx, 0, input.clone());
            scratch
        });
        let (memory, in_offset) = match &scratch {
            Some(scratch) => (scratch, 0),
            None => (&state.memory, in_offset.expect("cheatcode input offset is concrete")),
        };
        let cond = memory.load_word(&mut self.cx, in_offset + 4)?;
        let cond = cond.nonzero_bool(&mut self.cx);
        let accepted = if skip { cond.not(&mut self.cx) } else { cond };
        let revert_data = if !skip {
            SymReturnData::from_concrete_bytes(&mut self.cx, MAGIC_ASSUME.to_vec())
        } else if state.call_depth == 0 {
            let mut payload = MAGIC_SKIP.to_vec();
            if selector == skip_1Call::SELECTOR {
                let reason = read_abi_string_arg(
                    &mut self.cx,
                    memory,
                    in_offset + 4,
                    1,
                    "symbolic vm.skip reason",
                )?;
                payload.extend_from_slice(reason.as_bytes());
            }
            SymReturnData::from_concrete_bytes(&mut self.cx, payload)
        } else {
            error_string_return_data(&mut self.cx, "vm.skip: `skip` can only be used at test level")
        };
        let return_data = SymReturnData::empty(&mut self.cx);
        match accepted.as_const() {
            Some(true) => {
                complete_cheatcode_call(&mut self.cx, state, out_offset, out_size, return_data)?;
            }
            Some(false) => {
                revert_cheatcode_call(&mut self.cx, state, out_offset, out_size, revert_data)?;
            }
            None => {
                let (accepted_seed_models, rejected_seed_models) =
                    state.split_corpus_seed_models(&accepted);
                let mut accepted_state = state.clone();
                accepted_state.constraints.push(accepted.clone());
                accepted_state.set_corpus_seed_models(accepted_seed_models);
                accepted_state.defer_feasibility_check();
                complete_cheatcode_call(
                    &mut self.cx,
                    &mut accepted_state,
                    out_offset.clone(),
                    out_size,
                    return_data,
                )?;
                worklist.push_back(accepted_state);
                // The rejected branch runs in place: an uncaught revert discards it without a
                // solver query. A caught one is checked before it calls out or completes.
                state.constraints.push(accepted.not(&mut self.cx));
                state.set_corpus_seed_models(rejected_seed_models);
                state.defer_feasibility_check();
                revert_cheatcode_call(&mut self.cx, state, out_offset, out_size, revert_data)?;
            }
        }
        Ok(Some(StepOutcome::Continue))
    }

    pub(super) fn solver_upper_bound_usize(
        &mut self,
        state: &PathState,
        expr: &SymExpr,
        max: usize,
        reason: &'static str,
    ) -> Result<usize, SymbolicError> {
        if let Some(bound) =
            state.upper_bound_usize(&mut self.cx, expr).filter(|bound| *bound <= max)
        {
            return Ok(bound);
        }
        let mut above_max = state.constraints.clone();
        above_max.push(SymBoolExpr::cmp_word_const(
            &mut self.cx,
            SymCmpOp::Ugt,
            expr,
            U256::from(max),
        ));
        if self.is_sat_with_state(state, &above_max)? {
            return Err(SymbolicError::Unsupported(reason));
        }

        let mut low = 0usize;
        let mut high = max;
        while low < high {
            let mid = low + (high - low) / 2;
            let mut above_mid = state.constraints.clone();
            above_mid.push(SymBoolExpr::cmp_word_const(
                &mut self.cx,
                SymCmpOp::Ugt,
                expr,
                U256::from(mid),
            ));
            if self.is_sat_with_state(state, &above_mid)? {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Ok(low)
    }

    pub(super) fn assume_expr_at_least(
        &mut self,
        state: &mut PathState,
        expr: &SymExpr,
        min: usize,
    ) -> Result<bool, SymbolicError> {
        let condition =
            SymBoolExpr::cmp_word_const(&mut self.cx, SymCmpOp::Uge, expr, U256::from(min));
        match condition.as_const() {
            Some(value) => Ok(value),
            None => {
                let mut constraints = state.constraints.clone();
                constraints.push(condition);
                if self.is_sat_with_state(state, &constraints)? {
                    state.constraints = constraints;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Proves that every feasible value is at least `min` without restricting the path.
    pub(super) fn proves_expr_at_least(
        &mut self,
        state: &PathState,
        expr: &SymExpr,
        min: usize,
    ) -> Result<bool, SymbolicError> {
        if state.lower_bound_usize(expr) >= min {
            return Ok(true);
        }

        let mut below_min = state.constraints.clone();
        below_min.push(SymBoolExpr::cmp_word_const(
            &mut self.cx,
            SymCmpOp::Ult,
            expr,
            U256::from(min),
        ));
        Ok(!self.is_sat_with_state(state, &below_min)?)
    }

    /// Resolves a path-constant word and proves that no alternate value is feasible.
    pub(super) fn constrained_word_with_solver(
        &mut self,
        state: &PathState,
        expr: &SymExpr,
    ) -> Result<Option<U256>, SymbolicError> {
        if let Some(value) = state.constrained_word(&mut self.cx, expr) {
            return Ok(Some(value));
        }
        if expr.contains_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }

        let replayable_storage = state.world.replay_storage_symbols();
        let model = self.solver.model_with_replayable_storage(
            &mut self.cx,
            &state.constraints,
            &replayable_storage,
        )?;
        let value = expr.eval_model(&model)?;
        let differs = SymBoolExpr::eq_word_const(&mut self.cx, expr, value).not(&mut self.cx);
        let mut constraints = state.constraints.clone();
        constraints.push(differs);
        if self.is_sat_with_state(state, &constraints)? { Ok(None) } else { Ok(Some(value)) }
    }

    /// Rejects symbolic integer bit widths outside the EVM word size.
    pub(super) fn validate_symbolic_integer_bits(
        bits: U256,
        context: &'static str,
    ) -> Result<(), SymbolicError> {
        if bits <= U256::from(256) { Ok(()) } else { Err(SymbolicError::Unsupported(context)) }
    }

    /// Handles `vm.bound` when `selector` names it.
    pub(super) fn bound_cheatcode_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        selector: [u8; 4],
        in_offset: usize,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        let signed = match selector {
            bound_0Call::SELECTOR => false,
            bound_1Call::SELECTOR => true,
            _ => return Ok(None),
        };
        self.handle_bound(state, worklist, in_offset + 4, signed, out_offset, out_size).map(Some)
    }

    /// Handles `vm.bound` for unsigned or signed (`int256`) ranges.
    ///
    /// Like Forge, a value outside the range reverts the cheatcode call. When the path allows
    /// both, the revert runs on its own branch.
    fn handle_bound(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        args_offset: usize,
        signed: bool,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        let value = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 2)?;
        let (Some(min_word), Some(max_word)) = (min.as_const(), max.as_const()) else {
            return Err(SymbolicError::Unsupported("symbolic vm.bound range"));
        };
        let order = |a: &U256, b: &U256| {
            if signed { I256::from_raw(*a).cmp(&I256::from_raw(*b)) } else { a.cmp(b) }
        };
        if !order(&min_word, &max_word).is_lt() {
            let revert = self.bound_revert_data(state, &value, min_word, max_word, signed);
            revert_cheatcode_call(&mut self.cx, state, out_offset, out_size, revert)?;
            return Ok(StepOutcome::Continue);
        }

        // Signed bounds are expressed as `!(x < min) && !(x > max)`.
        let range_conditions = |cx: &mut SymCx, word: &SymExpr, as_consts: bool| {
            let cmp = |cx: &mut SymCx, op, bound| {
                if as_consts {
                    SymBoolExpr::cmp_word_const(cx, op, word, bound)
                } else {
                    let bound = SymExpr::constant(cx, bound);
                    SymBoolExpr::cmp_word_expr(cx, op, word, bound)
                }
            };
            if signed {
                let below_min = cmp(cx, SymCmpOp::Slt, min_word).not(cx);
                let above_max = cmp(cx, SymCmpOp::Sgt, max_word).not(cx);
                [below_min, above_max]
            } else {
                [cmp(cx, SymCmpOp::Uge, min_word), cmp(cx, SymCmpOp::Ule, max_word)]
            }
        };
        let in_range = range_conditions(&mut self.cx, &value, false);
        let in_range = SymBoolExpr::and(&mut self.cx, in_range.into());
        let (in_range_constraints, in_range_sat) =
            self.constraints_with_condition(state, in_range.clone())?;
        let out_of_range = in_range.not(&mut self.cx);
        if let Some(out_of_range_constraints) =
            self.constraints_for_condition(state, out_of_range)?
        {
            if !in_range_sat {
                let revert = self.bound_revert_data(state, &value, min_word, max_word, signed);
                revert_cheatcode_call(&mut self.cx, state, out_offset, out_size, revert)?;
                return Ok(StepOutcome::Continue);
            }
            let mut reverted = state.clone();
            reverted.constraints = out_of_range_constraints;
            let revert = self.bound_revert_data(&reverted, &value, min_word, max_word, signed);
            revert_cheatcode_call(
                &mut self.cx,
                &mut reverted,
                out_offset.clone(),
                out_size,
                revert,
            )?;
            worklist.push_back(reverted);
        }

        state.constraints = in_range_constraints;
        let bounded =
            state.fresh_word(&mut self.cx, if signed { "vmBoundInt" } else { "vmBoundUint" });
        state.constraints.extend(range_conditions(&mut self.cx, &bounded, true));
        let same_value = SymBoolExpr::eq(&mut self.cx, bounded.clone(), value);
        state.constraints.push(same_value.not(&mut self.cx));
        let return_data = SymReturnData::from_words(&mut self.cx, vec![bounded]);
        complete_cheatcode_call(&mut self.cx, state, out_offset, out_size, return_data)?;
        Ok(StepOutcome::Continue)
    }

    fn bound_revert_data(
        &mut self,
        state: &PathState,
        value: &SymExpr,
        min: U256,
        max: U256,
        signed: bool,
    ) -> SymReturnData {
        let Some(value) = state.constrained_word(&mut self.cx, value) else {
            // A caught revert may inspect its bytes. The payload depends on the symbolic value,
            // so a placeholder must not turn this path into a proof.
            self.defer_incomplete("symbolic vm.bound revert payload");
            return error_string_return_data(
                &mut self.cx,
                "vm.bound: cannot bound symbolic value in range",
            );
        };
        let message = if signed {
            format!(
                "vm.bound: cannot bound {} in [{}, {}] range",
                I256::from_raw(value),
                I256::from_raw(min),
                I256::from_raw(max)
            )
        } else {
            format!("vm.bound: cannot bound {value} in [{min}, {max}] range")
        };
        error_string_return_data(&mut self.cx, &message)
    }
}
