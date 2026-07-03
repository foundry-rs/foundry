use super::*;

impl SymbolicExecutor {
    pub(super) fn handle_assume(
        &mut self,
        state: &mut PathState,
        condition_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let cond = state.memory.load_word(&mut self.cx, condition_offset)?;
        let cond = cond.nonzero_bool(&mut self.cx);
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
                if condition.contains_gasleft() {
                    return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                }
                state.constraints.push(condition);
                if self.solver.is_sat(&mut self.cx, &state.constraints)? {
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
        if expr.contains_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let mut above_max = state.constraints.clone();
        above_max.push(SymBoolExpr::cmp_word_const(
            &mut self.cx,
            SymCmpOp::Ugt,
            expr,
            U256::from(max),
        ));
        if self.solver.is_sat(&mut self.cx, &above_max)? {
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
            if self.solver.is_sat(&mut self.cx, &above_mid)? {
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
        if expr.contains_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let condition =
            SymBoolExpr::cmp_word_const(&mut self.cx, SymCmpOp::Uge, expr, U256::from(min));
        match condition.as_const() {
            Some(value) => Ok(value),
            None => {
                let mut constraints = state.constraints.clone();
                constraints.push(condition);
                if self.solver.is_sat(&mut self.cx, &constraints)? {
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
        let value = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 2)?;

        if let (Some(value), Some(min), Some(max)) =
            (value.as_const(), min.as_const(), max.as_const())
        {
            if min >= max || value < min || value > max {
                return Ok(CheatcodeOutcome::Failure);
            }
            let bounded = if value == min { max } else { min };
            return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(&mut self.cx, bounded)]));
        }

        if let (Some(min), Some(max)) = (min.as_const(), max.as_const())
            && min >= max
        {
            return Ok(CheatcodeOutcome::Failure);
        }
        let (Some(min_word), Some(max_word)) = (min.as_const(), max.as_const()) else {
            return Err(SymbolicError::Unsupported("symbolic vm.bound range"));
        };

        let value_expr = value;
        let min_value = SymExpr::constant(&mut self.cx, min_word);
        let max_value = SymExpr::constant(&mut self.cx, max_word);
        let min_condition =
            SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Uge, value_expr.clone(), min_value);
        let max_condition =
            SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ule, value_expr.clone(), max_value);
        let in_range = SymBoolExpr::and(&mut self.cx, vec![min_condition, max_condition]);
        let (_in_range_constraints, in_range_sat) =
            self.constraints_with_condition(state, in_range.clone())?;
        if !in_range_sat {
            return Ok(CheatcodeOutcome::Failure);
        }
        let out_of_range = in_range.not(&mut self.cx);
        let (_out_of_range_constraints, out_of_range_sat) =
            self.constraints_with_condition(state, out_of_range)?;
        if out_of_range_sat {
            return Ok(CheatcodeOutcome::Failure);
        }

        let bounded = state.fresh_word(&mut self.cx, "vmBoundUint");
        let min_condition =
            SymBoolExpr::cmp_word_const(&mut self.cx, SymCmpOp::Uge, &bounded, min_word);
        let max_condition =
            SymBoolExpr::cmp_word_const(&mut self.cx, SymCmpOp::Ule, &bounded, max_word);
        state.constraints.push(min_condition);
        state.constraints.push(max_condition);
        let same_value = SymBoolExpr::eq(&mut self.cx, bounded.clone(), value_expr);
        state.constraints.push(same_value.not(&mut self.cx));
        Ok(CheatcodeOutcome::Continue(vec![bounded]))
    }

    pub(super) fn handle_bound_int(
        &mut self,
        state: &mut PathState,
        args_offset: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let value = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 2)?;

        if let (Some(value), Some(min), Some(max)) =
            (value.as_const(), min.as_const(), max.as_const())
        {
            if !slt(min, max) || slt(value, min) || slt(max, value) {
                return Ok(CheatcodeOutcome::Failure);
            }
            let bounded = if value == min { max } else { min };
            return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(&mut self.cx, bounded)]));
        }

        if let (Some(min), Some(max)) = (min.as_const(), max.as_const())
            && !slt(min, max)
        {
            return Ok(CheatcodeOutcome::Failure);
        }
        let (Some(min_word), Some(max_word)) = (min.as_const(), max.as_const()) else {
            return Err(SymbolicError::Unsupported("symbolic vm.bound range"));
        };

        let value_expr = value;
        let min_value = SymExpr::constant(&mut self.cx, min_word);
        let max_value = SymExpr::constant(&mut self.cx, max_word);
        let below_min =
            SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Slt, value_expr.clone(), min_value);
        let above_max =
            SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Sgt, value_expr.clone(), max_value);
        let below_min = below_min.not(&mut self.cx);
        let above_max = above_max.not(&mut self.cx);
        let in_range = SymBoolExpr::and(&mut self.cx, vec![below_min, above_max]);
        let (_in_range_constraints, in_range_sat) =
            self.constraints_with_condition(state, in_range.clone())?;
        if !in_range_sat {
            return Ok(CheatcodeOutcome::Failure);
        }
        let out_of_range = in_range.not(&mut self.cx);
        let (_out_of_range_constraints, out_of_range_sat) =
            self.constraints_with_condition(state, out_of_range)?;
        if out_of_range_sat {
            return Ok(CheatcodeOutcome::Failure);
        }

        let bounded = state.fresh_word(&mut self.cx, "vmBoundInt");
        let below_min =
            SymBoolExpr::cmp_word_const(&mut self.cx, SymCmpOp::Slt, &bounded, min_word);
        let above_max =
            SymBoolExpr::cmp_word_const(&mut self.cx, SymCmpOp::Sgt, &bounded, max_word);
        let below_min = below_min.not(&mut self.cx);
        let above_max = above_max.not(&mut self.cx);
        state.constraints.push(below_min);
        state.constraints.push(above_max);
        let same_value = SymBoolExpr::eq(&mut self.cx, bounded.clone(), value_expr);
        state.constraints.push(same_value.not(&mut self.cx));
        Ok(CheatcodeOutcome::Continue(vec![bounded]))
    }
}
