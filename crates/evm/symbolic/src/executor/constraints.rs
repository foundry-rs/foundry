use super::*;
use foundry_evm::revm::interpreter::instructions::i256::i256_cmp;

impl SymbolicExecutor {
    pub(super) fn handle_assume(
        &mut self,
        state: &mut PathState,
        condition_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let cond = state.memory.load_word(&mut self.cx, condition_offset)?;
        let cond = cond.nonzero_bool(&mut self.cx);
        if state.invariant_predicate && cond.as_const() != Some(true) {
            // A predicate must cover every reachable state. Restricting its input domain would
            // hide rejected states, including when accepted values remain on this same path.
            let rejected = cond.not(&mut self.cx);
            let (_, rejected_sat) = self.constraints_with_condition(state, rejected)?;
            if rejected_sat {
                return Err(SymbolicError::Unsupported(
                    "vm.assume may reject an invariant predicate",
                ));
            }
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        self.assume_condition(state, cond)
    }

    pub(super) fn handle_skip(
        &mut self,
        state: &mut PathState,
        condition_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let cond = state.memory.load_word(&mut self.cx, condition_offset)?;
        let cond = cond.nonzero_bool(&mut self.cx).not(&mut self.cx);
        self.assume_condition(state, cond)
    }

    pub(super) fn assume_condition(
        &mut self,
        state: &mut PathState,
        condition: SymBoolExpr,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        match condition.as_const() {
            Some(true) => Ok(CheatcodeOutcome::Continue(Vec::new())),
            Some(false) => Ok(CheatcodeOutcome::AssumeRejected),
            None => {
                state.constraints.push(condition);
                if self.is_sat_with_state(state, &state.constraints)? {
                    Ok(CheatcodeOutcome::Continue(Vec::new()))
                } else {
                    Ok(CheatcodeOutcome::AssumeRejected)
                }
            }
        }
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

    /// Handles `vm.bound` for unsigned or signed (`int256`) ranges.
    pub(super) fn handle_bound(
        &mut self,
        state: &mut PathState,
        args_offset: usize,
        signed: bool,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let value = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 2)?;
        let order = |a: &U256, b: &U256| if signed { i256_cmp(a, b) } else { a.cmp(b) };

        if let (Some(value), Some(min), Some(max)) =
            (value.as_const(), min.as_const(), max.as_const())
        {
            if !order(&min, &max).is_lt()
                || order(&value, &min).is_lt()
                || order(&value, &max).is_gt()
            {
                return Ok(CheatcodeOutcome::Failure);
            }
            let bounded = if value == min { max } else { min };
            return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(&mut self.cx, bounded)]));
        }

        if let (Some(min), Some(max)) = (min.as_const(), max.as_const())
            && !order(&min, &max).is_lt()
        {
            return Ok(CheatcodeOutcome::Failure);
        }
        let (Some(min_word), Some(max_word)) = (min.as_const(), max.as_const()) else {
            return Err(SymbolicError::Unsupported("symbolic vm.bound range"));
        };

        // Signed bounds are expressed as `!(x < min) && !(x > max)`.
        let range_conditions = |cx: &mut SymCx, word: &SymExpr, as_consts: bool| {
            let cmp = |cx: &mut SymCx, op, bound| {
                if as_consts {
                    SymBoolExpr::cmp_word_const(cx, op, word, bound)
                } else {
                    let bound = SymExpr::constant(cx, bound);
                    SymBoolExpr::cmp(cx, op, word.clone(), bound)
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
        let (_, in_range_sat) = self.constraints_with_condition(state, in_range.clone())?;
        if !in_range_sat {
            return Ok(CheatcodeOutcome::Failure);
        }
        let out_of_range = in_range.not(&mut self.cx);
        let (out_of_range_constraints, out_of_range_sat) =
            self.constraints_with_condition(state, out_of_range)?;
        if out_of_range_sat {
            state.constraints = out_of_range_constraints;
            return Ok(CheatcodeOutcome::Failure);
        }

        let bounded =
            state.fresh_word(&mut self.cx, if signed { "vmBoundInt" } else { "vmBoundUint" });
        state.constraints.extend(range_conditions(&mut self.cx, &bounded, true));
        let same_value = SymBoolExpr::eq(&mut self.cx, bounded.clone(), value);
        state.constraints.push(same_value.not(&mut self.cx));
        Ok(CheatcodeOutcome::Continue(vec![bounded]))
    }
}
