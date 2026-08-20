use super::{hashcons::HashConsed, *};

/// Bounds both the number of distinct word nodes inspected by constant-ITE equality expansion and
/// the size of the Boolean tree that a later non-memoized fold could observe.
const MAX_CONSTANT_ITE_EQ_NODES: usize = 128;
const MAX_CONSTANT_ITE_EQ_UNFOLDED_NODES: usize = 8 * 1024;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct SymBoolExpr {
    pub(in crate::runtime::expr) kind: HashConsed<SymBoolExprKind>,
}

impl fmt::Debug for SymBoolExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::runtime) enum SymBoolExprKind {
    Const(bool),
    Not(SymBoolExpr),
    And(Arc<[SymBoolExpr]>),
    Cmp(SymCmpOp, SymExpr, SymExpr),
}

impl SymBoolExpr {
    #[inline]
    pub(in crate::runtime) fn stable_hash_cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.kind.stable_hash_cmp(&other.kind)
    }

    pub(in crate::runtime) fn kind(&self) -> &SymBoolExprKind {
        self.kind.value()
    }

    pub(in crate::runtime) fn into_kind(self) -> SymBoolExprKind {
        self.kind.into_value()
    }

    pub(in crate::runtime) fn from_kind(cx: &mut SymCx, kind: SymBoolExprKind) -> Self {
        cx.mk_bool_kind(kind)
    }

    pub(crate) fn constant(cx: &mut SymCx, value: bool) -> Self {
        cx.cached_bool(value)
    }

    pub(crate) fn cmp_word_const(
        cx: &mut SymCx,
        op: SymCmpOp,
        word: &SymExpr,
        value: U256,
    ) -> Self {
        if let Some(word) = word.as_const() {
            Self::constant(cx, op.eval(word, value))
        } else {
            let value = SymExpr::constant(cx, value);
            Self::cmp(cx, op, word.clone(), value)
        }
    }

    pub(crate) fn eq_word_const(cx: &mut SymCx, word: &SymExpr, value: U256) -> Self {
        if let Some(word) = word.as_const() {
            Self::constant(cx, word == value)
        } else {
            let value = SymExpr::constant(cx, value);
            Self::eq(cx, word.clone(), value)
        }
    }

    pub(crate) fn eq(cx: &mut SymCx, left: SymExpr, right: SymExpr) -> Self {
        Self::cmp(cx, SymCmpOp::Eq, left, right)
    }

    pub(crate) fn cmp(cx: &mut SymCx, op: SymCmpOp, left: SymExpr, right: SymExpr) -> Self {
        if let (
            SymExprKind::Ite(left_condition, left_then, left_else),
            SymExprKind::Ite(right_condition, right_then, right_else),
        ) = (left.kind(), right.kind())
            && left_condition == right_condition
            && let (Some(left_then), Some(left_else), Some(right_then), Some(right_else)) = (
                left_then.as_const(),
                left_else.as_const(),
                right_then.as_const(),
                right_else.as_const(),
            )
        {
            // Compare aligned constant-arm ITEs pointwise without expanding either expression.
            return match (op.eval(left_then, right_then), op.eval(left_else, right_else)) {
                (true, true) => Self::constant(cx, true),
                (false, false) => Self::constant(cx, false),
                (true, false) => left_condition.clone(),
                (false, true) => Self::not_bool(cx, left_condition.clone()),
            };
        }

        match op {
            SymCmpOp::Eq => {
                if let Some(condition) = Self::ite_eq_arm(cx, &left, &right)
                    .or_else(|| Self::ite_eq_arm(cx, &right, &left))
                {
                    return condition;
                }
                match (left.kind(), right.kind()) {
                    // `a == a => true`.
                    _ if left == right => Self::constant(cx, true),
                    (SymExprKind::Const(left), SymExprKind::Const(right)) => {
                        // `const == const => const`.
                        Self::constant(cx, left == right)
                    }
                    (_, SymExprKind::Const(right_value)) => {
                        if let Some(condition) = Self::bool_word_eq_const(cx, &left, *right_value) {
                            return condition;
                        }
                        if let Some(left_value) = left.known_word() {
                            // `known(a) == const => const`.
                            return Self::constant(cx, left_value == *right_value);
                        }
                        // `a == b => ordered(a, b)`.
                        let (left, right) = SymExpr::ordered_commutative_operands(left, right);
                        Self::from_kind(cx, SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right))
                    }
                    (SymExprKind::Const(left_value), _) => {
                        if let Some(condition) = Self::bool_word_eq_const(cx, &right, *left_value) {
                            return condition;
                        }
                        if let Some(right_value) = right.known_word() {
                            // `const == known(a) => const`.
                            return Self::constant(cx, *left_value == right_value);
                        }
                        // `a == b => ordered(a, b)`.
                        let (left, right) = SymExpr::ordered_commutative_operands(left, right);
                        Self::from_kind(cx, SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right))
                    }
                    (
                        SymExprKind::Keccak { len: left_len, bytes: left_bytes, .. },
                        SymExprKind::Keccak { len: right_len, bytes: right_bytes, .. },
                    ) if left_bytes.len() == right_bytes.len() => {
                        // `keccak(a) == keccak(b) => len(a) == len(b) && bytes(a) == bytes(b)`.
                        let mut conditions =
                            vec![Self::eq(cx, left_len.clone(), right_len.clone())];
                        conditions.extend(
                            left_bytes
                                .iter()
                                .cloned()
                                .zip(right_bytes.iter().cloned())
                                .map(|(left, right)| Self::eq(cx, left, right)),
                        );
                        Self::and(cx, conditions)
                    }
                    (
                        SymExprKind::Hash { algorithm: left_algorithm, bytes: left_bytes, .. },
                        SymExprKind::Hash {
                            algorithm: right_algorithm, bytes: right_bytes, ..
                        },
                    ) if left_algorithm == right_algorithm
                        && left_bytes.len() == right_bytes.len() =>
                    {
                        // `hash(a) == hash(b) => bytes(a) == bytes(b)`.
                        let conditions = left_bytes
                            .iter()
                            .cloned()
                            .zip(right_bytes.iter().cloned())
                            .map(|(left, right)| Self::eq(cx, left, right))
                            .collect();
                        Self::and(cx, conditions)
                    }
                    _ => {
                        // `a == b => ordered(a, b)`.
                        let (left, right) = SymExpr::ordered_commutative_operands(left, right);
                        Self::from_kind(cx, SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right))
                    }
                }
            }
            SymCmpOp::Ult => match (left.kind(), right.kind()) {
                // `a < a => false`.
                _ if left == right => Self::constant(cx, false),
                (SymExprKind::Const(left), SymExprKind::Const(right)) => {
                    // `const < const => const`.
                    Self::constant(cx, op.eval(*left, *right))
                }
                // `a < 0 => false`.
                (_, SymExprKind::Const(value)) if value.is_zero() => Self::constant(cx, false),
                // `MAX < a => false`.
                (SymExprKind::Const(value), _) if *value == U256::MAX => Self::constant(cx, false),
                // `a < a & low_mask => false`.
                _ if low_masked_source_any(&right) == Some(&left) => Self::constant(cx, false),
                _ => Self::from_kind(cx, SymBoolExprKind::Cmp(op, left, right)),
            },
            SymCmpOp::Ugt => match (left.kind(), right.kind()) {
                // `a > a => false`.
                _ if left == right => Self::constant(cx, false),
                (SymExprKind::Const(left), SymExprKind::Const(right)) => {
                    // `const > const => const`.
                    Self::constant(cx, op.eval(*left, *right))
                }
                // `0 > a => false`.
                (SymExprKind::Const(value), _) if value.is_zero() => Self::constant(cx, false),
                // `a > MAX => false`.
                (_, SymExprKind::Const(value)) if *value == U256::MAX => Self::constant(cx, false),
                // `a & low_mask > a => false`.
                _ if low_masked_source_any(&left) == Some(&right) => Self::constant(cx, false),
                _ => Self::from_kind(cx, SymBoolExprKind::Cmp(op, left, right)),
            },
            SymCmpOp::Ule => match (left.kind(), right.kind()) {
                // `a <= a => true`.
                _ if left == right => Self::constant(cx, true),
                (SymExprKind::Const(left), SymExprKind::Const(right)) => {
                    // `const <= const => const`.
                    Self::constant(cx, op.eval(*left, *right))
                }
                // `0 <= a => true`.
                (SymExprKind::Const(value), _) if value.is_zero() => Self::constant(cx, true),
                // `a <= MAX => true`.
                (_, SymExprKind::Const(value)) if *value == U256::MAX => Self::constant(cx, true),
                // `a & low_mask <= a => true`.
                _ if low_masked_source_any(&left) == Some(&right) => Self::constant(cx, true),
                _ => Self::from_kind(cx, SymBoolExprKind::Cmp(op, left, right)),
            },
            SymCmpOp::Uge => match (left.kind(), right.kind()) {
                // `a >= a => true`.
                _ if left == right => Self::constant(cx, true),
                (SymExprKind::Const(left), SymExprKind::Const(right)) => {
                    // `const >= const => const`.
                    Self::constant(cx, op.eval(*left, *right))
                }
                // `a >= 0 => true`.
                (_, SymExprKind::Const(value)) if value.is_zero() => Self::constant(cx, true),
                // `MAX >= a => true`.
                (SymExprKind::Const(value), _) if *value == U256::MAX => Self::constant(cx, true),
                // `a >= a & low_mask => true`.
                _ if low_masked_source_any(&right) == Some(&left) => Self::constant(cx, true),
                _ => Self::from_kind(cx, SymBoolExprKind::Cmp(op, left, right)),
            },
            SymCmpOp::Slt | SymCmpOp::Sgt => match (left.kind(), right.kind()) {
                // `a <s a => false`, `a >s a => false`.
                _ if left == right => Self::constant(cx, false),
                (SymExprKind::Const(left), SymExprKind::Const(right)) => {
                    // `const <s const => const`.
                    Self::constant(cx, op.eval(*left, *right))
                }
                _ => Self::from_kind(cx, SymBoolExprKind::Cmp(op, left, right)),
            },
        }
    }

    pub(crate) fn and(cx: &mut SymCx, values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value.kind() {
                // `true && a => a`.
                SymBoolExprKind::Const(true) => {}
                // `false && a => false`.
                SymBoolExprKind::Const(false) => return Self::constant(cx, false),
                // `(a && b) && c => a && b && c`.
                SymBoolExprKind::And(values) => out.extend(values.iter().cloned()),
                _ => out.push(value),
            }
        }
        if out.is_empty() {
            // `and() => true`.
            Self::constant(cx, true)
        } else if out.len() == 1 {
            // `and(a) => a`.
            out.pop().expect("single item exists")
        } else {
            Self::from_kind(cx, SymBoolExprKind::And(out.into()))
        }
    }

    pub(crate) fn or(cx: &mut SymCx, values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value.kind() {
                // `false || a => a`.
                SymBoolExprKind::Const(false) => {}
                // `true || a => true`.
                SymBoolExprKind::Const(true) => return Self::constant(cx, true),
                _ => out.push(value),
            }
        }
        if out.is_empty() {
            // `or() => false`.
            Self::constant(cx, false)
        } else if out.len() == 1 {
            // `or(a) => a`.
            out.pop().expect("single item exists")
        } else {
            // `a || b => !(!a && !b)`.
            let values = out.into_iter().map(|value| Self::not_bool(cx, value)).collect();
            let and = Self::and(cx, values);
            Self::not_bool(cx, and)
        }
    }

    pub(crate) fn not_bool(cx: &mut SymCx, value: Self) -> Self {
        match value.kind() {
            // `!const => const`.
            SymBoolExprKind::Const(value) => Self::constant(cx, !*value),
            // `!!a => a`.
            SymBoolExprKind::Not(value) => value.clone(),
            _ => Self::from_kind(cx, SymBoolExprKind::Not(value)),
        }
    }

    fn bool_word_eq_const(cx: &mut SymCx, word: &SymExpr, value: U256) -> Option<Self> {
        let SymExprKind::Ite(condition, then_expr, else_expr) = word.kind() else { return None };
        match (then_expr.as_const(), else_expr.as_const()) {
            (Some(then_value), Some(else_value))
                if then_value == U256::from(1) && else_value.is_zero() =>
            {
                Some(if value.is_zero() {
                    Self::not_bool(cx, condition.clone())
                } else if value == U256::from(1) {
                    condition.clone()
                } else {
                    Self::constant(cx, false)
                })
            }
            (Some(then_value), Some(else_value))
                if then_value.is_zero() && else_value == U256::from(1) =>
            {
                Some(if value.is_zero() {
                    condition.clone()
                } else if value == U256::from(1) {
                    Self::not_bool(cx, condition.clone())
                } else {
                    Self::constant(cx, false)
                })
            }
            _ => None,
        }
    }

    fn ite_eq_arm(cx: &mut SymCx, conditional: &SymExpr, expected: &SymExpr) -> Option<Self> {
        let SymExprKind::Ite(condition, then_expr, else_expr) = conditional.kind() else {
            return None;
        };
        if then_expr == expected {
            let else_matches = Self::eq(cx, else_expr.clone(), expected.clone());
            return Some(Self::or(cx, vec![condition.clone(), else_matches]));
        }
        if else_expr == expected {
            let then_matches = Self::eq(cx, then_expr.clone(), expected.clone());
            let condition = Self::not_bool(cx, condition.clone());
            return Some(Self::or(cx, vec![condition, then_matches]));
        }
        if let Some(expected_value) = expected.as_const() {
            let mut cost_cache = HashMap::default();
            let mut remaining = MAX_CONSTANT_ITE_EQ_NODES;
            if Self::constant_ite_eq_unfolded_nodes(conditional, &mut cost_cache, &mut remaining)
                .is_none()
            {
                let (left, right) =
                    SymExpr::ordered_commutative_operands(conditional.clone(), expected.clone());
                return Some(Self::from_kind(cx, SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)));
            }

            let mut cache = HashMap::default();
            let mut remaining = MAX_CONSTANT_ITE_EQ_NODES;
            return Self::constant_ite_eq(
                cx,
                conditional,
                expected_value,
                &mut cache,
                &mut remaining,
            );
        }
        None
    }

    /// Computes a conservative upper bound for the unfolded Boolean result without interning it.
    ///
    /// Cached child costs are deliberately added again at each use. This keeps construction of a
    /// small shared word DAG from creating a Boolean DAG that becomes exponential when a later
    /// consumer traverses occurrences instead of identities.
    fn constant_ite_eq_unfolded_nodes(
        expr: &SymExpr,
        cache: &mut HashMap<SymExpr, Option<usize>>,
        remaining: &mut usize,
    ) -> Option<usize> {
        if let Some(cached) = cache.get(expr) {
            return *cached;
        }
        if *remaining == 0 {
            cache.insert(expr.clone(), None);
            return None;
        }
        *remaining -= 1;

        let result = match expr.kind() {
            SymExprKind::Const(_) => Some(1),
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                let then_nodes = Self::constant_ite_eq_unfolded_nodes(then_expr, cache, remaining)?;
                let else_nodes = Self::constant_ite_eq_unfolded_nodes(else_expr, cache, remaining)?;

                let mut pending = vec![condition];
                let mut condition_nodes = 0usize;
                while let Some(condition) = pending.pop() {
                    condition_nodes = condition_nodes.checked_add(1)?;
                    if condition_nodes > MAX_CONSTANT_ITE_EQ_UNFOLDED_NODES {
                        return None;
                    }
                    match condition.kind() {
                        SymBoolExprKind::Not(value) => pending.push(value),
                        SymBoolExprKind::And(values) => pending.extend(values.iter()),
                        SymBoolExprKind::Const(_) | SymBoolExprKind::Cmp(_, _, _) => {}
                    }
                }

                // Two selected branches and the `or` encoding add at most seven Boolean wrapper
                // occurrences around both children and two occurrences of the condition.
                then_nodes
                    .checked_add(else_nodes)
                    .and_then(|nodes| nodes.checked_add(2 * condition_nodes))
                    .and_then(|nodes| nodes.checked_add(7))
                    .filter(|nodes| *nodes <= MAX_CONSTANT_ITE_EQ_UNFOLDED_NODES)
            }
            _ => None,
        };
        cache.insert(expr.clone(), result);
        result
    }

    fn constant_ite_eq(
        cx: &mut SymCx,
        expr: &SymExpr,
        expected: U256,
        cache: &mut HashMap<SymExpr, Option<Self>>,
        remaining: &mut usize,
    ) -> Option<Self> {
        if let Some(cached) = cache.get(expr) {
            return cached.clone();
        }
        if *remaining == 0 {
            cache.insert(expr.clone(), None);
            return None;
        }
        *remaining -= 1;

        let result = match expr.kind() {
            SymExprKind::Const(value) => Some(Self::constant(cx, *value == expected)),
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                let condition = condition.clone();
                let then_matches = Self::constant_ite_eq(cx, then_expr, expected, cache, remaining);
                let else_matches = Self::constant_ite_eq(cx, else_expr, expected, cache, remaining);
                match (then_matches, else_matches) {
                    (Some(then_matches), Some(else_matches)) => {
                        let then_selected =
                            Self::and_ite_branch(cx, condition.clone(), then_matches);
                        let condition = Self::not_bool(cx, condition);
                        let else_selected = Self::and_ite_branch(cx, condition, else_matches);
                        Some(Self::or(cx, vec![then_selected, else_selected]))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        cache.insert(expr.clone(), result.clone());
        result
    }

    /// Builds one selected ITE branch without flattening a cached child conjunction.
    ///
    /// Flattening here makes a linear ITE chain retain conjunctions of lengths `1..n` in the
    /// per-call cache. Keeping this pair binary preserves the shared DAG and leaves flattening to
    /// solver constraint normalization, where only the final expression is expanded.
    fn and_ite_branch(cx: &mut SymCx, condition: Self, branch: Self) -> Self {
        match (condition.as_const(), branch.as_const()) {
            (Some(false), _) | (_, Some(false)) => Self::constant(cx, false),
            (Some(true), _) => branch,
            (_, Some(true)) => condition,
            _ if condition == branch => condition,
            _ => Self::from_kind(cx, SymBoolExprKind::And(vec![condition, branch].into())),
        }
    }

    pub(crate) fn as_const(&self) -> Option<bool> {
        match self.kind() {
            SymBoolExprKind::Const(value) => Some(*value),
            _ => None,
        }
    }

    pub(in crate::runtime) fn zero_check_operand(&self) -> Option<&SymExpr> {
        match self.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                if right.as_const().is_some_and(|value| value.is_zero()) =>
            {
                Some(left)
            }
            _ => None,
        }
    }

    pub(crate) fn contains_keccak(&self) -> bool {
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::Keccak { .. }))
    }

    pub(crate) fn contains_gasleft(&self) -> bool {
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::GasLeft(_)))
    }

    pub(crate) fn contains_udiv(&self) -> bool {
        self.visit_bool(|expr| expr.contains_udiv())
    }

    pub(crate) fn forces_expr_const_with_context(
        &self,
        expr: &SymExpr,
        context: &[Self],
    ) -> Option<U256> {
        match self.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => match right.kind() {
                SymExprKind::Const(value) => left.equality_forces_const(*value, expr, context),
                _ => None,
            },
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => match right.kind() {
                    SymExprKind::Const(value) if value.is_zero() => {
                        left.nonzero_forces_const(expr, context)
                    }
                    _ => None,
                },
                SymBoolExprKind::Not(value) => value.forces_expr_const_with_context(expr, context),
                _ => None,
            },
            SymBoolExprKind::And(values) => {
                values.iter().find_map(|value| value.forces_expr_const_with_context(expr, context))
            }
            _ => None,
        }
    }

    pub(crate) fn upper_bound_usize(&self, expr: &SymExpr) -> Option<usize> {
        match self.kind() {
            SymBoolExprKind::Const(_) | SymBoolExprKind::Not(_) => None,
            SymBoolExprKind::And(values) => {
                let mut bound: Option<usize> = None;
                for value in values.iter() {
                    if let Some(candidate) = value.upper_bound_usize(expr) {
                        bound = Some(bound.map_or(candidate, |bound| bound.min(candidate)));
                    }
                }
                bound
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                if *op == SymCmpOp::Eq {
                    return match (left == expr, right == expr) {
                        (true, _) => right.eval().and_then(|value| usize::try_from(value).ok()),
                        (_, true) => left.eval().and_then(|value| usize::try_from(value).ok()),
                        _ => None,
                    };
                }
                if left == expr {
                    match *op {
                        SymCmpOp::Ult => right
                            .eval()
                            .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                            .and_then(|value| usize::try_from(value).ok()),
                        SymCmpOp::Ule => right.eval().and_then(|value| usize::try_from(value).ok()),
                        _ => None,
                    }
                } else if right == expr {
                    match *op {
                        SymCmpOp::Ugt => left
                            .eval()
                            .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                            .and_then(|value| usize::try_from(value).ok()),
                        SymCmpOp::Uge => left.eval().and_then(|value| usize::try_from(value).ok()),
                        _ => None,
                    }
                } else {
                    None
                }
            }
        }
    }

    pub(crate) fn eval_model<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<bool, SymbolicError> {
        ModelEvaluator::new(model).eval_bool(self)
    }

    pub(crate) fn eval_model_if_complete<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<Option<bool>, SymbolicError> {
        let mut vars = SymbolicVars::default();
        self.collect_eval_vars(&mut vars);
        if vars.iter().copied().all(|var| model.contains_name(var)) {
            self.eval_model(model).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Visits all word expressions contained in this boolean expression.
    pub(crate) fn visit_exprs<B>(
        &self,
        visitor: &mut impl FnMut(&SymExpr) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        match self.kind() {
            SymBoolExprKind::Const(_) => {}
            SymBoolExprKind::Not(value) => value.visit_exprs(visitor)?,
            SymBoolExprKind::And(values) => {
                for value in values.iter() {
                    value.visit_exprs(visitor)?;
                }
            }
            SymBoolExprKind::Cmp(_, left, right) => {
                left.visit(visitor)?;
                right.visit(visitor)?;
            }
        }
        ControlFlow::Continue(())
    }

    pub(crate) fn visit_bool(&self, mut visitor: impl FnMut(&SymExpr) -> bool) -> bool {
        self.visit_exprs(&mut |expr| {
            if visitor(expr) { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
        })
        .is_break()
    }

    pub(crate) fn fold(
        self,
        cx: &mut SymCx,
        folder: &mut impl FnMut(&mut SymCx, Self) -> Self,
    ) -> Self {
        if matches!(self.kind(), SymBoolExprKind::Const(_)) {
            return folder(cx, self);
        }

        let expr = match self.into_kind() {
            SymBoolExprKind::Not(value) => {
                let value = value.fold(cx, folder);
                Self::not_bool(cx, value)
            }
            SymBoolExprKind::And(values) => {
                let values = values.iter().cloned().map(|value| value.fold(cx, folder)).collect();
                Self::and(cx, values)
            }
            SymBoolExprKind::Cmp(op, left, right) => Self::cmp(cx, op, left, right),
            SymBoolExprKind::Const(_) => unreachable!("leaf boolean returned before folding"),
        };
        folder(cx, expr)
    }

    pub(crate) fn fold_exprs(
        self,
        cx: &mut SymCx,
        folder: &mut impl FnMut(&mut SymCx, SymExpr) -> SymExpr,
    ) -> Self {
        if matches!(self.kind(), SymBoolExprKind::Const(_)) {
            return self;
        }

        match self.into_kind() {
            SymBoolExprKind::Not(value) => {
                let value = value.fold_exprs(cx, folder);
                Self::not_bool(cx, value)
            }
            SymBoolExprKind::And(values) => {
                let values =
                    values.iter().cloned().map(|value| value.fold_exprs(cx, folder)).collect();
                Self::and(cx, values)
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                let left = left.fold(cx, folder);
                let right = right.fold(cx, folder);
                Self::cmp(cx, op, left, right)
            }
            SymBoolExprKind::Const(_) => unreachable!("leaf boolean returned before folding exprs"),
        }
    }

    #[cfg(test)]
    pub(crate) fn raw_and(cx: &mut SymCx, values: Vec<Self>) -> Self {
        Self::from_kind(cx, SymBoolExprKind::And(values.into()))
    }

    pub(crate) fn cmp_word_expr(
        cx: &mut SymCx,
        op: SymCmpOp,
        word: &SymExpr,
        expr: SymExpr,
    ) -> Self {
        Self::cmp(cx, op, word.clone(), expr)
    }

    pub(crate) fn not(self, cx: &mut SymCx) -> Self {
        Self::not_bool(cx, self)
    }

    pub(crate) fn collect_vars(&self, vars: &mut SymbolicVars) {
        let _ = self.visit_exprs(&mut |expr| {
            if let Some(var) = expr.kind().get_var() {
                vars.insert(var);
            }
            ControlFlow::<()>::Continue(())
        });
    }

    pub(crate) fn collect_eval_vars(&self, vars: &mut SymbolicVars) {
        let _ = self.visit_exprs(&mut |expr| {
            if let Some(var) = expr.kind().get_eval_var() {
                vars.insert(var);
            }
            ControlFlow::<()>::Continue(())
        });
    }

    pub(crate) fn smt(&self, cx: &SymCx) -> String {
        let mut smt = String::new();
        self.write_smt(cx, &mut smt);
        smt
    }

    pub(in crate::runtime::expr) fn write_smt(&self, cx: &SymCx, out: &mut String) {
        match self.kind() {
            SymBoolExprKind::Const(value) => out.push_str(if *value { "true" } else { "false" }),
            SymBoolExprKind::Not(value) => {
                out.push_str("(not ");
                value.write_smt(cx, out);
                out.push(')');
            }
            SymBoolExprKind::And(values) => {
                out.push_str("(and");
                for value in values.iter() {
                    out.push(' ');
                    value.write_smt(cx, out);
                }
                out.push(')');
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                left.write_smt(cx, out);
                out.push(' ');
                right.write_smt(cx, out);
                out.push(')');
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SymCmpOp {
    Eq,
    Ult,
    Ugt,
    Ule,
    Uge,
    Slt,
    Sgt,
}

impl SymCmpOp {
    pub(crate) const fn smt(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Ult => "bvult",
            Self::Ugt => "bvugt",
            Self::Ule => "bvule",
            Self::Uge => "bvuge",
            Self::Slt => "bvslt",
            Self::Sgt => "bvsgt",
        }
    }

    pub(crate) fn eval(self, left: U256, right: U256) -> bool {
        match self {
            Self::Eq => left == right,
            Self::Ult => left < right,
            Self::Ugt => left > right,
            Self::Ule => left <= right,
            Self::Uge => left >= right,
            Self::Slt => slt(left, right),
            Self::Sgt => slt(right, left),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_ite_equality_rejects_exponential_shared_dag() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let first = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x.clone(), y.clone());
        let second = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, x.clone(), y.clone());
        let third = SymBoolExpr::cmp(&mut cx, SymCmpOp::Eq, x, y);
        let zero = SymExpr::zero(&mut cx);
        let one = SymExpr::one(&mut cx);
        let mut shared = SymExpr::ite(&mut cx, first.clone(), zero.clone(), one.clone());

        for _ in 0..32 {
            let left = SymExpr::ite(&mut cx, first.clone(), shared.clone(), zero.clone());
            let right = SymExpr::ite(&mut cx, second.clone(), shared.clone(), one.clone());
            shared = SymExpr::ite(&mut cx, third.clone(), left, right);
        }

        let raw = SymBoolExpr::from_kind(
            &mut cx,
            SymBoolExprKind::Cmp(SymCmpOp::Eq, shared.clone(), one.clone()),
        );
        let expanded = SymBoolExpr::eq(&mut cx, shared, one);
        assert_eq!(expanded, raw);
    }

    #[test]
    fn constant_ite_equality_keeps_linear_chain_linear() {
        let mut cx = SymCx::new();
        let zero = SymExpr::zero(&mut cx);
        let one = SymExpr::one(&mut cx);
        let mut value = zero.clone();
        for index in 0..64 {
            let selector = SymExpr::var(&mut cx, &format!("selector_{index}"));
            let condition = SymBoolExpr::eq_word_const(&mut cx, &selector, U256::ZERO);
            value = SymExpr::ite(&mut cx, condition, value, one.clone());
        }

        let expanded = SymBoolExpr::eq(&mut cx, value, zero);
        let mut pending = vec![expanded];
        let mut visited = HashSet::<SymBoolExpr>::default();
        while let Some(expr) = pending.pop() {
            if !visited.insert(expr.clone()) {
                continue;
            }
            match expr.kind() {
                SymBoolExprKind::Not(value) => pending.push(value.clone()),
                SymBoolExprKind::And(values) => {
                    assert!(values.len() <= 2);
                    pending.extend(values.iter().cloned());
                }
                SymBoolExprKind::Const(_) | SymBoolExprKind::Cmp(_, _, _) => {}
            }
        }
        assert!(visited.len() < 2 * 64);
    }

    #[test]
    fn constant_ite_equality_stops_at_expansion_budget() {
        let mut cx = SymCx::new();
        let zero = SymExpr::zero(&mut cx);
        let one = SymExpr::one(&mut cx);
        let mut value = zero;
        for index in 0..MAX_CONSTANT_ITE_EQ_NODES {
            let selector = SymExpr::var(&mut cx, &format!("selector_{index}"));
            let condition = SymBoolExpr::eq_word_const(&mut cx, &selector, U256::ZERO);
            value = SymExpr::ite(&mut cx, condition, value, one.clone());
        }
        let two = SymExpr::constant(&mut cx, U256::from(2));
        let comparison = SymBoolExpr::eq(&mut cx, value, two);

        assert!(matches!(comparison.kind(), SymBoolExprKind::Cmp(SymCmpOp::Eq, _, _)));
    }
}
