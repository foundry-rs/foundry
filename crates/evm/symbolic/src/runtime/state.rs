use super::*;

#[derive(Clone, Debug)]
pub(crate) struct PathState {
    pub(crate) depth: usize,
    pub(crate) call_depth: usize,
    pub(crate) origin: Address,
    pub(crate) origin_word: SymWord,
    pub(crate) gas_price: SymWord,
    pub(crate) ffi_enabled: bool,
    pub(crate) block: SymbolicBlock,
    pub(crate) frame: CallFrame,
    pub(crate) world: SymbolicWorld,
    pub(crate) prank: SymbolicPrank,
    pub(crate) constraints: Vec<BoolExpr>,
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
    pub(crate) wallets: BTreeSet<Address>,
    pub(crate) labels: HashMap<Address, String>,
}

impl PathState {
    /// Constructs a new instance.
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
            origin_word: SymWord::Concrete(address_word(caller)),
            gas_price: SymWord::zero(),
            ffi_enabled,
            block: SymbolicBlock::default(),
            frame: CallFrame::new(
                address,
                address,
                address,
                caller,
                SymWord::Concrete(callvalue),
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
            wallets: BTreeSet::new(),
            labels: HashMap::default(),
        }
    }

    /// Implements the `empty` symbolic state helper.
    pub(crate) fn empty(address: Address, caller: Address, ffi_enabled: bool) -> Self {
        Self {
            depth: 0,
            call_depth: 0,
            origin: caller,
            origin_word: SymWord::Concrete(address_word(caller)),
            gas_price: SymWord::zero(),
            ffi_enabled,
            block: SymbolicBlock::default(),
            frame: CallFrame::new(
                address,
                address,
                address,
                caller,
                SymWord::zero(),
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
            wallets: BTreeSet::new(),
            labels: HashMap::default(),
        }
    }

    /// Applies the `apply_executor_env` symbolic state helper.
    pub(crate) fn apply_executor_env<FEN: FoundryEvmNetwork>(&mut self, executor: &Executor<FEN>) {
        self.block = SymbolicBlock::from_executor(executor);
        let gas_price = executor
            .inspector()
            .cheatcodes
            .as_ref()
            .and_then(|cheats| cheats.gas_price)
            .unwrap_or_else(|| executor.tx_env().gas_price());
        self.gas_price = SymWord::Concrete(U256::from(gas_price));
    }

    /// Implements the `child` symbolic state helper.
    pub(crate) fn child(&self, frame: CallFrame) -> Self {
        let mut child = self.clone();
        child.call_depth += 1;
        child.frame = frame;
        child.loop_jumps = HashMap::default();
        child
    }

    /// Implements the `constrained_usize` symbolic state helper.
    pub(crate) fn constrained_usize(&self, word: &SymWord) -> Option<usize> {
        let value = self.constrained_word(word)?;
        (value <= U256::from(usize::MAX)).then(|| value.to::<usize>())
    }

    /// Implements the `upper_bound_usize` symbolic state helper.
    pub(crate) fn upper_bound_usize(&self, word: &SymWord) -> Option<usize> {
        self.constrained_usize(word).or_else(|| match word {
            SymWord::Concrete(value) => u256_to_usize(*value),
            SymWord::Expr(expr) => self.expr_upper_bound_usize(expr),
        })
    }

    /// Implements the `constrained_word` symbolic state helper.
    pub(crate) fn constrained_word(&self, word: &SymWord) -> Option<U256> {
        let value = match word {
            SymWord::Concrete(value) => *value,
            SymWord::Expr(expr) => self
                .constraints
                .iter()
                .find_map(|constraint| {
                    bool_forces_expr_const_with_context(constraint, expr, &self.constraints)
                })
                .or_else(|| self.constrained_expr_value(expr))?,
        };
        Some(value)
    }

    /// Implements the `constrained_expr_value` symbolic state helper.
    pub(crate) fn constrained_expr_value(&self, expr: &Expr) -> Option<U256> {
        if let Some(value) = expr_const_value(expr) {
            return Some(value);
        }
        if let Some(value) = expr_known_word(expr) {
            return Some(value);
        }

        let mut vars = BTreeSet::new();
        collect_eval_vars(expr, &mut vars);
        let mut model = BTreeMap::new();
        for var in vars {
            let var_expr = Expr::var(var.clone());
            let value = self.constraints.iter().find_map(|constraint| {
                bool_forces_expr_const_with_context(constraint, &var_expr, &self.constraints)
            })?;
            model.insert(var.to_string(), value);
        }

        eval_expr(expr, &model).ok()
    }

    /// Returns the `expr_upper_bound_usize` symbolic state helper result.
    pub(crate) fn expr_upper_bound_usize(&self, expr: &Expr) -> Option<usize> {
        if let Some(value) = expr_const_value(expr) {
            return u256_to_usize(value);
        }
        if let Some(value) = expr_known_word(expr) {
            return u256_to_usize(value);
        }

        let constraint_bound = self.constraint_upper_bound_usize(expr);
        let structural_bound = match expr {
            Expr::Const(value) => u256_to_usize(*value),
            Expr::Var(_) | Expr::GasLeft(_) | Expr::Keccak(_) | Expr::Hash(_) => None,
            Expr::Not(_) => None,
            Expr::AddMod { modulus, .. } | Expr::MulMod { modulus, .. } => {
                match expr_const_value(modulus) {
                    Some(modulus) if modulus.is_zero() => Some(0),
                    Some(modulus) => u256_to_usize(modulus - U256::from(1)),
                    None => {
                        self.expr_upper_bound_usize(modulus).and_then(|bound| bound.checked_sub(1))
                    }
                }
            }
            Expr::Ite(_, left, right) => {
                Some(self.expr_upper_bound_usize(left)?.max(self.expr_upper_bound_usize(right)?))
            }
            Expr::Op(op, left, right) => match op {
                ExprOp::Add => self
                    .expr_upper_bound_usize(left)?
                    .checked_add(self.expr_upper_bound_usize(right)?),
                ExprOp::Mul => self
                    .expr_upper_bound_usize(left)?
                    .checked_mul(self.expr_upper_bound_usize(right)?),
                ExprOp::UDiv => {
                    let left = self.expr_upper_bound_usize(left)?;
                    match expr_const_value(right)? {
                        divisor if divisor.is_zero() => Some(0),
                        divisor => Some(left / u256_to_usize(divisor)?),
                    }
                }
                ExprOp::URem => match expr_const_value(right) {
                    Some(divisor) if divisor.is_zero() => Some(0),
                    Some(divisor) => u256_to_usize(divisor - U256::from(1)),
                    None => self.expr_upper_bound_usize(left),
                },
                ExprOp::And => expr_const_value(right)
                    .and_then(u256_to_usize)
                    .or_else(|| expr_const_value(left).and_then(u256_to_usize))
                    .map(|mask| {
                        self.expr_upper_bound_usize(left)
                            .or_else(|| self.expr_upper_bound_usize(right))
                            .map_or(mask, |bound| bound.min(mask))
                    }),
                ExprOp::Shr => {
                    let left = self.expr_upper_bound_usize(left)?;
                    let shift = u256_to_usize(expr_const_value(right)?)?;
                    Some(if shift >= usize::BITS as usize { 0 } else { left >> shift })
                }
                ExprOp::Sub
                | ExprOp::SDiv
                | ExprOp::SRem
                | ExprOp::Or
                | ExprOp::Xor
                | ExprOp::Shl
                | ExprOp::Sar => None,
            },
        };

        match (constraint_bound, structural_bound) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(bound), None) | (None, Some(bound)) => Some(bound),
            (None, None) => None,
        }
    }

    /// Implements the `constraint_upper_bound_usize` symbolic state helper.
    pub(crate) fn constraint_upper_bound_usize(&self, expr: &Expr) -> Option<usize> {
        let mut bound: Option<usize> = None;
        for constraint in &self.constraints {
            if let Some(candidate) = bool_upper_bound_usize(constraint, expr) {
                bound = Some(bound.map_or(candidate, |bound| bound.min(candidate)));
            }
        }
        bound
    }

    /// Implements the `expect_constrained_usize` symbolic state helper.
    pub(crate) fn expect_constrained_usize(
        &self,
        word: SymWord,
        reason: &'static str,
    ) -> Result<usize, SymbolicError> {
        self.constrained_usize(&word).ok_or(SymbolicError::Unsupported(reason))
    }

    /// Implements the `expect_constrained_word` symbolic state helper.
    pub(crate) fn expect_constrained_word(
        &self,
        word: SymWord,
        reason: &'static str,
    ) -> Result<U256, SymbolicError> {
        self.constrained_word(&word).ok_or(SymbolicError::Unsupported(reason))
    }

    /// Implements the `bin_word` symbolic state helper.
    pub(crate) fn bin_word(
        &mut self,
        concrete: impl FnOnce(U256, U256) -> U256,
        op: ExprOp,
    ) -> Result<StepOutcome, SymbolicError> {
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(match (a, b) {
            (SymWord::Concrete(a), SymWord::Concrete(b)) => SymWord::Concrete(concrete(a, b)),
            (a, b) => SymWord::from_expr(Expr::op(op, a.into_expr(), b.into_expr())),
        })?;
        Ok(StepOutcome::Continue)
    }

    /// Implements the `bin_word_div_zero_guard` symbolic state helper.
    pub(crate) fn bin_word_div_zero_guard(
        &mut self,
        concrete: impl FnOnce(U256, U256) -> U256,
        op: ExprOp,
    ) -> Result<StepOutcome, SymbolicError> {
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(match (a, b) {
            (SymWord::Concrete(a), SymWord::Concrete(b)) => SymWord::Concrete(concrete(a, b)),
            (a, b) => {
                let a = a.into_expr();
                let b = b.into_expr();
                SymWord::from_expr(Expr::ite(
                    BoolExpr::eq(b.clone(), Expr::Const(U256::ZERO)),
                    Expr::Const(U256::ZERO),
                    Expr::op(op, a, b),
                ))
            }
        })?;
        Ok(StepOutcome::Continue)
    }

    /// Implements the `cmp_word` symbolic state helper.
    pub(crate) fn cmp_word(
        &mut self,
        concrete: impl FnOnce(U256, U256) -> bool,
        op: BoolExprOp,
    ) -> Result<StepOutcome, SymbolicError> {
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(match (a, b) {
            (SymWord::Concrete(a), SymWord::Concrete(b)) => {
                SymWord::Concrete(U256::from(concrete(a, b)))
            }
            (a, b) => SymWord::from_bool(BoolExpr::cmp(op, a.into_expr(), b.into_expr())),
        })?;
        Ok(StepOutcome::Continue)
    }

    /// Computes the `shift_word` symbolic state helper result.
    pub(crate) fn shift_word(&mut self, kind: ShiftKind) -> Result<StepOutcome, SymbolicError> {
        let shift = self.stack.pop()?;
        let value = self.stack.pop()?;
        let result = match (value, shift) {
            (SymWord::Concrete(value), SymWord::Concrete(shift)) => {
                let result = if shift >= U256::from(256) {
                    if matches!(kind, ShiftKind::Sar) && ((value >> 255) == U256::from(1)) {
                        U256::MAX
                    } else {
                        U256::ZERO
                    }
                } else {
                    let shift = shift.to::<usize>();
                    match kind {
                        ShiftKind::Shl => value << shift,
                        ShiftKind::Shr => value >> shift,
                        ShiftKind::Sar => sar(value, shift),
                    }
                };
                SymWord::Concrete(result)
            }
            (value, shift) => {
                let expr = match kind {
                    ShiftKind::Shl => Expr::op(ExprOp::Shl, value.into_expr(), shift.into_expr()),
                    ShiftKind::Shr => Expr::op(ExprOp::Shr, value.into_expr(), shift.into_expr()),
                    ShiftKind::Sar => Expr::op(ExprOp::Sar, value.into_expr(), shift.into_expr()),
                };
                expr_known_word(&expr)
                    .map(SymWord::Concrete)
                    .unwrap_or_else(|| SymWord::from_expr(expr))
            }
        };
        self.stack.push(result)?;
        Ok(StepOutcome::Continue)
    }

    /// Computes the `exp_word` symbolic state helper result.
    pub(crate) fn exp_word(&mut self) -> Result<StepOutcome, SymbolicError> {
        let base = self.stack.pop()?;
        let exponent = self.stack.pop()?;
        let result = if let Some(exponent) = self.constrained_word(&exponent) {
            match base {
                SymWord::Concrete(base) => SymWord::Concrete(pow_mod(base, exponent)),
                base if exponent <= U256::from(SYMBOLIC_EXP_CONCRETE_EXPONENT_LIMIT) => {
                    SymWord::from_expr(exp_expr_for_concrete_exponent(
                        base.into_expr(),
                        exponent.to::<usize>(),
                    ))
                }
                _ => return Err(SymbolicError::Unsupported("symbolic EXP base")),
            }
        } else {
            let exponent_limit = if matches!(base, SymWord::Concrete(_)) {
                CONCRETE_BASE_SYMBOLIC_EXPONENT_LIMIT
            } else {
                SYMBOLIC_EXP_CONCRETE_EXPONENT_LIMIT
            };
            let max_exponent = self
                .upper_bound_usize(&exponent)
                .filter(|exponent| *exponent <= exponent_limit as usize)
                .ok_or(SymbolicError::Unsupported("symbolic EXP exponent"))?;
            let exponent = exponent.into_expr();
            let base = base.into_expr();
            let mut expr = Expr::Const(U256::ZERO);
            for candidate in (0..=max_exponent).rev() {
                expr = Expr::ite(
                    BoolExpr::eq(exponent.clone(), Expr::Const(U256::from(candidate))),
                    exp_expr_for_concrete_exponent(base.clone(), candidate),
                    expr,
                );
            }
            SymWord::from_expr(expr)
        };
        self.stack.push(result)?;
        Ok(StepOutcome::Continue)
    }

    /// Implements the `balance` symbolic state helper.
    pub(crate) fn balance<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> SymWord {
        self.world.balance_word_for_address(executor, address)
    }

    /// Implements the `balance_word` symbolic state helper.
    pub(crate) fn balance_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        self.world.balance_word(executor, word)
    }

    /// Implements the `extcode_size_word` symbolic state helper.
    pub(crate) fn extcode_size_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        self.world.extcode_size_word(executor, word)
    }

    /// Implements the `extcode_hash_word` symbolic state helper.
    pub(crate) fn extcode_hash_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        self.world.extcode_hash_word(executor, word)
    }

    /// Implements the `extcode_bytes_word` symbolic state helper.
    pub(crate) fn extcode_bytes_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
        offset: SymWord,
        size: usize,
    ) -> Result<Vec<SymWord>, SymbolicError> {
        self.world.extcode_bytes_word(executor, word, offset, size)
    }

    /// Implements the `pop_address_word_or_symbolic_slot` symbolic state helper.
    pub(crate) fn pop_address_word_or_symbolic_slot(
        &mut self,
    ) -> Result<(SymWord, Address), SymbolicError> {
        let word = self.stack.pop()?;
        let address = self.address_or_symbolic_slot(word.clone());
        Ok((word, address))
    }

    /// Returns the `address_or_symbolic_slot` symbolic state helper result.
    pub(crate) fn address_or_symbolic_slot(&mut self, word: SymWord) -> Address {
        if let Some(value) = self.constrained_word(&word) {
            return word_to_address(value);
        }
        self.world.resolve_address(&word).unwrap_or_else(|| self.world.symbolic_address_slot(word))
    }

    /// Implements the `fresh_word` symbolic state helper.
    pub(crate) fn fresh_word(&mut self, prefix: &'static str) -> SymWord {
        let id = self.next_symbol;
        self.next_symbol += 1;
        SymWord::expr(Expr::var(format!("{prefix}_{id}")))
    }

    /// Implements the `fresh_gasleft` symbolic state helper.
    pub(crate) const fn fresh_gasleft(&mut self) -> SymWord {
        let id = self.next_symbol;
        self.next_symbol += 1;
        SymWord::Expr(Expr::GasLeft(id))
    }

    /// Implements the `fresh_bounded_uint` symbolic state helper.
    pub(crate) fn fresh_bounded_uint(&mut self, bits: U256) -> SymWord {
        let value = self.fresh_word("symbolic");
        if bits < U256::from(256) {
            let upper =
                if bits.is_zero() { U256::ZERO } else { U256::from(1) << bits.to::<usize>() };
            self.constraints.push(BoolExpr::cmp(
                BoolExprOp::Ult,
                value.clone().into_expr(),
                Expr::Const(upper),
            ));
        }
        value
    }

    /// Implements the `fresh_bounded_int` symbolic state helper.
    pub(crate) fn fresh_bounded_int(&mut self, bits: U256) -> SymWord {
        let value = self.fresh_word("symbolic");
        if bits.is_zero() {
            self.constraints.push(BoolExpr::eq(value.clone().into_expr(), Expr::Const(U256::ZERO)));
        } else if bits < U256::from(256) {
            let magnitude = U256::from(1) << (bits.to::<usize>() - 1);
            self.constraints.push(BoolExpr::or(vec![
                BoolExpr::cmp(BoolExprOp::Ult, value.clone().into_expr(), Expr::Const(magnitude)),
                BoolExpr::cmp(
                    BoolExprOp::Uge,
                    value.clone().into_expr(),
                    Expr::Const(U256::ZERO.wrapping_sub(magnitude)),
                ),
            ]));
        }
        value
    }

    /// Implements the `prank_for_next_call` symbolic state helper.
    pub(crate) fn prank_for_next_call(&mut self) -> (Address, SymWord, Option<(Address, SymWord)>) {
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

    /// Returns the `read_callers_words` symbolic state helper result.
    pub(crate) fn read_callers_words(&self) -> Vec<SymWord> {
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
        vec![SymWord::Concrete(mode), caller, origin]
    }

    /// Applies the `record_log` symbolic state helper.
    pub(crate) fn record_log(&mut self, log: SymbolicLog) {
        if let Some(logs) = &mut self.recorded_logs {
            logs.push(log);
        }
    }

    /// Applies the `record_sload` symbolic state helper.
    pub(crate) fn record_sload(&mut self, address: Address, slot: SymWord) {
        if let Some(record) = &mut self.access_record {
            record.read(address, slot);
        }
    }

    /// Applies the `record_sstore` symbolic state helper.
    pub(crate) fn record_sstore(&mut self, address: Address, slot: SymWord) {
        if let Some(record) = &mut self.access_record {
            record.write(address, slot);
        }
    }

    /// Returns whether `expectations_satisfied` holds.
    pub(crate) fn expectations_satisfied(&self) -> bool {
        self.expected_revert.is_none()
            && self.expected_emit.as_ref().is_none_or(ExpectedEmit::is_satisfied)
            && self.expected_calls.iter().all(ExpectedCall::is_satisfied)
            && self.expected_creates.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SymbolicLog {
    pub(crate) topics: Vec<SymWord>,
    pub(crate) data_len: SymWord,
    pub(crate) data: Vec<SymWord>,
    pub(crate) emitter: Address,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AccessRecord {
    pub(crate) reads: HashMap<Address, Vec<SymWord>>,
    pub(crate) writes: HashMap<Address, Vec<SymWord>>,
}

impl AccessRecord {
    /// Implements the `read` symbolic state helper.
    pub(crate) fn read(&mut self, address: Address, slot: SymWord) {
        push_unique_slot(self.reads.entry(address).or_default(), slot);
    }

    /// Implements the `write` symbolic state helper.
    pub(crate) fn write(&mut self, address: Address, slot: SymWord) {
        push_unique_slot(self.writes.entry(address).or_default(), slot);
    }
}

/// Applies the `push_unique_slot` symbolic state helper.
pub(crate) fn push_unique_slot(slots: &mut Vec<SymWord>, slot: SymWord) {
    if !slots.iter().any(|existing| existing == &slot) {
        slots.push(slot);
    }
}

/// Implements the `adjust_expected_call_gas_for_value` symbolic state helper.
pub(crate) fn adjust_expected_call_gas_for_value(
    value: Option<U256>,
    gas: Option<u64>,
    min_gas: Option<u64>,
) -> (Option<u64>, Option<u64>) {
    if value.is_some_and(|value| !value.is_zero()) {
        (
            gas.map(|gas| gas.saturating_add(CALL_VALUE_STIPEND)),
            min_gas.map(|gas| gas.saturating_add(CALL_VALUE_STIPEND)),
        )
    } else {
        (gas, min_gas)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedRevert {
    pub(crate) data: ExpectedRevertData,
    pub(crate) reverter: Option<SymWord>,
    pub(crate) remaining: u64,
}

impl ExpectedRevert {
    /// Implements the `consume_one` symbolic state helper.
    pub(crate) const fn consume_one(&mut self) -> bool {
        self.remaining = self.remaining.saturating_sub(1);
        self.remaining == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExpectedRevertData {
    Any,
    Prefix(Vec<SymWord>),
    Exact(Vec<SymWord>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AssumeNoRevert {
    Any,
    Filtered(Vec<ExpectedRevert>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedCall {
    pub(crate) callee: SymWord,
    pub(crate) value: Option<U256>,
    pub(crate) gas: Option<u64>,
    pub(crate) min_gas: Option<u64>,
    pub(crate) data: Vec<SymWord>,
    pub(crate) expected: u64,
    pub(crate) observed: u64,
    pub(crate) exact: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedCreate {
    pub(crate) bytecode: Vec<u8>,
    pub(crate) deployer: SymWord,
    pub(crate) kind: CreateKind,
}

impl ExpectedCall {
    /// Implements the `static_parts_match` symbolic state helper.
    pub(crate) fn static_parts_match(
        &self,
        value: Option<U256>,
        gas: &SymWord,
    ) -> Result<bool, SymbolicError> {
        Ok(self.value.is_none_or(|expected| value.is_some_and(|value| expected == value))
            && self.gas_matches(gas, value)?)
    }

    /// Returns whether `gas_matches` holds.
    pub(crate) fn gas_matches(
        &self,
        gas: &SymWord,
        value: Option<U256>,
    ) -> Result<bool, SymbolicError> {
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

    /// Applies the `observe` symbolic state helper.
    pub(crate) const fn observe(&mut self) -> bool {
        if self.exact && self.observed >= self.expected {
            return false;
        }
        self.observed = self.observed.saturating_add(1);
        true
    }

    /// Returns whether `is_satisfied` holds.
    pub(crate) const fn is_satisfied(&self) -> bool {
        if self.exact { self.observed == self.expected } else { self.observed >= self.expected }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CallMock {
    pub(crate) callee: SymWord,
    pub(crate) value: Option<U256>,
    pub(crate) data: Vec<SymWord>,
    pub(crate) returns: Vec<SymReturnData>,
    pub(crate) reverts: bool,
    pub(crate) calls: usize,
}

impl CallMock {
    /// Implements the `static_parts_match` symbolic state helper.
    pub(crate) fn static_parts_match(&self, value: Option<U256>) -> bool {
        self.value.is_none_or(|expected| value.is_some_and(|value| expected == value))
    }

    /// Implements the `next_outcome` symbolic state helper.
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
    pub(crate) return_data: SymReturnData,
    pub(crate) reverts: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FunctionMock {
    pub(crate) callee: SymWord,
    pub(crate) target: Address,
    pub(crate) data: Vec<SymWord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedEmit {
    pub(crate) checks: ExpectedEmitChecks,
    pub(crate) emitter: Option<SymWord>,
    pub(crate) remaining: u64,
    pub(crate) template: Option<SymbolicLog>,
}

impl ExpectedEmit {
    /// Returns whether `is_satisfied` holds.
    pub(crate) const fn is_satisfied(&self) -> bool {
        self.template.is_none() && self.remaining == 0
    }

    /// Implements the `consume_one` symbolic state helper.
    pub(crate) fn consume_one(&mut self) -> bool {
        self.remaining = self.remaining.saturating_sub(1);
        if self.remaining == 0 {
            self.template = None;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedEmitChecks {
    pub(crate) topics: [bool; 4],
    pub(crate) data: bool,
}

impl ExpectedEmitChecks {
    /// Implements the `default_non_anonymous` symbolic state helper.
    pub(crate) const fn default_non_anonymous() -> Self {
        Self { topics: [true, true, true, true], data: true }
    }

    /// Implements the `default_anonymous` symbolic state helper.
    pub(crate) const fn default_anonymous() -> Self {
        Self { topics: [true, true, true, true], data: true }
    }

    /// Converts values for the `from_non_anonymous_args` symbolic state helper.
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

    /// Converts values for the `from_anonymous_args` symbolic state helper.
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

    /// Implements the `deref` symbolic state helper.
    fn deref(&self) -> &Self::Target {
        &self.frame
    }
}

impl DerefMut for PathState {
    /// Implements the `deref_mut` symbolic state helper.
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.frame
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CallFrame {
    pub(crate) pc: usize,
    pub(crate) address: Address,
    pub(crate) address_word: SymWord,
    #[allow(dead_code)]
    pub(crate) code_address: Address,
    pub(crate) storage_address: Address,
    pub(crate) caller: Address,
    pub(crate) caller_word: SymWord,
    pub(crate) callvalue: SymWord,
    pub(crate) is_static: bool,
    pub(crate) calldata: SymCalldata,
    pub(crate) stack: SymStack,
    pub(crate) memory: SymMemory,
    pub(crate) return_data: SymReturnData,
}

impl CallFrame {
    /// Constructs a new instance.
    pub(crate) fn new(
        address: Address,
        code_address: Address,
        storage_address: Address,
        caller: Address,
        callvalue: SymWord,
        is_static: bool,
        calldata: SymCalldata,
    ) -> Self {
        Self {
            pc: 0,
            address,
            address_word: SymWord::Concrete(address_word(address)),
            code_address,
            storage_address,
            caller,
            caller_word: SymWord::Concrete(address_word(caller)),
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
    pub(crate) next_caller: Option<(Address, SymWord)>,
    pub(crate) next_origin: Option<(Address, SymWord)>,
    pub(crate) persistent_caller: Option<(Address, SymWord)>,
    pub(crate) persistent_origin: Option<(Address, SymWord)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StorageWrite {
    pub(crate) address: Address,
    pub(crate) key: SymWord,
    pub(crate) value: SymWord,
}

impl StorageWrite {
    /// Constructs a new instance.
    pub(crate) const fn new(address: Address, key: SymWord, value: SymWord) -> Self {
        Self { address, key, value }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SymbolicWorldSnapshot {
    pub(crate) storage: Vec<StorageWrite>,
    pub(crate) transient_storage: Vec<StorageWrite>,
    pub(crate) current_transaction_created_accounts: HashSet<Address>,
    pub(crate) balances: HashMap<Address, SymWord>,
    pub(crate) code_cache: HashMap<Address, SymCode>,
    pub(crate) nonces: HashMap<Address, u64>,
    pub(crate) existing_accounts: HashSet<Address>,
    pub(crate) destroyed_accounts: HashSet<Address>,
    pub(crate) arbitrary_storage_accounts: HashSet<Address>,
    pub(crate) arbitrary_storage_all: bool,
    pub(crate) zero_init_symbolic_storage: bool,
    pub(crate) symbolic_address_aliases: HashMap<SymWord, Address>,
}

impl From<&SymbolicWorld> for SymbolicWorldSnapshot {
    /// Implements the `from` symbolic state helper.
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
    pub(crate) storage: Vec<StorageWrite>,
    pub(crate) transient_storage: Vec<StorageWrite>,
    pub(crate) current_transaction_created_accounts: HashSet<Address>,
    pub(crate) balances: HashMap<Address, SymWord>,
    pub(crate) code_cache: HashMap<Address, SymCode>,
    pub(crate) nonces: HashMap<Address, u64>,
    pub(crate) existing_accounts: HashSet<Address>,
    pub(crate) destroyed_accounts: HashSet<Address>,
    pub(crate) arbitrary_storage_accounts: HashSet<Address>,
    pub(crate) arbitrary_storage_all: bool,
    pub(crate) zero_init_symbolic_storage: bool,
    pub(crate) symbolic_address_aliases: HashMap<SymWord, Address>,
    pub(crate) snapshots: HashMap<U256, SymbolicWorldSnapshot>,
    pub(crate) next_snapshot_id: u64,
}

impl SymbolicWorld {
    /// Applies the `set_storage_layout` symbolic state helper.
    pub(crate) const fn set_storage_layout(&mut self, layout: SymbolicStorageLayout) {
        self.arbitrary_storage_all = matches!(layout, SymbolicStorageLayout::Generic);
        self.zero_init_symbolic_storage = matches!(layout, SymbolicStorageLayout::ZeroInit);
    }

    /// Implements the `sload` symbolic state helper.
    pub(crate) fn sload<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
        key: SymWord,
        concrete_key: Option<U256>,
    ) -> Result<SymWord, SymbolicError> {
        let base = self.storage_base(executor, address, &key, concrete_key)?;
        let read_key = concrete_key.map(SymWord::Concrete).unwrap_or(key);
        Ok(read_storage_writes(&self.storage, address, read_key, base))
    }

    /// Implements the `sstore` symbolic state helper.
    pub(crate) fn sstore(&mut self, address: Address, key: SymWord, value: SymWord) {
        self.storage.push(StorageWrite::new(address, key, value));
    }

    /// Implements the `tload` symbolic state helper.
    pub(crate) fn tload(&self, address: Address, key: SymWord) -> SymWord {
        read_storage_writes(&self.transient_storage, address, key, SymWord::zero())
    }

    /// Implements the `tstore` symbolic state helper.
    pub(crate) fn tstore(&mut self, address: Address, key: SymWord, value: SymWord) {
        self.transient_storage.push(StorageWrite::new(address, key, value));
    }

    /// Clears transaction-scoped state at a top-level call boundary.
    pub(crate) fn clear_transaction_scoped_state(&mut self) {
        self.transient_storage.clear();
        self.current_transaction_created_accounts.clear();
    }

    /// Applies the `mark_current_transaction_created` symbolic state helper.
    pub(crate) fn mark_current_transaction_created(&mut self, address: Address) {
        self.current_transaction_created_accounts.insert(address);
    }

    /// Returns whether `address` was created in the current top-level symbolic transaction.
    pub(crate) fn was_created_in_current_transaction(&self, address: Address) -> bool {
        self.current_transaction_created_accounts.contains(&address)
    }

    /// Applies the `enable_arbitrary_storage` symbolic state helper.
    pub(crate) fn enable_arbitrary_storage(&mut self, address: Address) {
        self.arbitrary_storage_accounts.insert(address);
    }

    /// Implements the `resolve_address` symbolic state helper.
    pub(crate) fn resolve_address(&self, word: &SymWord) -> Option<Address> {
        match word {
            SymWord::Concrete(value) => Some(word_to_address(*value)),
            SymWord::Expr(_) => self.symbolic_address_aliases.get(word).copied().or_else(|| {
                self.symbolic_address_aliases.iter().find_map(|(alias, address)| {
                    symbolic_address_equivalent(word, alias).then_some(*address)
                })
            }),
        }
    }

    /// Returns the `symbolic_address_slot` symbolic state helper result.
    pub(crate) fn symbolic_address_slot(&mut self, word: SymWord) -> Address {
        if let Some(address) = self.resolve_address(&word) {
            return address;
        }
        let address = representative_symbolic_address(&word);
        self.symbolic_address_aliases.insert(word, address);
        address
    }

    /// Returns the `symbolic_word_for_address` symbolic state helper result.
    pub(crate) fn symbolic_word_for_address(&self, address: Address) -> Option<SymWord> {
        self.symbolic_address_aliases
            .iter()
            .find_map(|(word, slot)| (*slot == address).then(|| word.clone()))
    }

    /// Implements the `snapshot_state` symbolic state helper.
    pub(crate) fn snapshot_state(&mut self) -> U256 {
        let id = U256::from(self.next_snapshot_id);
        self.next_snapshot_id = self.next_snapshot_id.saturating_add(1);
        self.snapshots.insert(id, SymbolicWorldSnapshot::from(&*self));
        id
    }

    /// Applies the `restore_snapshot` symbolic state helper.
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

    /// Applies the `delete_snapshot` symbolic state helper.
    pub(crate) fn delete_snapshot(&mut self, id: U256) -> bool {
        self.snapshots.remove(&id).is_some()
    }

    /// Applies the `delete_snapshots` symbolic state helper.
    pub(crate) fn delete_snapshots(&mut self) {
        self.snapshots.clear();
    }

    /// Implements the `storage_base` symbolic state helper.
    pub(crate) fn storage_base<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
        key: &SymWord,
        concrete_key: Option<U256>,
    ) -> Result<SymWord, SymbolicError> {
        if self.arbitrary_storage_all || self.arbitrary_storage_accounts.contains(&address) {
            return Ok(SymWord::expr(Expr::var(stable_symbol(
                "storage",
                format!("{address:?}:{key:?}"),
            ))));
        }
        if let Some(key) = concrete_key {
            return executor
                .backend()
                .storage_ref(address, key)
                .map(SymWord::Concrete)
                .map_err(|err| SymbolicError::Backend(err.to_string()));
        }
        match key {
            SymWord::Concrete(key) => executor
                .backend()
                .storage_ref(address, *key)
                .map(SymWord::Concrete)
                .map_err(|err| SymbolicError::Backend(err.to_string())),
            SymWord::Expr(_) if self.zero_init_symbolic_storage => Ok(SymWord::zero()),
            SymWord::Expr(_) => Ok(SymWord::expr(Expr::var(stable_symbol(
                "storage",
                format!("{address:?}:{key:?}"),
            )))),
        }
    }

    /// Implements the `backend_balance` symbolic state helper.
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

    /// Implements the `balance_word_for_address` symbolic state helper.
    pub(crate) fn balance_word_for_address<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> SymWord {
        if self.destroyed_accounts.contains(&address) {
            return SymWord::zero();
        }
        self.balances
            .get(&address)
            .cloned()
            .unwrap_or_else(|| SymWord::Concrete(self.backend_balance(executor, address)))
    }

    /// Implements the `balance_word` symbolic state helper.
    pub(crate) fn balance_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return Ok(self.balance_word_for_address(executor, address));
        }

        let expr = word.into_expr();
        let representative = representative_symbolic_address(&SymWord::expr(expr.clone()));
        let mut result = self.balance_word_for_address(executor, representative).into_expr();
        for (address, balance) in &self.balances {
            if self.destroyed_accounts.contains(address) {
                continue;
            }
            result = Expr::ite(
                BoolExpr::eq(expr.clone(), Expr::Const(address_word(*address))),
                balance.clone().into_expr(),
                result,
            );
        }

        Ok(SymWord::from_expr(result))
    }

    /// Applies the `set_balance_word` symbolic state helper.
    pub(crate) fn set_balance_word(&mut self, address: Address, value: SymWord) {
        self.balances.insert(address, value.clone());
        if !matches!(value, SymWord::Concrete(value) if value.is_zero()) {
            self.existing_accounts.insert(address);
            self.destroyed_accounts.remove(&address);
        }
    }

    /// Implements the `transfer` symbolic state helper.
    pub(crate) fn transfer<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        from: Address,
        to: Address,
        value: SymWord,
    ) {
        if matches!(value, SymWord::Concrete(value) if value.is_zero()) {
            return;
        }
        let from_balance = self.balance_word_for_address(executor, from);
        let to_balance = self.balance_word_for_address(executor, to);
        self.set_balance_word(from, sym_sub(from_balance, value.clone()));
        self.set_balance_word(to, sym_add(to_balance, value));
    }

    /// Implements the `nonce` symbolic state helper.
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

    /// Applies the `set_nonce` symbolic state helper.
    pub(crate) fn set_nonce(&mut self, address: Address, nonce: u64) {
        self.nonces.insert(address, nonce);
        if nonce != 0 {
            self.existing_accounts.insert(address);
            self.destroyed_accounts.remove(&address);
        }
    }

    /// Implements the `increment_nonce` symbolic state helper.
    pub(crate) fn increment_nonce<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<(), SymbolicError> {
        let nonce = self.nonce(executor, address)?;
        self.set_nonce(address, nonce.saturating_add(1));
        Ok(())
    }

    /// Returns whether `has_code_or_nonce` holds.
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

    /// Applies the `install_code` symbolic state helper.
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
        if beneficiary != address && !matches!(balance, SymWord::Concrete(value) if value.is_zero())
        {
            let beneficiary_balance = self.balance_word_for_address(executor, beneficiary);
            self.set_balance_word(beneficiary, sym_add(beneficiary_balance, balance));
        }
        self.balances.insert(address, SymWord::zero());
        self.code_cache.insert(address, SymCode::default());
        if !self.nonces.contains_key(&address) {
            let nonce = self.nonce(executor, address)?;
            self.nonces.insert(address, nonce);
        }
        self.storage.retain(|write| write.address != address);
        self.transient_storage.retain(|write| write.address != address);
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
        if beneficiary != address && !matches!(balance, SymWord::Concrete(value) if value.is_zero())
        {
            let beneficiary_balance = self.balance_word_for_address(executor, beneficiary);
            // Symbolic balances are treated as possibly non-zero, matching transfer's
            // account-existence approximation.
            self.set_balance_word(beneficiary, sym_add(beneficiary_balance, balance));
            self.balances.insert(address, SymWord::zero());
        }
    }

    /// Implements the `account_exists` symbolic state helper.
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
            .is_some_and(|balance| !matches!(balance, SymWord::Concrete(value) if value.is_zero()))
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

        let code = account.code.map(|code| code.original_bytes().to_vec()).unwrap_or_default();
        if !code.is_empty() {
            self.code_cache.insert(address, SymCode::concrete(code));
            self.existing_accounts.insert(address);
            return Ok(true);
        }

        Ok(false)
    }

    /// Implements the `extcode` symbolic state helper.
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
        let code = account
            .as_ref()
            .and_then(|account| account.code.as_ref().map(|code| code.original_bytes().to_vec()))
            .unwrap_or_default();
        if let Some(account) = account
            && (account.nonce != 0 || !account.balance.is_zero() || !code.is_empty())
        {
            self.existing_accounts.insert(address);
        }
        let code = SymCode::concrete(code);
        self.code_cache.insert(address, code.clone());
        Ok(code)
    }

    /// Implements the `extcode_hash_for_address` symbolic state helper.
    pub(crate) fn extcode_hash_for_address<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        address: Address,
    ) -> Result<SymWord, SymbolicError> {
        if self.account_exists(executor, address)? {
            let code = self.extcode(executor, address)?;
            Ok(keccak_word(code.read_bytes(0, code.len())))
        } else {
            Ok(SymWord::zero())
        }
    }

    /// Implements the `extcode_size_word` symbolic state helper.
    pub(crate) fn extcode_size_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return Ok(SymWord::Concrete(U256::from(self.extcode(executor, address)?.len())));
        }

        let expr = word.into_expr();
        let representative = representative_symbolic_address(&SymWord::expr(expr.clone()));
        let mut result = Expr::Const(U256::from(self.extcode(executor, representative)?.len()));
        for (address, code) in &self.code_cache {
            if self.destroyed_accounts.contains(address) {
                continue;
            }
            result = Expr::ite(
                BoolExpr::eq(expr.clone(), Expr::Const(address_word(*address))),
                Expr::Const(U256::from(code.len())),
                result,
            );
        }

        Ok(SymWord::from_expr(result))
    }

    /// Implements the `extcode_hash_word` symbolic state helper.
    pub(crate) fn extcode_hash_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return self.extcode_hash_for_address(executor, address);
        }

        let expr = word.into_expr();
        let representative = representative_symbolic_address(&SymWord::expr(expr.clone()));
        let mut result = self.extcode_hash_for_address(executor, representative)?.into_expr();
        let cached_codes: Vec<_> =
            self.code_cache.iter().map(|(address, code)| (*address, code.clone())).collect();
        for (address, code) in cached_codes.into_iter().rev() {
            let hash = if self.destroyed_accounts.contains(&address) {
                SymWord::zero()
            } else {
                keccak_word(code.read_bytes(0, code.len()))
            };
            result = Expr::ite(
                BoolExpr::eq(expr.clone(), Expr::Const(address_word(address))),
                hash.into_expr(),
                result,
            );
        }

        Ok(SymWord::from_expr(result))
    }

    /// Implements the `extcode_bytes_word` symbolic state helper.
    pub(crate) fn extcode_bytes_word<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        word: SymWord,
        offset: SymWord,
        size: usize,
    ) -> Result<Vec<SymWord>, SymbolicError> {
        if let Some(address) = self.resolve_address(&word) {
            return Ok(self.extcode(executor, address)?.read_bytes_offset(offset, size));
        }

        let expr = word.into_expr();
        let representative = representative_symbolic_address(&SymWord::expr(expr.clone()));
        let mut result =
            self.extcode(executor, representative)?.read_bytes_offset(offset.clone(), size);
        let cached_codes: Vec<_> =
            self.code_cache.iter().map(|(address, code)| (*address, code.clone())).collect();
        for (address, code) in cached_codes.into_iter().rev() {
            let bytes = if self.destroyed_accounts.contains(&address) {
                vec![SymWord::zero(); size]
            } else {
                code.read_bytes_offset(offset.clone(), size)
            };
            for (idx, byte) in bytes.into_iter().enumerate() {
                result[idx] = SymWord::from_expr(Expr::ite(
                    BoolExpr::eq(expr.clone(), Expr::Const(address_word(address))),
                    byte.into_expr(),
                    result[idx].clone().into_expr(),
                ));
            }
        }

        Ok(result)
    }

    /// Returns the `symbolic_call_targets` symbolic state helper result.
    pub(crate) fn symbolic_call_targets<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
    ) -> Result<Vec<Address>, SymbolicError> {
        let mut addresses = BTreeSet::new();
        addresses.extend(self.code_cache.keys().copied());
        addresses.extend(self.existing_accounts.iter().copied());
        addresses.extend(executor.backend().mem_db().cache.accounts.keys().copied());
        if let Some(db) = executor.backend().active_fork_db() {
            addresses.extend(db.cache.accounts.keys().copied());
        }

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
    pub(crate) chain_id: SymWord,
    pub(crate) coinbase: Address,
    pub(crate) timestamp: SymWord,
    pub(crate) number: SymWord,
    pub(crate) difficulty: SymWord,
    pub(crate) gaslimit: SymWord,
    pub(crate) basefee: SymWord,
    pub(crate) blob_basefee: SymWord,
    pub(crate) block_hashes: HashMap<U256, SymWord>,
    pub(crate) blob_hashes: Vec<B256>,
}

impl Default for SymbolicBlock {
    /// Implements the `default` symbolic state helper.
    fn default() -> Self {
        Self {
            chain_id: SymWord::Concrete(U256::from(1)),
            coinbase: Address::ZERO,
            timestamp: SymWord::zero(),
            number: SymWord::zero(),
            difficulty: SymWord::zero(),
            gaslimit: SymWord::zero(),
            basefee: SymWord::zero(),
            blob_basefee: SymWord::zero(),
            block_hashes: HashMap::default(),
            blob_hashes: Vec::new(),
        }
    }
}

/// Collects the symbolic variables needed to concretely evaluate an expression.
fn collect_eval_vars(expr: &Expr, vars: &mut SymbolicVars) {
    expr.visit(&mut |expr| match expr {
        Expr::Var(var) => {
            vars.insert(var.clone());
        }
        Expr::Hash(hash) => {
            vars.insert(hash.name.clone());
        }
        Expr::Const(_)
        | Expr::GasLeft(_)
        | Expr::Keccak(_)
        | Expr::Not(_)
        | Expr::Op(_, _, _)
        | Expr::AddMod { .. }
        | Expr::MulMod { .. }
        | Expr::Ite(_, _, _) => {}
    });
}

impl SymbolicBlock {
    /// Converts values for the `from_executor` symbolic state helper.
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
            chain_id: SymWord::Concrete(U256::from(evm_env.cfg_env.chain_id)),
            coinbase: block.beneficiary(),
            timestamp: SymWord::Concrete(block.timestamp()),
            number: SymWord::Concrete(block.number()),
            difficulty: SymWord::Concrete(difficulty),
            gaslimit: SymWord::Concrete(U256::from(block.gas_limit())),
            basefee: SymWord::Concrete(U256::from(block.basefee())),
            blob_basefee: SymWord::Concrete(U256::from(block.blob_gasprice().unwrap_or_default())),
            block_hashes: HashMap::default(),
            blob_hashes: executor.tx_env().blob_versioned_hashes().to_vec(),
        }
    }

    /// Applies the `set_block_hash` symbolic state helper.
    pub(crate) fn set_block_hash(
        &mut self,
        block_number: U256,
        block_hash: SymWord,
    ) -> Result<(), SymbolicError> {
        let current =
            self.number.clone().into_concrete("symbolic vm.setBlockhash current number")?;
        if block_number < current && current - block_number <= U256::from(256) {
            self.block_hashes.insert(block_number, block_hash);
        }
        Ok(())
    }

    /// Implements the `block_hash` symbolic state helper.
    pub(crate) fn block_hash<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        block_number: U256,
    ) -> Result<SymWord, SymbolicError> {
        let current = self.number.clone().into_concrete("symbolic BLOCKHASH current number")?;
        if block_number >= current || current - block_number > U256::from(256) {
            return Ok(SymWord::zero());
        }
        if let Some(hash) = self.block_hashes.get(&block_number) {
            return Ok(hash.clone());
        }
        let Ok(block_number) = u64::try_from(block_number) else {
            return Ok(SymWord::zero());
        };
        let hash = executor
            .backend()
            .block_hash_ref(block_number)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?;
        Ok(SymWord::Concrete(U256::from_be_slice(hash.as_slice())))
    }

    /// Implements the `block_hash_word` symbolic state helper.
    pub(crate) fn block_hash_word<FEN: FoundryEvmNetwork>(
        &self,
        executor: &Executor<FEN>,
        block_number: SymWord,
    ) -> Result<SymWord, SymbolicError> {
        let block_number = match block_number {
            SymWord::Concrete(block_number) => {
                return self.block_hash(executor, block_number);
            }
            SymWord::Expr(block_number) => block_number,
        };

        let current = self.number.clone().into_concrete("symbolic BLOCKHASH current number")?;
        if current.is_zero() {
            return Ok(SymWord::zero());
        }

        let mut result = Expr::Const(U256::ZERO);
        let max_distance = current.min(U256::from(256)).to::<usize>();
        for distance in (1..=max_distance).rev() {
            let candidate = current - U256::from(distance);
            let hash = self.block_hash(executor, candidate)?;
            if matches!(&hash, SymWord::Concrete(hash) if hash.is_zero()) {
                continue;
            }
            result = Expr::ite(
                BoolExpr::eq(block_number.clone(), Expr::Const(candidate)),
                hash.into_expr(),
                result,
            );
        }

        Ok(SymWord::from_expr(result))
    }

    /// Applies the `set_blob_hashes` symbolic state helper.
    pub(crate) fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>) {
        self.blob_hashes = blob_hashes;
    }

    /// Implements the `blob_hash` symbolic state helper.
    pub(crate) fn blob_hash(&self, index: usize) -> B256 {
        self.blob_hashes.get(index).copied().unwrap_or_default()
    }
}
