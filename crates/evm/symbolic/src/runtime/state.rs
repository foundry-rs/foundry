use super::*;

#[derive(Clone, Debug)]
pub(crate) struct PathState {
    pub(crate) depth: usize,
    pub(crate) call_depth: usize,
    pub(crate) origin: Address,
    pub(crate) origin_word: SymExpr,
    pub(crate) gas_price: SymExpr,
    pub(crate) ffi_enabled: bool,
    pub(crate) block: SymbolicBlock,
    pub(crate) frame: CallFrame,
    pub(crate) world: SymbolicWorld,
    pub(crate) prank: SymbolicPrank,
    pub(crate) constraints: Vec<SymBoolExpr>,
    pub(crate) next_symbol: usize,
    pub(crate) recorded_logs: Option<Vec<SymbolicLog>>,
    pub(crate) access_record: Option<AccessRecord>,
    pub(crate) root_calldata: Option<SymbolicCalldata>,
    pub(crate) loop_jumps: HashMap<usize, u32>,
    pub(crate) expected_revert: Option<ExpectedRevert>,
    pub(crate) assume_no_revert_next_call: Option<AssumeNoRevert>,
    pub(crate) expected_emit: Option<ExpectedEmit>,
    pub(crate) expected_calls: Vec<ExpectedCall>,
    pub(crate) expected_creates: Vec<ExpectedCreate>,
    pub(crate) call_mocks: Vec<CallMock>,
    pub(crate) function_mocks: Vec<FunctionMock>,
    pub(crate) persistent_accounts: HashSet<Address>,
    pub(crate) wallets: IndexSet<Address>,
    pub(crate) labels: HashMap<Address, String>,
}

impl PathState {
    pub(crate) fn new(
        address: Address,
        caller: Address,
        callvalue: U256,
        calldata: SymbolicCalldata,
        ffi_enabled: bool,
    ) -> Self {
        let constraints = calldata.constraints().to_vec();
        let call_data = calldata.call_data();
        Self {
            depth: 0,
            call_depth: 0,
            origin: caller,
            origin_word: SymExpr::constant(address_word(caller)),
            gas_price: SymExpr::zero(),
            ffi_enabled,
            block: SymbolicBlock::default(),
            frame: CallFrame::new(
                address,
                address,
                address,
                caller,
                SymExpr::constant(callvalue),
                false,
                call_data,
            ),
            world: SymbolicWorld::default(),
            prank: SymbolicPrank::default(),
            constraints,
            next_symbol: 0,
            recorded_logs: None,
            access_record: None,
            root_calldata: Some(calldata),
            loop_jumps: HashMap::default(),
            expected_revert: None,
            assume_no_revert_next_call: None,
            expected_emit: None,
            expected_calls: Vec::new(),
            expected_creates: Vec::new(),
            call_mocks: Vec::new(),
            function_mocks: Vec::new(),
            persistent_accounts: HashSet::default(),
            wallets: IndexSet::default(),
            labels: HashMap::default(),
        }
    }

    pub(crate) fn empty(address: Address, caller: Address, ffi_enabled: bool) -> Self {
        Self {
            depth: 0,
            call_depth: 0,
            origin: caller,
            origin_word: SymExpr::constant(address_word(caller)),
            gas_price: SymExpr::zero(),
            ffi_enabled,
            block: SymbolicBlock::default(),
            frame: CallFrame::new(
                address,
                address,
                address,
                caller,
                SymExpr::zero(),
                false,
                SymCalldata::new(Vec::new()),
            ),
            world: SymbolicWorld::default(),
            prank: SymbolicPrank::default(),
            constraints: Vec::new(),
            next_symbol: 0,
            recorded_logs: None,
            access_record: None,
            root_calldata: None,
            loop_jumps: HashMap::default(),
            expected_revert: None,
            assume_no_revert_next_call: None,
            expected_emit: None,
            expected_calls: Vec::new(),
            expected_creates: Vec::new(),
            call_mocks: Vec::new(),
            function_mocks: Vec::new(),
            persistent_accounts: HashSet::default(),
            wallets: IndexSet::default(),
            labels: HashMap::default(),
        }
    }

    pub(crate) fn apply_executor_env<FEN: FoundryEvmNetwork>(&mut self, executor: &Executor<FEN>) {
        self.block = SymbolicBlock::from_executor(executor);
        let gas_price = executor
            .inspector()
            .cheatcodes
            .as_ref()
            .and_then(|cheats| cheats.gas_price)
            .unwrap_or_else(|| executor.tx_env().gas_price());
        self.gas_price = SymExpr::constant(U256::from(gas_price));
    }

    pub(crate) fn child(&self, frame: CallFrame) -> Self {
        let mut child = self.clone();
        child.call_depth += 1;
        child.frame = frame;
        child.loop_jumps = HashMap::default();
        child
    }

    pub(crate) fn copy_call_output_offset(
        &mut self,
        dest: SymExpr,
        size: &BoundedCopySize,
    ) -> Result<(), SymbolicError> {
        let CallFrame { memory, return_data, .. } = &mut self.frame;
        memory.copy_call_output_offset(dest, size, return_data)
    }

    pub(crate) fn copy_calldata_to_offset(
        &mut self,
        dest: SymExpr,
        offset: SymExpr,
        size: usize,
    ) -> Result<(), SymbolicError> {
        let CallFrame { memory, calldata, .. } = &mut self.frame;
        memory.copy_calldata_to_offset(dest, offset, size, calldata)
    }

