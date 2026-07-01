use super::{hashcons::HashConsed, *};
use std::hash::{Hash, Hasher};

#[derive(Clone)]
pub(crate) struct SymBoolExpr {
    pub(in crate::runtime::expr) kind: HashConsed<SymBoolExprKind>,
}

impl PartialEq for SymBoolExpr {
    fn eq(&self, other: &Self) -> bool {
        self.kind.ptr_eq(&other.kind)
    }
}

impl Eq for SymBoolExpr {}

impl Hash for SymBoolExpr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
    }
}

impl PartialOrd for SymBoolExpr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SymBoolExpr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.kind.cmp(&other.kind)
    }
}

impl fmt::Debug for SymBoolExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
                cx.not_bool(value)
            }
            SymBoolExprKind::And(values) => {
                let values = values.iter().cloned().map(|value| value.fold(cx, folder)).collect();
                cx.and(values)
            }
            SymBoolExprKind::Eq(left, right) => cx.eq(left, right),
            SymBoolExprKind::Cmp(op, left, right) => cx.cmp(op, left, right),
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
                cx.not_bool(value)
            }
            SymBoolExprKind::And(values) => {
                let values =
                    values.iter().cloned().map(|value| value.fold_exprs(cx, folder)).collect();
                cx.and(values)
            }
            SymBoolExprKind::Eq(left, right) => {
                let left = left.fold(cx, folder);
                let right = right.fold(cx, folder);
                cx.eq(left, right)
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                let left = left.fold(cx, folder);
                let right = right.fold(cx, folder);
                cx.cmp(op, left, right)
            }
            SymBoolExprKind::Const(_) => unreachable!("leaf boolean returned before folding exprs"),
        }
    }

    #[cfg(test)]
    pub(crate) fn raw_and(cx: &mut SymCx, values: Vec<Self>) -> Self {
        cx.bool_from_kind(SymBoolExprKind::And(values.into()))
    }

    pub(crate) fn cmp_word_expr(
        cx: &mut SymCx,
        op: SymBoolExprOp,
        word: &SymExpr,
        expr: SymExpr,
    ) -> Self {
        cx.cmp(op, word.clone(), expr)
    }

    pub(crate) fn not(self, cx: &mut SymCx) -> Self {
        cx.not_bool(self)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
