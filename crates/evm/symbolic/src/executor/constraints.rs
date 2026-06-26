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
        condition: BoolExpr,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        match condition {
            BoolExpr::Const(true) => Ok(CheatcodeOutcome::Continue(Vec::new())),
            BoolExpr::Const(false) => Ok(CheatcodeOutcome::AssumeRejected),
            condition => {
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
        word: &SymWord,
        max: usize,
        reason: &'static str,
    ) -> Result<usize, SymbolicError> {
        if word.contains_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let mut above_max = state.constraints.clone();
        above_max.push(BoolExpr::cmp_word_const(BoolExprOp::Ugt, word, U256::from(max)));
        if self.solver.is_sat(&above_max)? {
            return Err(SymbolicError::Unsupported(reason));
        }

        let mut low = 0usize;
        let mut high = max;
        while low < high {
            let mid = low + (high - low) / 2;
            let mut above_mid = state.constraints.clone();
            above_mid.push(BoolExpr::cmp_word_const(BoolExprOp::Ugt, word, U256::from(mid)));
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
        word: &SymWord,
        min: usize,
    ) -> Result<bool, SymbolicError> {
        if word.contains_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let condition = BoolExpr::cmp_word_const(BoolExprOp::Uge, word, U256::from(min));
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
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::constant(bounded)]));
        }

        if let (Some(min), Some(max)) = (min.as_const(), max.as_const())
            && min >= max
        {
            return Ok(CheatcodeOutcome::Failure);
        }
        let (Some(min_value), Some(max_value)) = (min.as_const(), max.as_const()) else {
            return Err(SymbolicError::Unsupported("symbolic vm.bound range"));
        };

        let value_expr = value.into_expr();
        let in_range = BoolExpr::and(vec![
            BoolExpr::cmp(BoolExprOp::Uge, value_expr.clone(), Expr::constant(min_value)),
            BoolExpr::cmp(BoolExprOp::Ule, value_expr.clone(), Expr::constant(max_value)),
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
        state.constraints.push(BoolExpr::cmp_word_const(BoolExprOp::Uge, &bounded, min_value));
        state.constraints.push(BoolExpr::cmp_word_const(BoolExprOp::Ule, &bounded, max_value));
        state.constraints.push(BoolExpr::eq_word_expr(&bounded, value_expr).not());
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
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::constant(bounded)]));
        }

        if let (Some(min), Some(max)) = (min.as_const(), max.as_const())
            && !slt(min, max)
        {
            return Ok(CheatcodeOutcome::Failure);
        }
        let (Some(min_value), Some(max_value)) = (min.as_const(), max.as_const()) else {
            return Err(SymbolicError::Unsupported("symbolic vm.bound range"));
        };

        let value_expr = value.into_expr();
        let in_range = BoolExpr::and(vec![
            BoolExpr::cmp(BoolExprOp::Slt, value_expr.clone(), Expr::constant(min_value)).not(),
            BoolExpr::cmp(BoolExprOp::Sgt, value_expr.clone(), Expr::constant(max_value)).not(),
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
            .push(BoolExpr::cmp_word_const(BoolExprOp::Slt, &bounded, min_value).not());
        state
            .constraints
            .push(BoolExpr::cmp_word_const(BoolExprOp::Sgt, &bounded, max_value).not());
        state.constraints.push(BoolExpr::eq_word_expr(&bounded, value_expr).not());
        Ok(CheatcodeOutcome::Continue(vec![bounded]))
    }
}