    pub(crate) fn copy_calldata_symbolic_size(
        &mut self,
        dest: SymExpr,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Result<(), SymbolicError> {
        let CallFrame { memory, calldata, .. } = &mut self.frame;
        memory.copy_calldata_symbolic_size(dest, offset, size, max_size, calldata)
    }

    pub(crate) fn copy_return_data_to_offset(
        &mut self,
        dest: SymExpr,
        offset: SymExpr,
        size: usize,
    ) -> Result<(), SymbolicError> {
        let CallFrame { memory, return_data, .. } = &mut self.frame;
        memory.copy_return_data_to_offset(dest, offset, size, return_data)
    }

    pub(crate) fn copy_return_data_symbolic_size(
        &mut self,
        dest: SymExpr,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Result<(), SymbolicError> {
        let CallFrame { memory, return_data, .. } = &mut self.frame;
        memory.copy_return_data_symbolic_size(dest, offset, size, max_size, return_data)
    }

    pub(crate) fn constrained_usize(&self, word: &SymExpr) -> Option<usize> {
        self.constrained_usize_checked(word).and_then(Result::ok)
    }

    pub(crate) fn constrained_usize_checked(&self, word: &SymExpr) -> Option<Result<usize, U256>> {
        self.constrained_word(word).map(|value| usize::try_from(value).map_err(|_| value))
    }

    pub(crate) fn upper_bound_usize(&self, word: &SymExpr) -> Option<usize> {
        self.constrained_usize(word).or_else(|| {
            word.as_const()
                .and_then(|value| usize::try_from(value).ok())
                .or_else(|| self.expr_upper_bound_usize(word))
        })
    }

    pub(crate) fn constrained_word(&self, word: &SymExpr) -> Option<U256> {
        word.as_const().or_else(|| {
            let expr = word;
            self.constraints
                .iter()
                .find_map(|constraint| {
                    constraint.forces_expr_const_with_context(expr, &self.constraints)
                })
                .or_else(|| self.constrained_expr_value(expr))
        })
    }

    pub(crate) fn constrained_expr_value(&self, expr: &SymExpr) -> Option<U256> {
        if let Some(value) = expr.eval() {
            return Some(value);
        }
        if let Some(value) = expr.known_word() {
            return Some(value);
        }

        let mut vars = SymbolicVars::default();
        collect_eval_vars(expr, &mut vars);
        let mut model = SymbolicModel::default();
        for var in vars {
            let var_expr = SymExpr::var_symbol(var);
            let value = self.constraints.iter().find_map(|constraint| {
                constraint.forces_expr_const_with_context(&var_expr, &self.constraints)
            })?;
            model.insert(var, value);
        }

        expr.eval_model(&model).ok()
    }

    pub(crate) fn expr_upper_bound_usize(&self, expr: &SymExpr) -> Option<usize> {
        if let Some(value) = expr.eval() {
            return usize::try_from(value).ok();
        }
        if let Some(value) = expr.known_word() {
            return usize::try_from(value).ok();
        }

        let constraint_bound = self.constraint_upper_bound_usize(expr);
        let structural_bound = match expr.kind() {
            SymExprKind::Const(value) => usize::try_from(*value).ok(),
            SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. } => None,
            SymExprKind::Not(_) => None,
            SymExprKind::AddMod { modulus, .. } | SymExprKind::MulMod { modulus, .. } => {
                match modulus.eval() {
                    Some(modulus) if modulus.is_zero() => Some(0),
                    Some(modulus) => usize::try_from(modulus - U256::from(1)).ok(),
                    None => {
                        self.expr_upper_bound_usize(modulus).and_then(|bound| bound.checked_sub(1))
                    }
                }
            }
            SymExprKind::Ite(_, left, right) => {
                Some(self.expr_upper_bound_usize(left)?.max(self.expr_upper_bound_usize(right)?))
            }
            SymExprKind::Op(op, left, right) => match op {
                SymExprOp::Add => self
                    .expr_upper_bound_usize(left)?
                    .checked_add(self.expr_upper_bound_usize(right)?),
                SymExprOp::Mul => self
                    .expr_upper_bound_usize(left)?
                    .checked_mul(self.expr_upper_bound_usize(right)?),
                SymExprOp::UDiv => {
                    let left = self.expr_upper_bound_usize(left)?;
                    match right.eval()? {
                        divisor if divisor.is_zero() => Some(0),
                        divisor => Some(left / usize::try_from(divisor).ok()?),
                    }
                }
                SymExprOp::URem => match right.eval() {
                    Some(divisor) if divisor.is_zero() => Some(0),
                    Some(divisor) => usize::try_from(divisor - U256::from(1)).ok(),
                    None => self.expr_upper_bound_usize(left),
                },
                SymExprOp::And => right
                    .eval()
                    .and_then(|value| usize::try_from(value).ok())
                    .or_else(|| left.eval().and_then(|value| usize::try_from(value).ok()))
                    .map(|mask| {
                        self.expr_upper_bound_usize(left)
                            .or_else(|| self.expr_upper_bound_usize(right))
                            .map_or(mask, |bound| bound.min(mask))
                    }),
                SymExprOp::Shr => {
                    let left = self.expr_upper_bound_usize(left)?;
                    let shift = usize::try_from(right.eval()?).ok()?;
                    Some(if shift >= usize::BITS as usize { 0 } else { left >> shift })
                }
                SymExprOp::Sub
                | SymExprOp::SDiv
                | SymExprOp::SRem
                | SymExprOp::Or
                | SymExprOp::Xor
                | SymExprOp::Shl
                | SymExprOp::Sar => None,
            },
        };

        match (constraint_bound, structural_bound) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(bound), None) | (None, Some(bound)) => Some(bound),
            (None, None) => None,
        }
    }

    pub(crate) fn constraint_upper_bound_usize(&self, expr: &SymExpr) -> Option<usize> {
        let mut bound: Option<usize> = None;
        for constraint in &self.constraints {
            if let Some(candidate) = constraint.upper_bound_usize(expr) {
                bound = Some(bound.map_or(candidate, |bound| bound.min(candidate)));
            }
        }
        bound
    }

    pub(crate) fn expect_constrained_usize(
        &self,
        word: SymExpr,
        reason: &'static str,
    ) -> Result<usize, SymbolicError> {
        self.constrained_usize(&word).ok_or(SymbolicError::Unsupported(reason))
    }

    pub(crate) fn expect_constrained_word(
        &self,
        word: SymExpr,
        reason: &'static str,
    ) -> Result<U256, SymbolicError> {
        self.constrained_word(&word).ok_or(SymbolicError::Unsupported(reason))
    }

    pub(crate) fn bin_word(&mut self, op: SymExprOp) -> Result<StepOutcome, SymbolicError> {
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(SymExpr::op(op, a, b))?;
        Ok(StepOutcome::Continue)
    }

    pub(crate) fn bin_word_div_zero_guard(
        &mut self,
        op: SymExprOp,
    ) -> Result<StepOutcome, SymbolicError> {
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(SymExpr::ite(
            SymBoolExpr::eq(b.clone(), SymExpr::constant(U256::ZERO)),
            SymExpr::constant(U256::ZERO),
            SymExpr::op(op, a, b),
        ))?;
        Ok(StepOutcome::Continue)
    }

    pub(crate) fn cmp_word(&mut self, op: SymBoolExprOp) -> Result<StepOutcome, SymbolicError> {
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(SymExpr::from_bool(SymBoolExpr::cmp(op, a, b)))?;
        Ok(StepOutcome::Continue)
    }

    pub(crate) fn shift_word(&mut self, kind: ShiftKind) -> Result<StepOutcome, SymbolicError> {
        let shift = self.stack.pop()?;
        let value = self.stack.pop()?;
        let result = if let (Some(value), Some(shift)) = (value.as_const(), shift.as_const()) {
            let result = if shift >= U256::from(256) {
                if matches!(kind, ShiftKind::Sar) && ((value >> 255) == U256::from(1)) {
                    U256::MAX
                } else {
                    U256::ZERO
                }
            } else {
                let shift = usize::try_from(shift).expect("checked word shift");
                match kind {
                    ShiftKind::Shl => value << shift,
                    ShiftKind::Shr => value >> shift,
                    ShiftKind::Sar => sar(value, shift),
                }
            };
            SymExpr::constant(result)
        } else {
            let expr = match kind {
                ShiftKind::Shl => SymExpr::op(SymExprOp::Shl, value, shift),
                ShiftKind::Shr => SymExpr::op(SymExprOp::Shr, value, shift),
                ShiftKind::Sar => SymExpr::op(SymExprOp::Sar, value, shift),
            };
            expr.known_word().map(SymExpr::constant).unwrap_or(expr)
        };
        self.stack.push(result)?;
        Ok(StepOutcome::Continue)
    }

    pub(crate) fn exp_word(&mut self) -> Result<StepOutcome, SymbolicError> {
        let base = self.stack.pop()?;
        let exponent = self.stack.pop()?;
        let result = if let Some(exponent) = self.constrained_word(&exponent) {
            if let Some(base_value) = base.as_const() {
                SymExpr::constant(pow_mod(base_value, exponent))
            } else if exponent <= U256::from(SYMBOLIC_EXP_CONCRETE_EXPONENT_LIMIT) {
                exp_expr_for_concrete_exponent(
                    base,
                    usize::try_from(exponent).expect("checked symbolic exponent"),
                )
            } else {
                return Err(SymbolicError::Unsupported("symbolic EXP base"));
            }
        } else {
            let exponent_limit = if base.as_const().is_some() {
                CONCRETE_BASE_SYMBOLIC_EXPONENT_LIMIT
            } else {
                SYMBOLIC_EXP_CONCRETE_EXPONENT_LIMIT
            };
            let max_exponent = self
                .upper_bound_usize(&exponent)
                .filter(|exponent| *exponent <= exponent_limit as usize)
                .ok_or(SymbolicError::Unsupported("symbolic EXP exponent"))?;
            let mut expr = SymExpr::constant(U256::ZERO);
            for candidate in (0..=max_exponent).rev() {
                expr = SymExpr::ite(
                    SymBoolExpr::eq(exponent.clone(), SymExpr::constant(U256::from(candidate))),
                    exp_expr_for_concrete_exponent(base.clone(), candidate),
                    expr,
                );
            }
            expr
        };
        self.stack.push(result)?;
        Ok(StepOutcome::Continue)
    }

    pub(crate) fn balance<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> SymExpr {
        self.world.balance_word_for_address(executor, address)
    }

    pub(crate) fn balance_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        self.world.balance_word(executor, word)
    }

    pub(crate) fn extcode_size_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        self.world.extcode_size_word(executor, word)
    }

    pub(crate) fn extcode_hash_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        self.world.extcode_hash_word(executor, word)
    }

    pub(crate) fn extcode_bytes_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymExpr,
        offset: SymExpr,
        size: usize,
    ) -> Result<Vec<SymExpr>, SymbolicError> {
        self.world.extcode_bytes_word(executor, word, offset, size)
    }

    pub(crate) fn pop_address_word_or_symbolic_slot(
        &mut self,
    ) -> Result<(SymExpr, Address), SymbolicError> {
        let word = self.stack.pop()?;
        let address = self.address_or_symbolic_slot(word.clone());
        Ok((word, address))
    }

    pub(crate) fn address_or_symbolic_slot(&mut self, word: SymExpr) -> Address {
        if let Some(value) = self.constrained_word(&word) {
            return word_to_address(value);
        }
        self.world.resolve_address(&word).unwrap_or_else(|| self.world.symbolic_address_slot(word))
    }

    pub(crate) fn fresh_word(&mut self, prefix: &'static str) -> SymExpr {
        let id = self.next_symbol;
        self.next_symbol += 1;
        SymExpr::var(&format!("{prefix}_{id}"))
    }

    pub(crate) fn fresh_gasleft(&mut self) -> SymExpr {
        let id = self.next_symbol;
        self.next_symbol += 1;
        SymExpr::gas_left(id)
    }

    pub(crate) fn fresh_bounded_uint(&mut self, bits: U256) -> SymExpr {
        let value = self.fresh_word("symbolic");
        if bits < U256::from(256) {
            let upper = if bits.is_zero() {
                U256::ZERO
            } else {
                U256::from(1) << usize::try_from(bits).expect("checked bit width")
            };
            self.constraints.push(SymBoolExpr::cmp_word_const(SymBoolExprOp::Ult, &value, upper));
        }
        value
    }

    pub(crate) fn fresh_bytes(&mut self, len: usize) -> Vec<SymExpr> {
        (0..len).map(|_| self.fresh_bounded_uint(U256::from(8))).collect()
    }

    pub(crate) fn fresh_printable_ascii_bytes(&mut self, len: usize) -> Vec<SymExpr> {
        (0..len)
            .map(|_| {
                let byte = self.fresh_bounded_uint(U256::from(8));
                self.constraints.push(SymBoolExpr::cmp_word_const(
                    SymBoolExprOp::Uge,
                    &byte,
                    U256::from(0x20),
                ));
                self.constraints.push(SymBoolExpr::cmp_word_const(
                    SymBoolExprOp::Ule,
                    &byte,
                    U256::from(0x7e),
                ));
                byte
            })
            .collect()
    }

    pub(crate) fn fresh_bounded_int(&mut self, bits: U256) -> SymExpr {
        let value = self.fresh_word("symbolic");
        if bits.is_zero() {
            self.constraints.push(SymBoolExpr::eq_word_const(&value, U256::ZERO));
        } else if bits < U256::from(256) {
            let magnitude =
                U256::from(1) << (usize::try_from(bits).expect("checked bit width") - 1);
            self.constraints.push(SymBoolExpr::or(vec![
                SymBoolExpr::cmp_word_const(SymBoolExprOp::Ult, &value, magnitude),
                SymBoolExpr::cmp_word_const(
                    SymBoolExprOp::Uge,
                    &value,
                    U256::ZERO.wrapping_sub(magnitude),
                ),
            ]));
        }
        value
    }

    pub(crate) fn prank_for_next_call(&mut self) -> (Address, SymExpr, Option<(Address, SymExpr)>) {
        if let Some((caller, caller_word)) = self.prank.next_caller.take() {
            (caller, caller_word, self.prank.next_origin.take())
        } else {
            match self.prank.persistent_caller.clone() {
                Some((caller, caller_word)) => {
                    (caller, caller_word, self.prank.persistent_origin.clone())
                }
                None => {
                    (self.address, self.address_word.clone(), self.prank.persistent_origin.clone())
                }
            }
        }
    }

    pub(crate) fn read_callers_words(&self) -> Vec<SymExpr> {
        let (mode, caller, origin) = if let Some((_, caller_word)) = self.prank.next_caller.as_ref()
        {
            (
                U256::from(3),
                caller_word.clone(),
                self.prank
                    .next_origin
                    .as_ref()
                    .map(|(_, origin_word)| origin_word.clone())
                    .unwrap_or_else(|| self.origin_word.clone()),
            )
        } else if let Some((_, caller_word)) = self.prank.persistent_caller.as_ref() {
            (
                U256::from(4),
                caller_word.clone(),
                self.prank
                    .persistent_origin
                    .as_ref()
                    .map(|(_, origin_word)| origin_word.clone())
                    .unwrap_or_else(|| self.origin_word.clone()),
            )
        } else {
            (U256::ZERO, self.caller_word.clone(), self.origin_word.clone())
        };
        vec![SymExpr::constant(mode), caller, origin]
    }

    pub(crate) fn record_log(&mut self, log: SymbolicLog) {
        if let Some(logs) = &mut self.recorded_logs {
            logs.push(log);
        }
    }

    pub(crate) fn record_sload(&mut self, address: Address, slot: SymExpr) {
        if let Some(record) = &mut self.access_record {
            record.read(address, slot);
        }
    }

    pub(crate) fn record_sstore(&mut self, address: Address, slot: SymExpr) {
        if let Some(record) = &mut self.access_record {
            record.write(address, slot);
        }
    }

    pub(crate) fn expectations_satisfied(&self) -> bool {
        self.expected_revert.is_none()
            && self.expected_emit.as_ref().is_none_or(ExpectedEmit::is_satisfied)
            && self.expected_calls.iter().all(ExpectedCall::is_satisfied)
            && self.expected_creates.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SymbolicLog {
    topics: Arc<[SymExpr]>,
    data_len: SymExpr,
    data: Arc<[SymExpr]>,
    emitter: Address,
}

impl SymbolicLog {
    pub(crate) fn new(
        topics: Vec<SymExpr>,
        data_len: SymExpr,
        data: Vec<SymExpr>,
        emitter: Address,
    ) -> Self {
        Self { topics: topics.into(), data_len, data: data.into(), emitter }
    }

    pub(crate) fn into_parts(self) -> (Arc<[SymExpr]>, SymExpr, Arc<[SymExpr]>, Address) {
        (self.topics, self.data_len, self.data, self.emitter)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AccessRecord {
    reads: HashMap<Address, Vec<SymExpr>>,
    writes: HashMap<Address, Vec<SymExpr>>,
}

impl AccessRecord {
    pub(crate) fn read(&mut self, address: Address, slot: SymExpr) {
        Self::push_unique_slot(self.reads.entry(address).or_default(), slot);
    }

    pub(crate) fn write(&mut self, address: Address, slot: SymExpr) {
        Self::push_unique_slot(self.writes.entry(address).or_default(), slot);
    }

    pub(crate) fn addresses(&self) -> Vec<Address> {
        let mut addresses = HashSet::<Address>::default();
        addresses.extend(self.reads.keys().copied());
        addresses.extend(self.writes.keys().copied());
        let mut addresses = addresses.into_iter().collect::<Vec<_>>();
        addresses.sort_unstable();
        addresses
    }

    pub(crate) fn read_slots(&self, address: Address) -> Vec<SymExpr> {
        self.reads.get(&address).cloned().unwrap_or_default()
    }

    pub(crate) fn write_slots(&self, address: Address) -> Vec<SymExpr> {
        self.writes.get(&address).cloned().unwrap_or_default()
    }

    fn push_unique_slot(slots: &mut Vec<SymExpr>, slot: SymExpr) {
        if !slots.iter().any(|existing| existing == &slot) {
            slots.push(slot);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedRevert {
    data: ExpectedRevertData,
    reverter: Option<SymExpr>,
    remaining: u64,
}

impl ExpectedRevert {
    pub(crate) fn new(data: ExpectedRevertData, reverter: Option<SymExpr>, remaining: u64) -> Self {
        Self { data, reverter, remaining: remaining.max(1) }
    }

    pub(crate) const fn consume_one(&mut self) -> bool {
        self.remaining = self.remaining.saturating_sub(1);
        self.remaining == 0
    }

    pub(crate) fn match_condition(
        &self,
        reverter: Address,
        return_data: &SymReturnData,
    ) -> Option<SymBoolExpr> {
        let mut conditions = Vec::new();
        if let Some(expected_reverter) = &self.reverter {
            conditions.push(address_match_condition(expected_reverter, reverter));
        }
        match &self.data {
            ExpectedRevertData::Any => {}
            ExpectedRevertData::Prefix(prefix) => {
                if return_data.len() < prefix.len() {
                    return None;
                }
                conditions.push(SymBoolExpr::cmp(
                    SymBoolExprOp::Uge,
                    return_data.len_expr(),
                    SymExpr::constant(U256::from(prefix.len())),
                ));
                conditions.extend(prefix.iter().enumerate().map(|(offset, expected)| {
                    let actual = return_data.byte(offset);
                    SymBoolExpr::eq_words(&actual, expected)
                }));
            }
            ExpectedRevertData::Exact(data) => {
                if return_data.len() < data.len() {
                    return None;
                }
                conditions.push(SymBoolExpr::eq(
                    return_data.len_expr(),
                    SymExpr::constant(U256::from(data.len())),
                ));
                conditions.extend(data.iter().enumerate().map(|(offset, expected)| {
                    let actual = return_data.byte(offset);
                    SymBoolExpr::eq_words(&actual, expected)
                }));
            }
        }
        Some(SymBoolExpr::and(conditions))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExpectedRevertData {
    Any,
    Prefix(Arc<[SymExpr]>),
    Exact(Arc<[SymExpr]>),
}

impl ExpectedRevertData {
    pub(crate) fn prefix(data: Vec<SymExpr>) -> Self {
        Self::Prefix(data.into())
    }

    pub(crate) fn exact(data: Vec<SymExpr>) -> Self {
        Self::Exact(data.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AssumeNoRevert {
    Any,
    Filtered(Vec<ExpectedRevert>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedCall {
    callee: SymExpr,
    value: Option<U256>,
    gas: Option<u64>,
    min_gas: Option<u64>,
    data: Arc<[SymExpr]>,
    expected: u64,
    observed: u64,
    exact: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedCreate {
    bytecode: Vec<u8>,
    deployer: SymExpr,
    kind: CreateKind,
}

impl ExpectedCreate {
    pub(crate) const fn new(bytecode: Vec<u8>, deployer: SymExpr, kind: CreateKind) -> Self {
        Self { bytecode, deployer, kind }
    }

    pub(crate) fn match_condition(
        &self,
        deployer: Address,
        kind: CreateKind,
        bytecode: &[u8],
    ) -> Option<SymBoolExpr> {
        (self.kind == kind && self.bytecode == bytecode)
            .then(|| address_match_condition(&self.deployer, deployer))
    }
}

impl ExpectedCall {
    pub(crate) fn new(
        callee: SymExpr,
        value: Option<U256>,
        gas: Option<u64>,
        min_gas: Option<u64>,
        data: Vec<SymExpr>,
        count: Option<u64>,
    ) -> Self {
        let (gas, min_gas) = if value.is_some_and(|value| !value.is_zero()) {
            (
                gas.map(|gas| gas.saturating_add(CALL_VALUE_STIPEND)),
                min_gas.map(|gas| gas.saturating_add(CALL_VALUE_STIPEND)),
            )
        } else {
            (gas, min_gas)
        };
        Self {
            callee,
            value,
            gas,
            min_gas,
            data: data.into(),
            expected: count.unwrap_or(1).max(1),
            observed: 0,
            exact: count.is_some(),
        }
    }

    pub(crate) const fn value(&self) -> Option<U256> {
        self.value
    }

    pub(crate) fn match_condition(
        &self,
        callee: Address,
        value: Option<U256>,
        gas: &SymExpr,
        calldata: &[SymExpr],
    ) -> Result<Option<SymBoolExpr>, SymbolicError> {
        if !self.static_parts_match(value, gas)? {
            return Ok(None);
        }
        let Some(data_condition) =
            calldata_prefix_condition(calldata, &self.data, "symbolic expected call calldata")?
        else {
            return Ok(None);
        };
        Ok(Some(SymBoolExpr::and(vec![
            address_match_condition(&self.callee, callee),
            data_condition,
        ])))
    }

    fn static_parts_match(
        &self,
        value: Option<U256>,
        gas: &SymExpr,
    ) -> Result<bool, SymbolicError> {
        Ok(self.value.is_none_or(|expected| value.is_some_and(|value| expected == value))
            && self.gas_matches(gas, value)?)
    }

    fn gas_matches(&self, gas: &SymExpr, value: Option<U256>) -> Result<bool, SymbolicError> {
        if self.gas.is_none() && self.min_gas.is_none() {
            return Ok(true);
        }
        let mut gas = gas.clone().into_concrete("symbolic expected call gas")?;
        if value.is_some_and(|value| !value.is_zero()) {
            gas = gas.saturating_add(U256::from(CALL_VALUE_STIPEND));
        }
        Ok(self.gas.is_none_or(|expected| gas == U256::from(expected))
            && self.min_gas.is_none_or(|expected| gas >= U256::from(expected)))
    }

    pub(crate) const fn observe(&mut self) -> bool {
        if self.exact && self.observed >= self.expected {
            return false;
        }
        self.observed = self.observed.saturating_add(1);
        true
    }

    pub(crate) const fn is_satisfied(&self) -> bool {
        if self.exact { self.observed == self.expected } else { self.observed >= self.expected }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CallMock {
    callee: SymExpr,
    value: Option<U256>,
    data: Arc<[SymExpr]>,
    returns: Vec<SymReturnData>,
    reverts: bool,
    calls: usize,
}

impl CallMock {
    pub(crate) fn new(
        callee: SymExpr,
        value: Option<U256>,
        data: Vec<SymExpr>,
        returns: Vec<SymReturnData>,
        reverts: bool,
    ) -> Self {
        Self { callee, value, data: data.into(), returns, reverts, calls: 0 }
    }

    pub(crate) const fn value(&self) -> Option<U256> {
        self.value
    }

    pub(crate) fn specificity(&self) -> (usize, bool) {
        (self.data.len(), self.value.is_some())
    }

    pub(crate) fn match_condition(
        &self,
        callee: Address,
        value: Option<U256>,
        calldata: &[SymExpr],
    ) -> Result<Option<SymBoolExpr>, SymbolicError> {
        if !self.static_parts_match(value) {
            return Ok(None);
        }
        let Some(data_condition) =
            calldata_prefix_condition(calldata, &self.data, "symbolic mocked call calldata")?
        else {
            return Ok(None);
        };
        Ok(Some(SymBoolExpr::and(vec![
            address_match_condition(&self.callee, callee),
            data_condition,
        ])))
    }

    fn static_parts_match(&self, value: Option<U256>) -> bool {
        self.value.is_none_or(|expected| value.is_some_and(|value| expected == value))
    }

    pub(crate) fn next_outcome(&mut self) -> CallMockOutcome {
        let idx = self.calls.min(self.returns.len().saturating_sub(1));
        self.calls = self.calls.saturating_add(1);
        CallMockOutcome {
            return_data: self.returns.get(idx).cloned().unwrap_or_default(),
            reverts: self.reverts,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CallMockOutcome {
    return_data: SymReturnData,
    reverts: bool,
}

impl CallMockOutcome {
    pub(crate) fn into_parts(self) -> (SymReturnData, bool) {
        (self.return_data, self.reverts)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FunctionMock {
    callee: SymExpr,
    target: Address,
    data: Arc<[SymExpr]>,
}

impl FunctionMock {
    pub(crate) fn new(callee: SymExpr, target: Address, data: Vec<SymExpr>) -> Self {
        Self { callee, target, data: data.into() }
    }

    pub(crate) fn matches_definition(&self, callee: &SymExpr, data: &[SymExpr]) -> bool {
        self.callee == *callee && self.data.as_ref() == data
    }

    pub(crate) const fn set_target(&mut self, target: Address) {
        self.target = target;
    }

    pub(crate) fn calldata_len(&self) -> usize {
        self.data.len()
    }

    pub(crate) const fn target(&self) -> Address {
        self.target
    }

    pub(crate) fn match_condition(
        &self,
        callee: Address,
        calldata: &[SymExpr],
        reason: &'static str,
    ) -> Result<Option<SymBoolExpr>, SymbolicError> {
        let Some(data_condition) = calldata_prefix_condition(calldata, &self.data, reason)? else {
            return Ok(None);
        };
        Ok(Some(SymBoolExpr::and(vec![
            address_match_condition(&self.callee, callee),
            data_condition,
        ])))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedEmit {
    checks: ExpectedEmitChecks,
    emitter: Option<SymExpr>,
    remaining: u64,
    template: Option<SymbolicLog>,
}

impl ExpectedEmit {
    pub(crate) fn new(
        checks: ExpectedEmitChecks,
        emitter: Option<SymExpr>,
        remaining: u64,
    ) -> Self {
        Self { checks, emitter, remaining: remaining.max(1), template: None }
    }

    pub(crate) const fn is_satisfied(&self) -> bool {
        self.template.is_none() && self.remaining == 0
    }

    pub(crate) const fn template(&self) -> Option<&SymbolicLog> {
        self.template.as_ref()
    }

    pub(crate) fn set_template(&mut self, log: SymbolicLog) {
        self.template = Some(log);
    }

    pub(crate) fn consume_one(&mut self) -> bool {
        self.remaining = self.remaining.saturating_sub(1);
        if self.remaining == 0 {
            self.template = None;
            true
        } else {
            false
        }
    }

    pub(crate) fn match_condition(
        &self,
        template: &SymbolicLog,
        actual: &SymbolicLog,
    ) -> Option<SymBoolExpr> {
        let mut conditions = Vec::new();
        if let Some(expected_emitter) = &self.emitter {
            conditions.push(address_match_condition(expected_emitter, actual.emitter));
        }
        for idx in 0..self.checks.topics.len() {
            if !self.checks.topics[idx] {
                continue;
            }
            match (template.topics.get(idx), actual.topics.get(idx)) {
                (Some(left), Some(right)) => {
                    conditions.push(SymBoolExpr::eq_words(left, right));
                }
                (None, None) => {}
                _ => return None,
            }
        }

        if self.checks.data {
            conditions.push(SymBoolExpr::eq_words(&template.data_len, &actual.data_len));
            if template.data.len() != actual.data.len() {
                return None;
            }
            conditions.extend(
                template
                    .data
                    .iter()
                    .zip(actual.data.iter())
                    .map(|(left, right)| SymBoolExpr::eq_words(left, right)),
            );
        }

        Some(SymBoolExpr::and(conditions))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedEmitChecks {
    topics: [bool; 4],
    data: bool,
}

impl ExpectedEmitChecks {
    pub(crate) const fn default_non_anonymous() -> Self {
        Self { topics: [true, true, true, true], data: true }
    }

    pub(crate) const fn default_anonymous() -> Self {
        Self { topics: [true, true, true, true], data: true }
    }

    pub(crate) fn from_non_anonymous_args(
        memory: &SymMemory,
        args_offset: usize,
    ) -> Result<Self, SymbolicError> {
        Ok(Self {
            topics: [
                true,
                read_abi_bool_arg(memory, args_offset, 0, "symbolic vm.expectEmit")?,
                read_abi_bool_arg(memory, args_offset, 1, "symbolic vm.expectEmit")?,
                read_abi_bool_arg(memory, args_offset, 2, "symbolic vm.expectEmit")?,
            ],
            data: read_abi_bool_arg(memory, args_offset, 3, "symbolic vm.expectEmit")?,
        })
    }

    pub(crate) fn from_anonymous_args(
        memory: &SymMemory,
        args_offset: usize,
    ) -> Result<Self, SymbolicError> {
        Ok(Self {
            topics: [
                read_abi_bool_arg(memory, args_offset, 0, "symbolic vm.expectEmitAnonymous")?,
                read_abi_bool_arg(memory, args_offset, 1, "symbolic vm.expectEmitAnonymous")?,
                read_abi_bool_arg(memory, args_offset, 2, "symbolic vm.expectEmitAnonymous")?,
                read_abi_bool_arg(memory, args_offset, 3, "symbolic vm.expectEmitAnonymous")?,
            ],
            data: read_abi_bool_arg(memory, args_offset, 4, "symbolic vm.expectEmitAnonymous")?,
        })
    }
}

impl Deref for PathState {
    type Target = CallFrame;

    fn deref(&self) -> &Self::Target {
        &self.frame
    }
}

impl DerefMut for PathState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.frame
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CallFrame {
    pub(crate) pc: usize,
    pub(crate) address: Address,
    pub(crate) address_word: SymExpr,
    #[allow(dead_code)]
    pub(crate) code_address: Address,
    pub(crate) storage_address: Address,
    pub(crate) caller: Address,
    pub(crate) caller_word: SymExpr,
    pub(crate) callvalue: SymExpr,
    pub(crate) is_static: bool,
    pub(crate) calldata: SymCalldata,
    pub(crate) stack: SymStack,
    pub(crate) memory: SymMemory,
    pub(crate) return_data: SymReturnData,
}

impl CallFrame {
    pub(crate) fn new(
        address: Address,
        code_address: Address,
        storage_address: Address,
        caller: Address,
        callvalue: SymExpr,
        is_static: bool,
        calldata: SymCalldata,
    ) -> Self {
        Self {
            pc: 0,
            address,
            address_word: SymExpr::constant(address_word(address)),
            code_address,
            storage_address,
            caller,
            caller_word: SymExpr::constant(address_word(caller)),
            callvalue,
            is_static,
            calldata,
            stack: SymStack::default(),
            memory: SymMemory::default(),
            return_data: SymReturnData::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ExternalCallOutcome {
    pub(crate) status: TopLevelCallStatus,
    pub(crate) return_data: SymReturnData,
    pub(crate) state: PathState,
}

#[derive(Clone, Debug)]
pub(crate) struct SequencePath {
    pub(crate) state: PathState,
    pub(crate) steps: Vec<SequenceStepTemplate>,
}

#[derive(Clone, Debug)]
pub(crate) struct SequenceStepTemplate {
    pub(crate) sender: Address,
    pub(crate) address: Address,
    pub(crate) contract_name: Option<String>,
    pub(crate) function: Function,
    pub(crate) calldata: SymbolicCalldata,
}

#[derive(Clone, Debug)]
pub(crate) struct InvariantCheckOutcome {
    pub(crate) failed: bool,
    pub(crate) state: PathState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TopLevelCallStatus {
    Success,
    Revert,
    Failure,
}

#[derive(Clone, Debug)]
pub(crate) struct TopLevelCallOutcome {
    pub(crate) status: TopLevelCallStatus,
    pub(crate) return_data: SymReturnData,
    pub(crate) state: PathState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SymbolicPrank {
    next_caller: Option<(Address, SymExpr)>,
    next_origin: Option<(Address, SymExpr)>,
    persistent_caller: Option<(Address, SymExpr)>,
    persistent_origin: Option<(Address, SymExpr)>,
}

impl SymbolicPrank {
    pub(crate) fn set_next(
        &mut self,
        caller: (Address, SymExpr),
        origin: Option<(Address, SymExpr)>,
    ) {
        self.next_caller = Some(caller);
        self.next_origin = origin;
    }

    pub(crate) fn set_persistent(
        &mut self,
        caller: (Address, SymExpr),
        origin: Option<(Address, SymExpr)>,
    ) {
        self.persistent_caller = Some(caller);
        self.persistent_origin = origin;
    }

    pub(crate) const fn has_active(&self) -> bool {
        self.next_caller.is_some()
            || self.next_origin.is_some()
            || self.persistent_caller.is_some()
            || self.persistent_origin.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StorageWrite {
    address: Address,
    key: SymExpr,
    value: SymExpr,
}

impl StorageWrite {
    pub(crate) const fn new(address: Address, key: SymExpr, value: SymExpr) -> Self {
        Self { address, key, value }
    }

    pub(crate) const fn address(&self) -> Address {
        self.address
    }

    #[cfg(test)]
    pub(crate) const fn value(&self) -> &SymExpr {
        &self.value
    }

    pub(crate) fn select(&self, read_key: SymExpr, base: SymExpr) -> SymExpr {
        storage_select(read_key, self.key.clone(), self.value.clone(), base)
    }
}

#[derive(Clone, Debug, Default)]
struct SymbolicWorldSnapshot {
    storage: Vec<StorageWrite>,
    transient_storage: Vec<StorageWrite>,
    current_transaction_created_accounts: HashSet<Address>,
    balances: HashMap<Address, SymExpr>,
    code_cache: HashMap<Address, SymCode>,
    nonces: HashMap<Address, u64>,
    existing_accounts: HashSet<Address>,
    destroyed_accounts: HashSet<Address>,
    arbitrary_storage_accounts: HashSet<Address>,
    arbitrary_storage_all: bool,
    zero_init_symbolic_storage: bool,
    symbolic_address_aliases: HashMap<SymExpr, Address>,
}

impl From<&SymbolicWorld> for SymbolicWorldSnapshot {
    fn from(world: &SymbolicWorld) -> Self {
        Self {
            storage: world.storage.clone(),
            transient_storage: world.transient_storage.clone(),
            current_transaction_created_accounts: world
                .current_transaction_created_accounts
                .clone(),
            balances: world.balances.clone(),
            code_cache: world.code_cache.clone(),
            nonces: world.nonces.clone(),
            existing_accounts: world.existing_accounts.clone(),
            destroyed_accounts: world.destroyed_accounts.clone(),
            arbitrary_storage_accounts: world.arbitrary_storage_accounts.clone(),
            arbitrary_storage_all: world.arbitrary_storage_all,
            zero_init_symbolic_storage: world.zero_init_symbolic_storage,
            symbolic_address_aliases: world.symbolic_address_aliases.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SymbolicWorld {
    storage: Vec<StorageWrite>,
    transient_storage: Vec<StorageWrite>,
    current_transaction_created_accounts: HashSet<Address>,
    balances: HashMap<Address, SymExpr>,
    code_cache: HashMap<Address, SymCode>,
    nonces: HashMap<Address, u64>,
    existing_accounts: HashSet<Address>,
    destroyed_accounts: HashSet<Address>,
    arbitrary_storage_accounts: HashSet<Address>,
    arbitrary_storage_all: bool,
    zero_init_symbolic_storage: bool,
    symbolic_address_aliases: HashMap<SymExpr, Address>,
    snapshots: HashMap<U256, SymbolicWorldSnapshot>,
    next_snapshot_id: u64,
}

impl SymbolicWorld {
    pub(crate) fn is_destroyed(&self, address: Address) -> bool {
        self.destroyed_accounts.contains(&address)
    }

    #[cfg(test)]
    pub(crate) fn cached_code(&self, address: Address) -> Option<&SymCode> {
        self.code_cache.get(&address)
    }

    #[cfg(test)]
    pub(crate) fn cached_nonce(&self, address: Address) -> Option<u64> {
        self.nonces.get(&address).copied()
    }

    #[cfg(test)]
    pub(crate) const fn storage_len(&self) -> usize {
        self.storage.len()
    }

    #[cfg(test)]
    pub(crate) fn storage_value(&self, index: usize) -> Option<&SymExpr> {
        self.storage.get(index).map(StorageWrite::value)
    }

    pub(crate) const fn set_storage_layout(&mut self, layout: SymbolicStorageLayout) {
        self.arbitrary_storage_all = matches!(layout, SymbolicStorageLayout::Generic);
        self.zero_init_symbolic_storage = matches!(layout, SymbolicStorageLayout::ZeroInit);
    }

    pub(crate) fn sload<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
        key: SymExpr,
        concrete_key: Option<U256>,
    ) -> Result<SymExpr, SymbolicError> {
        let base = self.storage_base(executor, address, &key, concrete_key)?;
        let read_key = concrete_key.map(SymExpr::constant).unwrap_or(key);
        Ok(read_storage_writes(&self.storage, address, read_key, base))
    }

    pub(crate) fn sstore(&mut self, address: Address, key: SymExpr, value: SymExpr) {
        self.storage.push(StorageWrite::new(address, key, value));
    }

    pub(crate) fn tload(&self, address: Address, key: SymExpr) -> SymExpr {
        read_storage_writes(&self.transient_storage, address, key, SymExpr::zero())
    }

    pub(crate) fn tstore(&mut self, address: Address, key: SymExpr, value: SymExpr) {
        self.transient_storage.push(StorageWrite::new(address, key, value));
    }

    /// Clears transaction-scoped state at a top-level call boundary.
    pub(crate) fn clear_transaction_scoped_state(&mut self) {
        self.transient_storage.clear();
        self.current_transaction_created_accounts.clear();
    }

    pub(crate) fn mark_current_transaction_created(&mut self, address: Address) {
        self.current_transaction_created_accounts.insert(address);
    }

    /// Returns whether `address` was created in the current top-level symbolic transaction.
    pub(crate) fn was_created_in_current_transaction(&self, address: Address) -> bool {
        self.current_transaction_created_accounts.contains(&address)
    }

    pub(crate) fn enable_arbitrary_storage(&mut self, address: Address) {
        self.arbitrary_storage_accounts.insert(address);
    }

    pub(crate) fn resolve_address(&self, word: &SymExpr) -> Option<Address> {
        word.as_const().map(word_to_address).or_else(|| {
            self.symbolic_address_aliases.get(word).copied().or_else(|| {
                self.symbolic_address_aliases.iter().find_map(|(alias, address)| {
                    symbolic_address_equivalent(word, alias).then_some(*address)
                })
            })
        })
    }

    pub(crate) fn symbolic_address_slot(&mut self, word: SymExpr) -> Address {
        if let Some(address) = self.resolve_address(&word) {
            return address;
        }
        let address = representative_symbolic_address(&word);
        self.symbolic_address_aliases.insert(word, address);
        address
    }

    pub(crate) fn symbolic_word_for_address(&self, address: Address) -> Option<SymExpr> {
        self.symbolic_address_aliases
            .iter()
            .find_map(|(word, slot)| (*slot == address).then(|| word.clone()))
    }

    pub(crate) fn snapshot_state(&mut self) -> U256 {
        let id = U256::from(self.next_snapshot_id);
        self.next_snapshot_id = self.next_snapshot_id.saturating_add(1);
        self.snapshots.insert(id, SymbolicWorldSnapshot::from(&*self));
        id
    }

    pub(crate) fn restore_snapshot(&mut self, id: U256) -> bool {
        let Some(snapshot) = self.snapshots.get(&id).cloned() else {
            return false;
        };
        self.storage = snapshot.storage;
        self.transient_storage = snapshot.transient_storage;
        self.current_transaction_created_accounts = snapshot.current_transaction_created_accounts;
        self.balances = snapshot.balances;
        self.code_cache = snapshot.code_cache;
        self.nonces = snapshot.nonces;
        self.existing_accounts = snapshot.existing_accounts;
        self.destroyed_accounts = snapshot.destroyed_accounts;
        self.arbitrary_storage_accounts = snapshot.arbitrary_storage_accounts;
        self.arbitrary_storage_all = snapshot.arbitrary_storage_all;
        self.zero_init_symbolic_storage = snapshot.zero_init_symbolic_storage;
        self.symbolic_address_aliases = snapshot.symbolic_address_aliases;
        true
    }

    pub(crate) fn delete_snapshot(&mut self, id: U256) -> bool {
        self.snapshots.remove(&id).is_some()
    }

    pub(crate) fn delete_snapshots(&mut self) {
        self.snapshots.clear();
    }

    pub(crate) fn storage_base<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
        key: &SymExpr,
        concrete_key: Option<U256>,
    ) -> Result<SymExpr, SymbolicError> {
        if self.arbitrary_storage_all || self.arbitrary_storage_accounts.contains(&address) {
            let name = stable_symbol("storage", format!("{address:?}:{key:?}").as_bytes());
            return Ok(SymExpr::var_symbol(name));
        }
        if let Some(key) = concrete_key {
            return executor
                .backend()
                .storage_ref(address, key)
                .map(SymExpr::constant)
                .map_err(|err| SymbolicError::Backend(err.to_string()));
        }
        if let Some(key) = key.as_const() {
            executor
                .backend()
                .storage_ref(address, key)
                .map(SymExpr::constant)
                .map_err(|err| SymbolicError::Backend(err.to_string()))
        } else if self.zero_init_symbolic_storage {
            Ok(SymExpr::zero())
        } else {
            let name = stable_symbol("storage", format!("{address:?}:{key:?}").as_bytes());
            Ok(SymExpr::var_symbol(name))
        }
    }

    pub(crate) fn backend_balance<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> U256 {
        executor
            .backend()
            .basic_ref(address)
            .ok()
            .flatten()
            .map(|account| account.balance)
            .unwrap_or_default()
    }

    pub(crate) fn balance_word_for_address<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> SymExpr {
        if self.destroyed_accounts.contains(&address) {
            return SymExpr::zero();
        }
        self.balances
            .get(&address)
            .cloned()
            .unwrap_or_else(|| SymExpr::constant(self.backend_balance(executor, address)))
    }

    pub(crate) fn balance_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return Ok(self.balance_word_for_address(executor, address));
        }

        let expr = word;
        let representative = representative_symbolic_address(&expr);
        let mut result = self.balance_word_for_address(executor, representative);
        for (address, balance) in &self.balances {
            if self.destroyed_accounts.contains(address) {
                continue;
            }
            result = SymExpr::ite(
                SymBoolExpr::eq(expr.clone(), SymExpr::constant(address_word(*address))),
                balance.clone(),
                result,
            );
        }

        Ok(result)
    }

    pub(crate) fn set_balance_word(&mut self, address: Address, value: SymExpr) {
        self.balances.insert(address, value.clone());
        if !value.as_const().is_some_and(|value| value.is_zero()) {
            self.existing_accounts.insert(address);
            self.destroyed_accounts.remove(&address);
        }
    }

    pub(crate) fn transfer<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        from: Address,
        to: Address,
        value: SymExpr,
    ) {
        if value.as_const().is_some_and(|value| value.is_zero()) {
            return;
        }
        let from_balance = self.balance_word_for_address(executor, from);
        let to_balance = self.balance_word_for_address(executor, to);
        self.set_balance_word(from, SymExpr::op(SymExprOp::Sub, from_balance, value.clone()));
        self.set_balance_word(to, SymExpr::op(SymExprOp::Add, to_balance, value));
    }

    pub(crate) fn nonce<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<u64, SymbolicError> {
        if self.destroyed_accounts.contains(&address) {
            return Ok(self.nonces.get(&address).copied().unwrap_or_default());
        }
        if let Some(nonce) = self.nonces.get(&address) {
            return Ok(*nonce);
        }
        executor
            .backend()
            .basic_ref(address)
            .map_err(|err| SymbolicError::Backend(err.to_string()))
            .map(|account| account.map(|account| account.nonce).unwrap_or_default())
    }

    pub(crate) fn set_nonce(&mut self, address: Address, nonce: u64) {
        self.nonces.insert(address, nonce);
        if nonce != 0 {
            self.existing_accounts.insert(address);
            self.destroyed_accounts.remove(&address);
        }
    }

    pub(crate) fn increment_nonce<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<(), SymbolicError> {
        let nonce = self.nonce(executor, address)?;
        self.set_nonce(address, nonce.saturating_add(1));
        Ok(())
    }

    pub(crate) fn has_code_or_nonce<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<bool, SymbolicError> {
        if self.destroyed_accounts.contains(&address) {
            return Ok(false);
        }
        Ok(!self.extcode(executor, address)?.is_empty() || self.nonce(executor, address)? != 0)
    }

    pub(crate) fn install_code(&mut self, address: Address, code: SymCode) {
        self.code_cache.insert(address, code);
        self.existing_accounts.insert(address);
        self.destroyed_accounts.remove(&address);
    }

    /// Implements legacy `SELFDESTRUCT` semantics.
    pub(crate) fn selfdestruct_legacy<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
        beneficiary: Address,
    ) -> Result<(), SymbolicError> {
        let balance = self.balance_word_for_address(executor, address);
        if beneficiary != address && !balance.as_const().is_some_and(|value| value.is_zero()) {
            let beneficiary_balance = self.balance_word_for_address(executor, beneficiary);
            self.set_balance_word(
                beneficiary,
                SymExpr::op(SymExprOp::Add, beneficiary_balance, balance),
            );
        }
        self.balances.insert(address, SymExpr::zero());
        self.code_cache.insert(address, SymCode::default());
        if !self.nonces.contains_key(&address) {
            let nonce = self.nonce(executor, address)?;
            self.nonces.insert(address, nonce);
        }
        self.storage.retain(|write| write.address() != address);
        self.transient_storage.retain(|write| write.address() != address);
        self.existing_accounts.remove(&address);
        self.destroyed_accounts.insert(address);
        Ok(())
    }

    /// Implements Cancun+ `SELFDESTRUCT` semantics for accounts not created in the current tx.
    pub(crate) fn selfdestruct_cancun_existing<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
        beneficiary: Address,
    ) {
        let balance = self.balance_word_for_address(executor, address);
        if beneficiary != address && !balance.as_const().is_some_and(|value| value.is_zero()) {
            let beneficiary_balance = self.balance_word_for_address(executor, beneficiary);
            // Symbolic balances are treated as possibly non-zero, matching transfer's
            // account-existence approximation.
            self.set_balance_word(
                beneficiary,
                SymExpr::op(SymExprOp::Add, beneficiary_balance, balance),
            );
            self.balances.insert(address, SymExpr::zero());
        }
    }

    pub(crate) fn account_exists<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<bool, SymbolicError> {
        let spec_id: SpecId = executor.spec_id().into();
        if is_known_cheatcode(address) || is_supported_precompile(address, spec_id) {
            return Ok(true);
        }
        if self.destroyed_accounts.contains(&address) {
            return Ok(false);
        }
        if self.existing_accounts.contains(&address) {
            return Ok(true);
        }
        if self
            .balances
            .get(&address)
            .is_some_and(|balance| !balance.as_const().is_some_and(|value| value.is_zero()))
            || self.nonces.get(&address).is_some_and(|nonce| *nonce != 0)
            || self.code_cache.get(&address).is_some_and(|code| !code.is_empty())
        {
            self.existing_accounts.insert(address);
            return Ok(true);
        }

        let Some(account) = executor
            .backend()
            .basic_ref(address)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?
        else {
            return Ok(false);
        };

        if account.nonce != 0 || !account.balance.is_zero() {
            self.existing_accounts.insert(address);
            return Ok(true);
        }

        if let Some(code) = account.code.as_ref()
            && !code.is_empty()
        {
            self.code_cache.insert(address, SymCode::from_bytecode(code));
            self.existing_accounts.insert(address);
            return Ok(true);
        }

        Ok(false)
    }

    pub(crate) fn extcode<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<SymCode, SymbolicError> {
        if is_known_cheatcode(address) {
            return Ok(SymCode::concrete(vec![0]));
        }
        let spec_id: SpecId = executor.spec_id().into();
        if is_supported_precompile(address, spec_id) {
            return Ok(SymCode::default());
        }
        if self.destroyed_accounts.contains(&address) {
            return Ok(SymCode::default());
        }
        if let Some(code) = self.code_cache.get(&address) {
            return Ok(code.clone());
        }
        let account = executor
            .backend()
            .basic_ref(address)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?;
        if let Some(account) = account.as_ref()
            && (account.nonce != 0
                || !account.balance.is_zero()
                || account.code.as_ref().is_some_and(|code| !code.is_empty()))
        {
            self.existing_accounts.insert(address);
        }
        let bytecode = account.as_ref().and_then(|account| account.code.as_ref());
        let code = bytecode.map(SymCode::from_bytecode).unwrap_or_default();
        self.code_cache.insert(address, code.clone());
        Ok(code)
    }

    pub(crate) fn extcode_hash_for_address<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<SymExpr, SymbolicError> {
        if self.account_exists(executor, address)? {
            let code = self.extcode(executor, address)?;
            Ok(keccak_word(code.read_bytes(0, code.len())))
        } else {
            Ok(SymExpr::zero())
        }
    }

    pub(crate) fn extcode_size_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return Ok(SymExpr::constant(U256::from(self.extcode(executor, address)?.len())));
        }

        let expr = word;
        let representative = representative_symbolic_address(&expr);
        let mut result =
            SymExpr::constant(U256::from(self.extcode(executor, representative)?.len()));
        for (address, code) in &self.code_cache {
            if self.destroyed_accounts.contains(address) {
                continue;
            }
            result = SymExpr::ite(
                SymBoolExpr::eq(expr.clone(), SymExpr::constant(address_word(*address))),
                SymExpr::constant(U256::from(code.len())),
                result,
            );
        }

        Ok(result)
    }

    pub(crate) fn extcode_hash_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return self.extcode_hash_for_address(executor, address);
        }

        let expr = word;
        let representative = representative_symbolic_address(&expr);
        let mut result = self.extcode_hash_for_address(executor, representative)?;
        let cached_codes = self.code_cache.iter().collect::<Vec<_>>();
        for (address, code) in cached_codes.into_iter().rev() {
            let hash = if self.destroyed_accounts.contains(address) {
                SymExpr::zero()
            } else {
                keccak_word(code.read_bytes(0, code.len()))
            };
            result = SymExpr::ite(
                SymBoolExpr::eq(expr.clone(), SymExpr::constant(address_word(*address))),
                hash,
                result,
            );
        }

        Ok(result)
    }

    pub(crate) fn extcode_bytes_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymExpr,
        offset: SymExpr,
        size: usize,
    ) -> Result<Vec<SymExpr>, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return Ok(self.extcode(executor, address)?.read_bytes_offset(offset, size));
        }

        let expr = word;
        let representative = representative_symbolic_address(&expr);
        let mut result =
            self.extcode(executor, representative)?.read_bytes_offset(offset.clone(), size);
        let cached_codes = self.code_cache.iter().collect::<Vec<_>>();
        for (address, code) in cached_codes.into_iter().rev() {
            let bytes = if self.destroyed_accounts.contains(address) {
                vec![SymExpr::zero(); size]
            } else {
                code.read_bytes_offset(offset.clone(), size)
            };
            let condition =
                SymBoolExpr::eq(expr.clone(), SymExpr::constant(address_word(*address)));
            for (idx, byte) in bytes.into_iter().enumerate() {
                result[idx] = SymExpr::ite(condition.clone(), byte, result[idx].clone());
            }
        }

        Ok(result)
    }

    pub(crate) fn symbolic_call_targets<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
    ) -> Result<Vec<Address>, SymbolicError> {
        let mut addresses = HashSet::<Address>::default();
        addresses.extend(self.code_cache.keys().copied());
        addresses.extend(self.existing_accounts.iter().copied());
        addresses.extend(executor.backend().mem_db().cache.accounts.keys().copied());
        if let Some(db) = executor.backend().active_fork_db() {
            addresses.extend(db.cache.accounts.keys().copied());
        }
        let mut addresses = addresses.into_iter().collect::<Vec<_>>();
        addresses.sort_unstable();

        let mut targets = Vec::new();
        let spec_id: SpecId = executor.spec_id().into();
        for address in addresses {
            if is_known_cheatcode(address) || is_supported_precompile(address, spec_id) {
                continue;
            }
            if !self.extcode(executor, address)?.is_empty() {
                targets.push(address);
            }
        }
        Ok(targets)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SymbolicBlock {
    pub(crate) chain_id: SymExpr,
    pub(crate) coinbase: Address,
    pub(crate) timestamp: SymExpr,
    pub(crate) number: SymExpr,
    pub(crate) difficulty: SymExpr,
    pub(crate) gaslimit: SymExpr,
    pub(crate) basefee: SymExpr,
    pub(crate) blob_basefee: SymExpr,
    pub(crate) block_hashes: HashMap<U256, SymExpr>,
    pub(crate) blob_hashes: Vec<B256>,
}

impl Default for SymbolicBlock {
    fn default() -> Self {
        Self {
            chain_id: SymExpr::constant(U256::from(1)),
            coinbase: Address::ZERO,
            timestamp: SymExpr::zero(),
            number: SymExpr::zero(),
            difficulty: SymExpr::zero(),
            gaslimit: SymExpr::zero(),
            basefee: SymExpr::zero(),
            blob_basefee: SymExpr::zero(),
            block_hashes: HashMap::default(),
            blob_hashes: Vec::new(),
        }
    }
}

/// Collects the symbolic variables needed to concretely evaluate an expression.
fn collect_eval_vars(expr: &SymExpr, vars: &mut SymbolicVars) {
    let _ = expr.visit(&mut |expr| {
        match expr.kind() {
            SymExprKind::Var(var) => {
                vars.insert(*var);
            }
            SymExprKind::Hash { name, .. } => {
                vars.insert(*name);
            }
            SymExprKind::Const(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Not(_)
            | SymExprKind::Op(_, _, _)
            | SymExprKind::AddMod { .. }
            | SymExprKind::MulMod { .. }
            | SymExprKind::Ite(_, _, _) => {}
        }
        ControlFlow::<()>::Continue(())
    });
}

impl SymbolicBlock {
    pub(crate) fn from_executor<FEN: FoundryEvmNetwork>(executor: &Executor<FEN>) -> Self {
        let evm_env = executor.evm_env();
        let block = executor
            .inspector()
            .cheatcodes
            .as_ref()
            .and_then(|cheats| cheats.block.as_ref())
            .unwrap_or(&evm_env.block_env);
        let difficulty = block
            .prevrandao()
            .map(|hash| U256::from_be_bytes(hash.0))
            .unwrap_or_else(|| block.difficulty());

        Self {
            chain_id: SymExpr::constant(U256::from(evm_env.cfg_env.chain_id)),
            coinbase: block.beneficiary(),
            timestamp: SymExpr::constant(block.timestamp()),
            number: SymExpr::constant(block.number()),
            difficulty: SymExpr::constant(difficulty),
            gaslimit: SymExpr::constant(U256::from(block.gas_limit())),
            basefee: SymExpr::constant(U256::from(block.basefee())),
            blob_basefee: SymExpr::constant(U256::from(block.blob_gasprice().unwrap_or_default())),
            block_hashes: HashMap::default(),
            blob_hashes: executor.tx_env().blob_versioned_hashes().to_vec(),
        }
    }

    pub(crate) fn set_block_hash(
        &mut self,
        block_number: U256,
        block_hash: SymExpr,
    ) -> Result<(), SymbolicError> {
        let current =
            self.number.clone().into_concrete("symbolic vm.setBlockhash current number")?;
        if block_number < current && current - block_number <= U256::from(256) {
            self.block_hashes.insert(block_number, block_hash);
        }
        Ok(())
    }

    pub(crate) fn block_hash<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        block_number: U256,
    ) -> Result<SymExpr, SymbolicError> {
        let current = self.number.clone().into_concrete("symbolic BLOCKHASH current number")?;
        if block_number >= current || current - block_number > U256::from(256) {
            return Ok(SymExpr::zero());
        }
        if let Some(hash) = self.block_hashes.get(&block_number) {
            return Ok(hash.clone());
        }
        let Ok(block_number) = u64::try_from(block_number) else {
            return Ok(SymExpr::zero());
        };
        let hash = executor
            .backend()
            .block_hash_ref(block_number)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?;
        Ok(SymExpr::constant(U256::from_be_slice(hash.as_slice())))
    }

    pub(crate) fn block_hash_word<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        block_number: SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        if let Some(block_number) = block_number.as_const() {
            return self.block_hash(executor, block_number);
        }
        let current = self.number.clone().into_concrete("symbolic BLOCKHASH current number")?;
        if current.is_zero() {
            return Ok(SymExpr::zero());
        }

        let mut result = SymExpr::constant(U256::ZERO);
        let max_distance =
            usize::try_from(current.min(U256::from(256))).expect("checked blockhash distance");
        for distance in (1..=max_distance).rev() {
            let candidate = current - U256::from(distance);
            let hash = self.block_hash(executor, candidate)?;
            if hash.as_const().is_some_and(|hash| hash.is_zero()) {
                continue;
            }
            result = SymExpr::ite(
                SymBoolExpr::eq(block_number.clone(), SymExpr::constant(candidate)),
                hash,
                result,
            );
        }

        Ok(result)
    }

    pub(crate) fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>) {
        self.blob_hashes = blob_hashes;
    }

    pub(crate) fn blob_hash(&self, index: usize) -> B256 {
        self.blob_hashes.get(index).copied().unwrap_or_default()
    }
}
