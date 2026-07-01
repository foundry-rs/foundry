use super::{super::hashcons::HashConsed, *};

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
    Eq(SymExpr, SymExpr),
    Cmp(SymBoolExprOp, SymExpr, SymExpr),
}

impl SymBoolExpr {
    pub(in crate::runtime) fn kind(&self) -> &SymBoolExprKind {
        self.kind.value()
    }

    #[cfg(test)]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        self.kind.ptr_eq(&other.kind)
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
        op: SymBoolExprOp,
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
        match (left.kind(), right.kind()) {
            _ if left == right => Self::constant(cx, true),
            (SymExprKind::Const(left), SymExprKind::Const(right)) => {
                Self::constant(cx, left == right)
            }
            (_, SymExprKind::Const(right_value)) => {
                if let Some(condition) = Self::bool_word_eq_const(cx, &left, *right_value) {
                    return condition;
                }
                if let Some(left_value) = left.known_word() {
                    return Self::constant(cx, left_value == *right_value);
                }
                Self::from_kind(cx, SymBoolExprKind::Eq(left, right))
            }
            (SymExprKind::Const(left_value), _) => {
                if let Some(condition) = Self::bool_word_eq_const(cx, &right, *left_value) {
                    return condition;
                }
                if let Some(right_value) = right.known_word() {
                    return Self::constant(cx, *left_value == right_value);
                }
                Self::from_kind(cx, SymBoolExprKind::Eq(left, right))
            }
            (
                SymExprKind::Keccak { len: left_len, bytes: left_bytes, .. },
                SymExprKind::Keccak { len: right_len, bytes: right_bytes, .. },
            ) if left_bytes.len() == right_bytes.len() => {
                let mut conditions = vec![Self::eq(cx, left_len.clone(), right_len.clone())];
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
                SymExprKind::Hash { algorithm: right_algorithm, bytes: right_bytes, .. },
            ) if left_algorithm == right_algorithm && left_bytes.len() == right_bytes.len() => {
                let conditions = left_bytes
                    .iter()
                    .cloned()
                    .zip(right_bytes.iter().cloned())
                    .map(|(left, right)| Self::eq(cx, left, right))
                    .collect();
                Self::and(cx, conditions)
            }
            _ => Self::from_kind(cx, SymBoolExprKind::Eq(left, right)),
        }
    }

    pub(crate) fn cmp(cx: &mut SymCx, op: SymBoolExprOp, left: SymExpr, right: SymExpr) -> Self {
        match (op, left.kind(), right.kind()) {
            (op, _, _) if left == right => {
                Self::constant(cx, matches!(op, SymBoolExprOp::Ule | SymBoolExprOp::Uge))
            }
            (op, SymExprKind::Const(left), SymExprKind::Const(right)) => {
                Self::constant(cx, op.eval(*left, *right))
            }
            (SymBoolExprOp::Ugt, SymExprKind::Const(value), _) if value.is_zero() => {
                Self::constant(cx, false)
            }
            (SymBoolExprOp::Ule, SymExprKind::Const(value), _) if value.is_zero() => {
                Self::constant(cx, true)
            }
            (SymBoolExprOp::Ult, _, SymExprKind::Const(value)) if value.is_zero() => {
                Self::constant(cx, false)
            }
            (SymBoolExprOp::Uge, _, SymExprKind::Const(value)) if value.is_zero() => {
                Self::constant(cx, true)
            }
            (SymBoolExprOp::Ult, SymExprKind::Const(value), _) if *value == U256::MAX => {
                Self::constant(cx, false)
            }
            (SymBoolExprOp::Uge, SymExprKind::Const(value), _) if *value == U256::MAX => {
                Self::constant(cx, true)
            }
            (SymBoolExprOp::Ugt, _, SymExprKind::Const(value)) if *value == U256::MAX => {
                Self::constant(cx, false)
            }
            (SymBoolExprOp::Ule, _, SymExprKind::Const(value)) if *value == U256::MAX => {
                Self::constant(cx, true)
            }
            _ => Self::from_kind(cx, SymBoolExprKind::Cmp(op, left, right)),
        }
    }

    pub(crate) fn and(cx: &mut SymCx, values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value.kind() {
                SymBoolExprKind::Const(true) => {}
                SymBoolExprKind::Const(false) => return Self::constant(cx, false),
                SymBoolExprKind::And(values) => out.extend(values.iter().cloned()),
                _ => out.push(value),
            }
        }
        if out.is_empty() {
            Self::constant(cx, true)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            Self::from_kind(cx, SymBoolExprKind::And(out.into()))
        }
    }

    pub(crate) fn or(cx: &mut SymCx, values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value.kind() {
                SymBoolExprKind::Const(false) => {}
                SymBoolExprKind::Const(true) => return Self::constant(cx, true),
                _ => out.push(value),
            }
        }
        if out.is_empty() {
            Self::constant(cx, false)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            let values = out.into_iter().map(|value| Self::not_bool(cx, value)).collect();
            let and = Self::and(cx, values);
            Self::not_bool(cx, and)
        }
    }

    pub(crate) fn not_bool(cx: &mut SymCx, value: Self) -> Self {
        match value.kind() {
            SymBoolExprKind::Const(value) => Self::constant(cx, !*value),
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

    pub(crate) fn as_const(&self) -> Option<bool> {
        match self.kind() {
            SymBoolExprKind::Const(value) => Some(*value),
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
            SymBoolExprKind::Eq(left, right) => match (left.kind(), right.kind()) {
                (_, SymExprKind::Const(value)) => left.equality_forces_const(*value, expr, context),
                (SymExprKind::Const(value), _) => {
                    right.equality_forces_const(*value, expr, context)
                }
                _ => None,
            },
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Eq(left, right) => match (left.kind(), right.kind()) {
                    (_, SymExprKind::Const(value)) if value.is_zero() => {
                        left.nonzero_forces_const(expr, context)
                    }
                    (SymExprKind::Const(value), _) if value.is_zero() => {
                        right.nonzero_forces_const(expr, context)
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
            SymBoolExprKind::Eq(left, right) => match (left == expr, right == expr) {
                (true, _) => right.eval().and_then(|value| usize::try_from(value).ok()),
                (_, true) => left.eval().and_then(|value| usize::try_from(value).ok()),
                _ => None,
            },
            SymBoolExprKind::Cmp(op, left, right) => {
                if left == expr {
                    match *op {
                        SymBoolExprOp::Ult => right
                            .eval()
                            .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                            .and_then(|value| usize::try_from(value).ok()),
                        SymBoolExprOp::Ule => {
                            right.eval().and_then(|value| usize::try_from(value).ok())
                        }
                        _ => None,
                    }
                } else if right == expr {
                    match *op {
                        SymBoolExprOp::Ugt => left
                            .eval()
                            .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                            .and_then(|value| usize::try_from(value).ok()),
                        SymBoolExprOp::Uge => {
                            left.eval().and_then(|value| usize::try_from(value).ok())
                        }
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
        Ok(match self.kind() {
            SymBoolExprKind::Const(value) => *value,
            SymBoolExprKind::Not(value) => !value.eval_model(model)?,
            SymBoolExprKind::And(values) => {
                for value in values.iter() {
                    if !value.eval_model(model)? {
                        return Ok(false);
                    }
                }
                true
            }
            SymBoolExprKind::Eq(left, right) => {
                left.eval_model(model)? == right.eval_model(model)?
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                op.eval(left.eval_model(model)?, right.eval_model(model)?)
            }
        })
    }

    pub(crate) fn eval_model_if_complete<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<Option<bool>, SymbolicError> {
        let mut vars = SymbolicVars::default();
        self.collect_eval_vars(&mut vars);
        if vars.iter().cloned().all(|var| model.contains_name(var)) {
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
            SymBoolExprKind::Eq(left, right) | SymBoolExprKind::Cmp(_, left, right) => {
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
            SymBoolExprKind::Eq(left, right) => Self::eq(cx, left, right),
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
            SymBoolExprKind::Eq(left, right) => {
                let left = left.fold(cx, folder);
                let right = right.fold(cx, folder);
                Self::eq(cx, left, right)
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
        op: SymBoolExprOp,
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
            match expr.kind() {
                SymExprKind::Var(var)
                | SymExprKind::Keccak { name: var, .. }
                | SymExprKind::Hash { name: var, .. } => {
                    vars.insert(var.clone());
                }
                _ => {}
            }
            ControlFlow::<()>::Continue(())
        });
    }

    pub(crate) fn collect_eval_vars(&self, vars: &mut SymbolicVars) {
        let _ = self.visit_exprs(&mut |expr| {
            match expr.kind() {
                SymExprKind::Var(var) | SymExprKind::Hash { name: var, .. } => {
                    vars.insert(var.clone());
                }
                _ => {}
            }
            ControlFlow::<()>::Continue(())
        });
    }

    pub(crate) fn smt(&self) -> String {
        let mut smt = String::new();
        self.write_smt(&mut smt);
        smt
    }

    pub(in crate::runtime::expr) fn write_smt(&self, out: &mut String) {
        match self.kind() {
            SymBoolExprKind::Const(value) => out.push_str(if *value { "true" } else { "false" }),
            SymBoolExprKind::Not(value) => {
                out.push_str("(not ");
                value.write_smt(out);
                out.push(')');
            }
            SymBoolExprKind::And(values) => {
                out.push_str("(and");
                for value in values.iter() {
                    out.push(' ');
                    value.write_smt(out);
                }
                out.push(')');
            }
            SymBoolExprKind::Eq(left, right) => {
                out.push_str("(= ");
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SymBoolExprOp {
    Ult,
    Ugt,
    Ule,
    Uge,
    Slt,
    Sgt,
}

impl SymBoolExprOp {
    pub(crate) const fn smt(self) -> &'static str {
        match self {
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
            Self::Ult => left < right,
            Self::Ugt => left > right,
            Self::Ule => left <= right,
            Self::Uge => left >= right,
            Self::Slt => slt(left, right),
            Self::Sgt => slt(right, left),
        }
    }
}
