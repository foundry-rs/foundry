use foundry_cheatcodes_spec::Vm::{bound_0Call, bound_1Call};

use super::*;

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
    ///
    /// Like Forge, a value outside the range reverts the cheatcode call. When the path allows
    /// both, the revert runs on its own branch.
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
        let args_offset = in_offset + 4;
        let value = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
        let min = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
        let max = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 2)?;
        let (Some(min_word), Some(max_word)) = (min.as_const(), max.as_const()) else {
            return Err(SymbolicError::Unsupported("symbolic vm.bound range"));
        };
        let order = |a: &U256, b: &U256| {
            if signed { I256::from_raw(*a).cmp(&I256::from_raw(*b)) } else { a.cmp(b) }
        };

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
        let out_of_range = if order(&min_word, &max_word).is_lt() {
            let in_range = range_conditions(&mut self.cx, &value, false);
            SymBoolExpr::and(&mut self.cx, in_range.into()).not(&mut self.cx)
        } else {
            SymBoolExpr::constant(&mut self.cx, true)
        };
        if self.fork_cheatcode_revert(
            state,
            worklist,
            out_of_range,
            &out_offset,
            out_size,
            |this, reverted| this.bound_revert_data(reverted, &value, min_word, max_word, signed),
        )? {
            return Ok(Some(StepOutcome::Continue));
        }

        let bounded =
            state.fresh_word(&mut self.cx, if signed { "vmBoundInt" } else { "vmBoundUint" });
        state.constraints.extend(range_conditions(&mut self.cx, &bounded, true));
        let same_value = SymBoolExpr::eq(&mut self.cx, bounded.clone(), value);
        state.constraints.push(same_value.not(&mut self.cx));
        let return_data = SymReturnData::from_words(&mut self.cx, vec![bounded]);
        complete_cheatcode_call(&mut self.cx, state, out_offset, out_size, return_data)?;
        Ok(Some(StepOutcome::Continue))
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
