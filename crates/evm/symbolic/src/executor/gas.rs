use super::*;
use foundry_evm::revm::{
    context_interface::cfg::{GasParams, gas_params::num_words},
    interpreter::instructions::gas_table_spec,
};

/// Largest memory size, in bytes, that the gas meter prices. Larger accesses run out of gas.
const MAX_METERED_MEMORY: usize = u32::MAX as usize;

/// Exclusive upper end of the gas bound search, far above any block gas limit.
const MAX_SEARCHED_GAS: u64 = 1 << 40;

/// A concrete input for a path, with the sender, storage and immutables it assumes.
struct WitnessReplay {
    input: SymbolicConcreteInput,
    sender: Option<Address>,
    storage: Vec<SymbolicStorageAssignment>,
    immutables: Vec<(usize, U256)>,
}

impl SymbolicExecutor {
    /// Charges the gas of `op` to the path before the opcode executes.
    ///
    /// Mirrors revm's static and dynamic gas for the executor's spec. Costs that depend on a
    /// symbolic operand are charged at their maximum, or marked unmodeled when no bound exists.
    pub(super) fn charge_gas<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        op: u8,
    ) {
        let spec: SpecId = executor.spec_id().into();
        if !spec.is_enabled_in(SpecId::BERLIN) || spec.is_enabled_in(SpecId::AMSTERDAM) {
            state.gas.unmodeled("symbolic gas is modeled from Berlin to Osaka");
            return;
        }
        let params = executor.evm_env().cfg_env.gas_params();
        state.gas.charge(u64::from(gas_table_spec(spec)[op as usize]));

        match op {
            opcode::EXP => {
                let power = self.operand(state, 1).unwrap_or(U256::MAX);
                state.gas.charge(params.exp_cost(power));
            }
            opcode::KECCAK256 => {
                self.charge_sized(state, 0, 1, |len| params.keccak256_cost(len), params);
            }
            opcode::CALLDATACOPY | opcode::CODECOPY | opcode::RETURNDATACOPY => {
                self.charge_sized(state, 0, 2, |len| params.copy_cost(len), params);
            }
            opcode::EXTCODECOPY => {
                self.charge_account(state, 0, params);
                self.charge_sized(state, 1, 3, |len| params.extcodecopy(len), params);
            }
            opcode::MCOPY => match self.operand_usize(state, 2) {
                Some(len) => {
                    state.gas.charge(params.mcopy_cost(len));
                    self.charge_memory(state, &[(0, 2), (1, 2)], params);
                }
                None => state.gas.unmodeled("symbolic gas for a symbolic length not modeled"),
            },
            opcode::MLOAD | opcode::MSTORE => self.charge_memory_len(state, 32, params),
            opcode::MSTORE8 => self.charge_memory_len(state, 1, params),
            opcode::LOG0..=opcode::LOG4 => {
                let topics = op - opcode::LOG0;
                self.charge_sized(state, 0, 1, |len| params.log_cost(topics, len as u64), params);
            }
            opcode::RETURN | opcode::REVERT => self.charge_memory(state, &[(0, 1)], params),
            opcode::BALANCE | opcode::EXTCODESIZE | opcode::EXTCODEHASH => {
                self.charge_account(state, 0, params);
            }
            opcode::SLOAD => {
                if let Ok(slot) = state.stack.peek(0).cloned()
                    && state.gas.warm.slots.insert((state.storage_address, slot))
                {
                    state.gas.charge(params.cold_storage_additional_cost());
                }
            }
            opcode::SSTORE => {
                let (Ok(key), Ok(value)) =
                    (state.stack.peek(0).cloned(), state.stack.peek(1).cloned())
                else {
                    return;
                };
                state.gas.charge(params.sstore_static_gas());
                if state.gas.warm.slots.insert((state.storage_address, key.clone())) {
                    state.gas.charge(params.cold_storage_cost());
                }
                self.charge_sstore(executor, state, key, value, params);
            }
            opcode::CALL | opcode::CALLCODE | opcode::DELEGATECALL | opcode::STATICCALL => {
                self.charge_call(executor, state, op, params);
            }
            opcode::CREATE | opcode::CREATE2 => {
                state.gas.unmodeled("symbolic gas for CREATE not modeled");
            }
            opcode::SELFDESTRUCT => {
                state.gas.unmodeled("symbolic gas for SELFDESTRUCT not modeled");
            }
            _ => {}
        }
    }

    /// Charges the EIP-2200 cost and EIP-3529 refund of storing `value` at `key`, without the
    /// static and cold costs.
    ///
    /// Both depend on the slot's present and original values, which can be symbolic. If the
    /// original value is unknown, charges the cost of writing an empty slot, the most expensive
    /// case, and no refund.
    fn charge_sstore<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        key: SymExpr,
        value: SymExpr,
        params: &GasParams,
    ) {
        let cx = &mut self.cx;
        let set = SymExpr::constant(cx, U256::from(params.sstore_set_without_load_cost()));
        if !state.gas.is_measuring() {
            state.gas.charge_expr(set);
            return;
        }
        let address = state.storage_address;
        let concrete_key = state.constrained_word(cx, &key);
        let values = state.gas.transaction_writes.and_then(|writes| {
            let present = state.world.sload(cx, executor, address, key.clone(), concrete_key);
            let original =
                state.world.sload_before(cx, executor, address, key, concrete_key, writes);
            present.ok().zip(original.ok())
        });
        let Some((present, original)) = values else {
            // Without the original value, a store of zero costs at most a reset: a set needs a
            // nonzero value.
            let reset =
                SymExpr::constant(cx, U256::from(params.sstore_reset_without_cold_load_cost()));
            let value_zero = SymBoolExpr::eq_word_const(cx, &value, U256::ZERO);
            state.gas.charge_expr(SymExpr::ite(cx, value_zero, reset, set));
            return;
        };

        let constant = |cx: &mut SymCx, value: u64| SymExpr::constant(cx, U256::from(value));
        let zero = SymExpr::zero(cx);
        let unchanged = SymBoolExpr::eq(cx, value.clone(), present.clone());
        let clean = SymBoolExpr::eq(cx, original.clone(), present.clone());
        let original_zero = SymBoolExpr::eq_word_const(cx, &original, U256::ZERO);
        let present_zero = SymBoolExpr::eq_word_const(cx, &present, U256::ZERO);
        let value_zero = SymBoolExpr::eq_word_const(cx, &value, U256::ZERO);

        // Cost: a changed clean slot pays for a set or a reset; every other store is warm.
        let changes = unchanged.clone().not(cx);
        let first_write = SymBoolExpr::and(cx, vec![changes, clean.clone()]);
        let reset = constant(cx, params.sstore_reset_without_cold_load_cost());
        let cost = SymExpr::ite(cx, original_zero.clone(), set, reset);
        let cost = SymExpr::ite(cx, first_write, cost, zero.clone());
        state.gas.charge_expr(cost);

        // Refund, as `GasParams::sstore_refund` computes it for Istanbul and later.
        let clear = params.sstore_clearing_slot_refund();
        let clear_refund = constant(cx, clear);
        let clear_penalty = SymExpr::constant(cx, U256::ZERO.wrapping_sub(U256::from(clear)));
        let dirty_clear = SymExpr::ite(cx, value_zero.clone(), clear_refund.clone(), zero.clone());
        let dirty_clear = SymExpr::ite(cx, present_zero, clear_penalty, dirty_clear);
        let dirty_clear = SymExpr::ite(cx, original_zero.clone(), zero.clone(), dirty_clear);
        let restored = SymBoolExpr::eq(cx, original, value);
        let set_refund = constant(cx, params.sstore_set_refund());
        let reset_refund = constant(cx, params.sstore_reset_refund());
        let restore = SymExpr::ite(cx, original_zero, set_refund, reset_refund);
        let restore = SymExpr::ite(cx, restored, restore, zero.clone());
        let dirty = SymExpr::binop(cx, SymBinOp::Add, dirty_clear, restore);
        let clears = SymBoolExpr::and(cx, vec![clean, value_zero]);
        let refund = SymExpr::ite(cx, clears, clear_refund, dirty);
        let refund = SymExpr::ite(cx, unchanged, zero, refund);
        state.gas.refund(refund);
    }

    fn charge_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        op: u8,
        params: &GasParams,
    ) {
        let has_value = matches!(op, opcode::CALL | opcode::CALLCODE);
        let args = if has_value { 3 } else { 2 };
        self.charge_memory(state, &[(args, args + 1), (args + 2, args + 3)], params);
        let spec: SpecId = executor.spec_id().into();
        let isolated = executor.inspector().enable_isolation && state.call_depth == 0;
        let target =
            state.stack.peek(1).ok().and_then(|target| state.world.resolve_address(target));
        if let Some(target) = target
            && precompile_number_for_spec(target, spec).is_some()
        {
            // EIP-2929 makes precompiles warm. Their own cost is charged here, as the callee's.
            match self.operand_usize(state, args).zip(self.operand_usize(state, args + 1)) {
                Some((offset, len)) if !isolated => {
                    let input = state
                        .memory
                        .read_bytes(&mut self.cx, offset, len)
                        .materialize(&mut self.cx);
                    let input = input
                        .iter()
                        .map(|byte| byte.as_const().map(|byte| byte.byte(0)))
                        .collect::<Vec<_>>();
                    match precompile_gas(target, &input, spec) {
                        Some(gas) => state.gas.charge(gas),
                        None => {
                            state.gas.unmodeled("symbolic gas for this precompile call not modeled")
                        }
                    }
                }
                _ => state.gas.unmodeled("symbolic gas for this precompile call not modeled"),
            }
        } else {
            self.charge_account(state, 1, params);
        }
        if has_value && self.operand(state, 2) != Some(U256::ZERO) {
            self.charge_value_transfer(executor, state, params);
        }
        if executor.inspector().enable_isolation && state.call_depth == 0 {
            self.isolate_call(state, op, args);
        }
    }

    /// Charges a call's value transfer, and a new account if the callee can be empty.
    ///
    /// Both costs apply only to a nonzero value, so the worst-case witness can reach them. The
    /// callee's stipend is not credited back.
    fn charge_value_transfer<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        params: &GasParams,
    ) {
        let (Ok(value), Ok(target)) = (state.stack.peek(2).cloned(), state.stack.peek(1).cloned())
        else {
            return;
        };
        let cx = &mut self.cx;
        let zero = SymExpr::zero(cx);
        let new_account = SymExpr::constant(cx, U256::from(params.new_account_cost(true, true)));
        let empty = match state.world.resolve_address(&target) {
            Some(address) => match state.world.has_code_or_nonce(cx, executor, address) {
                Ok(true) => None,
                Ok(false) => {
                    let balance = state.world.balance_word_for_address(cx, executor, address);
                    Some(SymBoolExpr::eq_word_const(cx, &balance, U256::ZERO))
                }
                Err(_) => Some(SymBoolExpr::constant(cx, true)),
            },
            // The transaction sender's nonce grows before execution, so it is never empty.
            None if target == SymExpr::var(cx, TRANSACTION_SENDER) => None,
            None => Some(SymBoolExpr::constant(cx, true)),
        };
        let mut cost = SymExpr::constant(cx, U256::from(params.transfer_value_cost()));
        if let Some(empty) = empty {
            let new_account = SymExpr::ite(cx, empty, new_account, zero.clone());
            cost = SymExpr::binop(cx, SymBinOp::Add, cost, new_account);
        }
        let nonzero = SymBoolExpr::eq_word_const(cx, &value, U256::ZERO).not(cx);
        state.gas.charge_expr(SymExpr::ite(cx, nonzero, cost, zero));
    }

    /// Applies test isolation to a call from the test frame, mirroring `InspectorStack::call`.
    fn isolate_call(&mut self, state: &mut PathState, op: u8, args: usize) {
        let target =
            state.stack.peek(1).ok().and_then(|target| state.world.resolve_address(target));
        if target == Some(CHEATCODE_ADDRESS) {
            return;
        }
        match op {
            // Isolation marks every account and slot cold before a top-level STATICCALL.
            opcode::STATICCALL => state.gas.warm = WarmSet::default(),
            opcode::CALL => {
                if target.is_none() {
                    state.gas.unmodeled("symbolic gas for an isolated call to a symbolic target");
                }
                let input =
                    match (self.operand_usize(state, args), self.operand_usize(state, args + 1)) {
                        (Some(offset), Some(len)) => state
                            .memory
                            .read_bytes(&mut self.cx, offset, len)
                            .materialize(&mut self.cx),
                        _ => {
                            state.gas.unmodeled(
                                "symbolic gas for symbolic isolated calldata not modeled",
                            );
                            Vec::new()
                        }
                    };
                // Record the call even if its gas is unmodeled: the callee's warm accounts and
                // slots belong to its own transaction and must not leak into the caller.
                let storage_writes = state.world.storage.len();
                state.gas.begin_isolated_call(state.call_depth, target, input, storage_writes);
            }
            _ => {}
        }
    }

    /// Charges the receipt gas of the root transaction and measures the whole path as the
    /// snapshot named after `function`.
    pub(super) fn measure_transaction<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        function: &Function,
    ) -> Result<(), SymbolicError> {
        self.settle_isolated_call(executor, state, TRANSACTION_DEPTH, 0);
        state.gas.measure_path(&mut self.cx, (None, function.signature()))
    }

    /// Ends a path whose loop was summarized as a successful transaction: charges the rest of
    /// the call from the loop bound and measures the transaction.
    pub(super) fn finish_summarized_path<FEN: FoundryEvmNetwork>(
        &mut self,
        input: &SymbolicRunInput<'_, FEN>,
        state: &mut PathState,
        bound: SymbolicLoopBound,
    ) -> Result<(), SymbolicError> {
        // A later iteration can undo a refund, so no refund of the transaction is certain.
        state.gas.refunds.clear();
        state
            .gas
            .charge(bound.iteration.saturating_mul(bound.iterations).saturating_add(bound.exit));
        state.gas.loops.push(bound);
        // The worst-case witness should fill every bounded array.
        for (slot, max) in self.storage_max_lengths.clone() {
            let key = SymExpr::constant(&mut self.cx, slot);
            let length =
                state.world.sload(&mut self.cx, input.executor, input.target, key, Some(slot))?;
            let full = SymBoolExpr::eq_word_const(&mut self.cx, &length, U256::from(max));
            if self.constraints_with_condition(state, full.clone()).is_ok_and(|(_, sat)| sat) {
                state.gas.witness_hints.push(full);
            }
        }
        self.measure_transaction(input.executor, state, input.function)
    }

    /// Raises `bounds` to the gas snapshots of a successful path, with a witness input for each
    /// bound that the path raises.
    pub(super) fn raise_gas_bounds(
        &mut self,
        function: &Function,
        state: &PathState,
    ) -> Result<(), SymbolicError> {
        let mut bounds = std::mem::take(&mut self.finished_gas);
        let result = self.raise_gas_bounds_in(function, state, &mut bounds);
        self.finished_gas = bounds;
        result
    }

    fn raise_gas_bounds_in(
        &mut self,
        function: &Function,
        state: &PathState,
        bounds: &mut Vec<SymbolicGasBound>,
    ) -> Result<(), SymbolicError> {
        for ((group, name), gas) in &state.gas.measured {
            let bound =
                bounds.iter().position(|bound| &bound.group == group && &bound.name == name);
            // Most paths cannot raise the bound: one query rules them out before the search.
            if let Some(bound) = bound
                && !self.gas_at_least(state, gas, bounds[bound].max.saturating_add(1))?
            {
                continue;
            }
            let max = self.max_path_gas(state, gas)?;
            if bound.is_some_and(|bound| bounds[bound].max >= max) {
                continue;
            }
            let mut witness = state.clone();
            witness.constraints.push(SymBoolExpr::cmp_word_const(
                &mut self.cx,
                SymCmpOp::Uge,
                gas,
                U256::from(max),
            ));
            let WitnessReplay { input, sender, storage, immutables } =
                self.replay_witness(function, &witness)?;
            let new = SymbolicGasBound {
                group: group.clone(),
                name: name.clone(),
                max,
                witness: input,
                sender,
                storage,
                immutables,
                loops: state.gas.loops.clone(),
            };
            match bound {
                Some(bound) => bounds[bound] = new,
                None => bounds.push(new),
            }
        }
        Ok(())
    }

    /// Checks that a successful path keeps every bounded array length of the target at most its
    /// maximum, and records a witness for each length the path can exceed.
    pub(super) fn check_max_lengths<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        target: Address,
        function: &Function,
        state: &mut PathState,
    ) -> Result<(), SymbolicError> {
        for (slot, max) in self.storage_max_lengths.clone() {
            if self.finished_length_violations.iter().any(|violation| violation.slot == slot) {
                continue;
            }
            let key = SymExpr::constant(&mut self.cx, slot);
            let length = state.world.sload(&mut self.cx, executor, target, key, Some(slot))?;
            let mut witness = state.clone();
            witness.constraints.push(SymBoolExpr::cmp_word_const(
                &mut self.cx,
                SymCmpOp::Ugt,
                &length,
                U256::from(max),
            ));
            if !self.is_sat_with_state(&witness, &witness.constraints)? {
                continue;
            }
            let WitnessReplay { input: witness, sender, storage, immutables } =
                self.replay_witness(function, &witness)?;
            self.finished_length_violations.push(SymbolicLengthViolation {
                slot,
                witness,
                sender,
                storage,
                immutables,
            });
        }
        Ok(())
    }

    /// Returns a concrete input for the path, with the sender, storage and immutables it
    /// assumes.
    fn replay_witness(
        &mut self,
        function: &Function,
        state: &PathState,
    ) -> Result<WitnessReplay, SymbolicError> {
        let (input, model) = self.materialize_hinted_root_model(function, state)?;
        let storage = state.world.replay_storage_assignments(&model)?;
        // An unconstrained sender can be any account; replay keeps the default sender.
        let sender = SymExpr::var(&mut self.cx, TRANSACTION_SENDER)
            .eval_model_if_complete(&model)?
            .map(|sender| Address::from_word(sender.into()));
        let immutables = self
            .symbolic_immutables
            .iter()
            .map(|&offset| {
                let word = SymExpr::var(&mut self.cx, &format!("immutable_{offset}"));
                Ok((offset, word.eval_model_if_complete(&model)?.unwrap_or_default()))
            })
            .collect::<Result<Vec<_>, SymbolicError>>()?;
        Ok(WitnessReplay { input, sender, storage, immutables })
    }

    /// Returns the largest value `gas` takes on the path.
    ///
    /// Starts from the value in one solver model, doubles the step until the solver rejects
    /// it, then binary searches. This needs about twice the logarithm of the distance from
    /// the model value to the maximum in solver queries.
    pub(super) fn max_path_gas(
        &mut self,
        state: &PathState,
        gas: &SymExpr,
    ) -> Result<u64, SymbolicError> {
        if let Some(gas) = gas.as_const() {
            return u64::try_from(gas)
                .map_err(|_| SymbolicError::Unsupported("symbolic gas overflow"));
        }
        let replayable_storage = state.world.replay_storage_symbols();
        let model = self.solver.model_with_replayable_storage(
            &mut self.cx,
            &state.constraints,
            &replayable_storage,
        )?;
        // `gas >= low` is feasible on the path and `gas >= high` is not.
        let mut low = gas
            .eval_model_if_complete(&model)?
            .and_then(|gas| u64::try_from(gas).ok())
            .unwrap_or_default();
        let mut step = 1;
        let mut high = loop {
            let probe = low.saturating_add(step);
            if probe >= MAX_SEARCHED_GAS {
                return Err(SymbolicError::Unsupported("symbolic gas above the searched range"));
            }
            if !self.gas_at_least(state, gas, probe)? {
                break probe;
            }
            low = probe;
            step *= 2;
        };
        while high - low > 1 {
            let mid = low + (high - low) / 2;
            if self.gas_at_least(state, gas, mid)? {
                low = mid;
            } else {
                high = mid;
            }
        }
        Ok(low)
    }

    fn gas_at_least(
        &mut self,
        state: &PathState,
        gas: &SymExpr,
        value: u64,
    ) -> Result<bool, SymbolicError> {
        let mut constraints = state.constraints.clone();
        constraints.push(SymBoolExpr::cmp_word_const(
            &mut self.cx,
            SymCmpOp::Uge,
            gas,
            U256::from(value),
        ));
        self.is_sat_with_state(state, &constraints)
    }

    /// Charges the receipt gas of an isolated call that returned to the current opcode.
    ///
    /// Intrinsic and floor gas price each symbolic calldata byte as nonzero when the path allows
    /// it, see [`Self::price_calldata`].
    pub(super) fn settle_isolated_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        call_depth: usize,
        pc: usize,
    ) {
        // Ask before taking the call: a root transaction is measured through its own record.
        let measuring = state.gas.is_measuring();
        let Some((input, execution, refunds)) = state.gas.returned_isolated_call(call_depth, pc)
        else {
            return;
        };
        let params = executor.evm_env().cfg_env.gas_params();
        // The refund cap must not exceed the real one, so it uses the cheapest calldata, with
        // every symbolic byte zero.
        let cheapest = input
            .iter()
            .map(|byte| byte.as_const().map_or(0, |byte| byte.byte(0)))
            .collect::<Vec<_>>();
        let cheapest = params.initial_tx_gas(&cheapest, false, 0, 0, 0, None);
        let input = if measuring {
            self.price_calldata(state, &input)
        } else {
            input.iter().map(|byte| byte.as_const().map_or(u8::MAX, |byte| byte.byte(0))).collect()
        };
        let gas = params.initial_tx_gas(&input, false, 0, 0, 0, None);
        let intrinsic = SymExpr::constant(&mut self.cx, U256::from(gas.initial_regular_gas));
        let floor = SymExpr::constant(&mut self.cx, U256::from(gas.floor_gas));
        let max_refund = (cheapest.initial_regular_gas + execution) / params.max_refund_quotient();
        state.gas.settle_isolated_call(
            &mut self.cx,
            intrinsic,
            floor,
            execution,
            refunds,
            max_refund,
        );
    }

    /// Returns calldata with each symbolic byte priced as nonzero if the path allows it, and as
    /// zero otherwise.
    ///
    /// A nonzero byte costs more, so this is an upper bound on the calldata cost of every input on
    /// the path. It is exact when the nonzero bytes can occur together, which is checked first.
    /// The conditions are kept as hints, so the worst-case witness picks nonzero bytes.
    fn price_calldata(&mut self, state: &mut PathState, input: &[SymExpr]) -> Vec<u8> {
        let nonzero = |cx: &mut SymCx, byte: &SymExpr| {
            SymBoolExpr::eq_word_const(cx, byte, U256::ZERO).not(cx)
        };
        let conditions = input
            .iter()
            .filter(|byte| byte.as_const().is_none())
            .map(|byte| nonzero(&mut self.cx, byte))
            .collect::<Vec<_>>();
        let all = SymBoolExpr::and(&mut self.cx, conditions);
        let all_nonzero =
            self.constraints_with_condition(state, all.clone()).is_ok_and(|(_, sat)| sat);
        if all_nonzero {
            state.gas.witness_hints.push(all);
        }
        input
            .iter()
            .map(|byte| match byte.as_const() {
                Some(byte) => byte.byte(0),
                None if all_nonzero => u8::MAX,
                None => {
                    let condition = nonzero(&mut self.cx, byte);
                    // A failed query keeps the byte nonzero, the more expensive case.
                    match self.constraints_with_condition(state, condition.clone()) {
                        Ok((_, false)) => 0,
                        Ok((_, true)) => {
                            state.gas.witness_hints.push(condition);
                            u8::MAX
                        }
                        Err(_) => u8::MAX,
                    }
                }
            })
            .collect()
    }

    /// Charges the cold-account surcharge if the account at stack `index` is not warm.
    fn charge_account(&mut self, state: &mut PathState, index: usize, params: &GasParams) {
        let cold = match state.stack.peek(index).cloned() {
            Ok(word) => match self.resolve_account(state, &word) {
                Some(address) => {
                    // An earlier access through an address expression that now resolves to
                    // `address` already warmed the account.
                    let words = state.gas.warm.account_words.iter().cloned().collect::<Vec<_>>();
                    let accessed =
                        words.iter().any(|word| self.resolve_account(state, word) == Some(address));
                    state.gas.warm.accounts.insert(address) && !accessed
                }
                None => state.gas.warm.account_words.insert(word),
            },
            Err(_) => true,
        };
        if cold {
            state.gas.charge(params.cold_account_additional_cost());
        }
    }

    /// Resolves an address word through known aliases or the path constraints.
    fn resolve_account(&mut self, state: &PathState, word: &SymExpr) -> Option<Address> {
        state.world.resolve_address(word).or_else(|| {
            state.constrained_word(&mut self.cx, word).map(|word| Address::from_word(word.into()))
        })
    }

    /// Charges a per-length cost and the memory expansion for the range at stack
    /// `offset`/`len`.
    fn charge_sized(
        &mut self,
        state: &mut PathState,
        offset: usize,
        len: usize,
        cost: impl FnOnce(usize) -> u64,
        params: &GasParams,
    ) {
        let Some(size) = self.operand_usize(state, len) else {
            state.gas.unmodeled("symbolic gas for a symbolic length not modeled");
            return;
        };
        state.gas.charge(cost(size));
        self.charge_memory(state, &[(offset, len)], params);
    }

    fn charge_memory_len(&mut self, state: &mut PathState, len: usize, params: &GasParams) {
        let Some(offset) = self.operand_usize(state, 0) else {
            state.gas.unmodeled("symbolic gas for a symbolic memory offset not modeled");
            return;
        };
        self.expand_memory(state, offset.checked_add(len), params);
    }

    /// Charges memory expansion for the ranges given as `(offset, len)` stack indices.
    fn charge_memory(
        &mut self,
        state: &mut PathState,
        ranges: &[(usize, usize)],
        params: &GasParams,
    ) {
        let mut end = Some(0usize);
        for &(offset, len) in ranges {
            match self.operand_usize(state, len) {
                Some(0) => {}
                Some(len) => {
                    let Some(offset) = self.operand_usize(state, offset) else {
                        state
                            .gas
                            .unmodeled("symbolic gas for a symbolic memory offset not modeled");
                        return;
                    };
                    end = end.zip(offset.checked_add(len)).map(|(end, range)| end.max(range));
                }
                None => {
                    state.gas.unmodeled("symbolic gas for a symbolic memory size not modeled");
                    return;
                }
            }
        }
        self.expand_memory(state, end, params);
    }

    fn expand_memory(&mut self, state: &mut PathState, end: Option<usize>, params: &GasParams) {
        let Some(end) = end.filter(|&end| end <= MAX_METERED_MEMORY) else {
            state.gas.unmodeled("symbolic gas for out-of-gas memory expansion not modeled");
            return;
        };
        let size = state.memory.size_word(&mut self.cx);
        let Some(size) = size.as_const().and_then(|size| usize::try_from(size).ok()) else {
            state.gas.unmodeled("symbolic gas for a symbolic memory size not modeled");
            return;
        };
        let (current, new) = (num_words(size), num_words(end));
        if new > current {
            state.gas.charge(params.memory_cost(new) - params.memory_cost(current));
        }
    }

    fn operand(&mut self, state: &PathState, index: usize) -> Option<U256> {
        let word = state.stack.peek(index).ok()?;
        state.constrained_word(&mut self.cx, word)
    }

    fn operand_usize(&mut self, state: &PathState, index: usize) -> Option<usize> {
        let word = state.stack.peek(index).ok()?;
        state.constrained_usize(&mut self.cx, word)
    }
}
