use super::*;

impl SymbolicExecutor {
    pub(super) fn handle_assume(
        &mut self,
        state: &mut PathState,
        condition_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let cond = state.memory.load_word(condition_offset)?;
        self.assume_condition(state, cond.nonzero_bool())
    }

    pub(super) fn handle_skip(
        &mut self,
        state: &mut PathState,
        condition_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let cond = state.memory.load_word(condition_offset)?;
        self.assume_condition(state, cond.nonzero_bool().not())
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
                if bool_contains_gasleft(&condition) {
                    return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                }
                state.constraints.push(condition);
                if self.solver.is_sat(&state.constraints)? {
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
        word: &SymExpr,
        max: usize,
        reason: &'static str,
    ) -> Result<usize, SymbolicError> {
        if word.contains_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let mut above_max = state.constraints.clone();
        above_max.push(SymBoolExpr::cmp_word_const(SymBoolExprOp::Ugt, word, U256::from(max)));
        if self.solver.is_sat(&above_max)? {
            return Err(SymbolicError::Unsupported(reason));
        }

        let mut low = 0usize;
        let mut high = max;
        while low < high {
            let mid = low + (high - low) / 2;
            let mut above_mid = state.constraints.clone();
            above_mid.push(SymBoolExpr::cmp_word_const(SymBoolExprOp::Ugt, word, U256::from(mid)));
            if self.solver.is_sat(&above_mid)? {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Ok(low)
    }

    pub(super) fn assume_word_at_least(
        &mut self,
        state: &mut PathState,
        word: &SymExpr,
        min: usize,
    ) -> Result<bool, SymbolicError> {
        if word.contains_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let condition = SymBoolExpr::cmp_word_const(SymBoolExprOp::Uge, word, U256::from(min));
        match condition.as_const() {
            Some(value) => Ok(value),
            None => {
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

    /// Rejects symbolic integer bit widths outside the EVM word size.
    pub(super) fn validate_symbolic_integer_bits(
        bits: U256,
        context: &'static str,
    ) -> Result<(), SymbolicError> {
        if bits <= U256::from(256) { Ok(()) } else { Err(SymbolicError::Unsupported(context)) }
    }

    pub(super) fn handle_bound_uint(
        &mut self,
        state: &mut PathState,
        args_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let value = read_abi_word_arg(&state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&state.memory, args_offset, 2)?;

        if let (Some(value), Some(min), Some(max)) =
            (value.as_const(), min.as_const(), max.as_const())
        {
            if min >= max || value < min || value > max {
                return Ok(CheatcodeOutcome::Failure);
            }
            let bounded = if value == min { max } else { min };
            return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(bounded)]));
        }

        if let (Some(min), Some(max)) = (min.as_const(), max.as_const())
            && min >= max
        {
            return Ok(CheatcodeOutcome::Failure);
        }
        let (Some(min_value), Some(max_value)) = (min.as_const(), max.as_const()) else {
            return Err(SymbolicError::Unsupported("symbolic vm.bound range"));
        };

        let value_expr = value;
        let in_range = SymBoolExpr::and(vec![
            SymBoolExpr::cmp(SymBoolExprOp::Uge, value_expr.clone(), SymExpr::constant(min_value)),
            SymBoolExpr::cmp(SymBoolExprOp::Ule, value_expr.clone(), SymExpr::constant(max_value)),
        ]);
        let (_in_range_constraints, in_range_sat) =
            self.constraints_with_condition(state, in_range.clone())?;
        if !in_range_sat {
            return Ok(CheatcodeOutcome::Failure);
        }
        let (_out_of_range_constraints, out_of_range_sat) =
            self.constraints_with_condition(state, in_range.not())?;
        if out_of_range_sat {
            return Ok(CheatcodeOutcome::Failure);
        }

        let bounded = state.fresh_word("vmBoundUint");
        state.constraints.push(SymBoolExpr::cmp_word_const(
            SymBoolExprOp::Uge,
            &bounded,
            min_value,
        ));
        state.constraints.push(SymBoolExpr::cmp_word_const(
            SymBoolExprOp::Ule,
            &bounded,
            max_value,
        ));
        state.constraints.push(SymBoolExpr::eq_word_expr(&bounded, value_expr).not());
        Ok(CheatcodeOutcome::Continue(vec![bounded]))
    }

    pub(super) fn handle_bound_int(
        &mut self,
        state: &mut PathState,
        args_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let value = read_abi_word_arg(&state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&state.memory, args_offset, 2)?;

        if let (Some(value), Some(min), Some(max)) =
            (value.as_const(), min.as_const(), max.as_const())
        {
            if !slt(min, max) || slt(value, min) || slt(max, value) {
                return Ok(CheatcodeOutcome::Failure);
            }
            let bounded = if value == min { max } else { min };
            return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(bounded)]));
        }

        if let (Some(min), Some(max)) = (min.as_const(), max.as_const())
            && !slt(min, max)
        {
            return Ok(CheatcodeOutcome::Failure);
        }
        let (Some(min_value), Some(max_value)) = (min.as_const(), max.as_const()) else {
            return Err(SymbolicError::Unsupported("symbolic vm.bound range"));
        };

        let value_expr = value;
        let in_range = SymBoolExpr::and(vec![
            SymBoolExpr::cmp(SymBoolExprOp::Slt, value_expr.clone(), SymExpr::constant(min_value))
                .not(),
            SymBoolExpr::cmp(SymBoolExprOp::Sgt, value_expr.clone(), SymExpr::constant(max_value))
                .not(),
        ]);
        let (_in_range_constraints, in_range_sat) =
            self.constraints_with_condition(state, in_range.clone())?;
        if !in_range_sat {
            return Ok(CheatcodeOutcome::Failure);
        }
        let (_out_of_range_constraints, out_of_range_sat) =
            self.constraints_with_condition(state, in_range.not())?;
        if out_of_range_sat {
            return Ok(CheatcodeOutcome::Failure);
        }

        let bounded = state.fresh_word("vmBoundInt");
        state
            .constraints
            .push(SymBoolExpr::cmp_word_const(SymBoolExprOp::Slt, &bounded, min_value).not());
        state
            .constraints
            .push(SymBoolExpr::cmp_word_const(SymBoolExprOp::Sgt, &bounded, max_value).not());
        state.constraints.push(SymBoolExpr::eq_word_expr(&bounded, value_expr).not());
        Ok(CheatcodeOutcome::Continue(vec![bounded]))
    }
}
