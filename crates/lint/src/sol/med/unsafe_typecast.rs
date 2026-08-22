use super::UnsafeTypecast;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint, analysis::primitives::branch_always_exits},
};
use alloy_primitives::{U256, map::HashMap};
use solar::{
    ast::{BinOpKind, LitKind, StrKind, UnOpKind},
    interface::{Span, data_structures::Never},
    sema::{
        Gcx, Hir,
        eval::ConstantEvaluator,
        hir::{
            self, Contract, ElementaryType, Expr, ExprKind, Function, ItemId, Res, SourceId, Stmt,
            StmtKind, TypeKind, VariableId, Visit,
        },
        ty::TyKind,
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(
    UNSAFE_TYPECAST,
    Severity::Med,
    "unsafe-typecast",
    "typecasts that can truncate values should be checked"
);

impl<'hir> LateLintPass<'hir> for UnsafeTypecast {
    fn check_nested_source(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        id: SourceId,
    ) {
        for &item in hir.source(id).items {
            if let ItemId::Variable(id) = item
                && let Some(initializer) = hir.variable(id).initializer
            {
                emit_findings(ctx, Checker::check_expr(gcx, hir, initializer));
            }
        }
    }

    fn check_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        contract: &'hir Contract<'hir>,
    ) {
        for &item in contract.items {
            if let ItemId::Variable(id) = item
                && let Some(initializer) = hir.variable(id).initializer
            {
                emit_findings(ctx, Checker::check_expr(gcx, hir, initializer));
            }
        }
        for modifier in contract.bases_args {
            emit_findings(ctx, Checker::check_call_args(gcx, hir, &modifier.args));
        }
    }

    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        emit_findings(ctx, Checker::check_function(gcx, hir, func));
    }
}

fn emit_findings(ctx: &LintContext<'_, '_>, findings: Vec<(Span, ElementaryType)>) {
    for (span, ty) in findings {
        ctx.emit_with_suggestion(
            &UNSAFE_TYPECAST,
            span,
            Suggestion::example(
                format!(
                    "// casting to '{abi_ty}' is safe because [explain why]\n// forge-lint: disable-next-line(unsafe-typecast)",
                    abi_ty = ty.to_abi_str()
                ),
            )
            .with_desc("consider disabling this lint if you're certain the cast is safe"),
        );
    }
}

/// Performs one structured forward pass over a function while carrying proven unsigned upper
/// bounds. Facts are intersected at joins and invalidated before every possible local mutation.
struct Checker<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    upper_bounds: HashMap<VariableId, U256>,
    findings: Vec<(Span, ElementaryType)>,
}

impl<'hir> Checker<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir Hir<'hir>) -> Self {
        Self { gcx, hir, upper_bounds: HashMap::default(), findings: Vec::new() }
    }

    fn check_function(
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) -> Vec<(Span, ElementaryType)> {
        let mut this = Self::new(gcx, hir);
        for modifier in func.modifiers {
            let _ = this.visit_call_args(&modifier.args);
        }
        if let Some(body) = func.body {
            this.visit_block(body);
        }
        this.findings
    }

    fn check_expr(
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        expr: &'hir Expr<'hir>,
    ) -> Vec<(Span, ElementaryType)> {
        let mut this = Self::new(gcx, hir);
        let _ = this.visit_expr(expr);
        this.findings
    }

    fn check_call_args(
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        args: &'hir hir::CallArgs<'hir>,
    ) -> Vec<(Span, ElementaryType)> {
        let mut this = Self::new(gcx, hir);
        let _ = this.visit_call_args(args);
        this.findings
    }

    fn visit_block(&mut self, block: hir::Block<'hir>) {
        for stmt in block.stmts {
            let _ = self.visit_stmt(stmt);
            if branch_always_exits(stmt) {
                break;
            }
        }
    }

    fn refine(&mut self, fact: Option<(VariableId, U256)>) {
        let Some((id, upper)) = fact else { return };
        if !self.is_unsigned_local(id) {
            return;
        }
        self.upper_bounds
            .entry(id)
            .and_modify(|current| *current = (*current).min(upper))
            .or_insert(upper);
    }

    fn merge_bounds(
        lhs: &HashMap<VariableId, U256>,
        rhs: &HashMap<VariableId, U256>,
    ) -> HashMap<VariableId, U256> {
        lhs.iter()
            .filter_map(|(&id, &left)| rhs.get(&id).map(|&right| (id, left.max(right))))
            .collect()
    }

    fn is_unsigned_local(&self, id: VariableId) -> bool {
        let var = self.hir.variable(id);
        var.parent.is_some() && matches!(var.ty.kind, TypeKind::Elementary(ElementaryType::UInt(_)))
    }

    fn cast_is_bounded(&self, source: &Expr<'hir>, target: &ElementaryType) -> bool {
        let ElementaryType::UInt(size) = target else { return false };
        let Some(id) = resolved_variable(source) else { return false };
        self.is_unsigned_local(id)
            && self.upper_bounds.get(&id).is_some_and(|&upper| upper <= uint_max(size.bits()))
    }

    fn invalidate_lvalue(&mut self, expr: &Expr<'hir>) {
        match &expr.peel_parens().kind {
            ExprKind::Ident(reses) => {
                for res in *reses {
                    if let Res::Item(ItemId::Variable(id)) = res {
                        self.upper_bounds.remove(id);
                    }
                }
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().flatten() {
                    self.invalidate_lvalue(expr);
                }
            }
            _ => {}
        }
    }
}

impl<'hir> Visit<'hir> for Checker<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match &stmt.kind {
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.visit_block(*block);
                return ControlFlow::Continue(());
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                let _ = self.visit_expr(cond);
                let before = self.upper_bounds.clone();

                self.refine(comparison_upper_bound(self.gcx, cond, true));
                let _ = self.visit_stmt(then_stmt);
                let after_then = self.upper_bounds.clone();

                self.upper_bounds = before;
                self.refine(comparison_upper_bound(self.gcx, cond, false));
                if let Some(else_stmt) = else_stmt {
                    let _ = self.visit_stmt(else_stmt);
                }
                let after_else = self.upper_bounds.clone();

                let then_exits = branch_always_exits(then_stmt);
                let else_exits = else_stmt.is_some_and(branch_always_exits);
                self.upper_bounds = match (then_exits, else_exits) {
                    (true, false) => after_else,
                    (false, true) => after_then,
                    (false, false) => Self::merge_bounds(&after_then, &after_else),
                    (true, true) => HashMap::default(),
                };
                return ControlFlow::Continue(());
            }
            // Repeated or multi-way control flow needs a fixed point to retain facts soundly.
            // Until that is shared by the HIR analyses, discard facts conservatively.
            StmtKind::Loop(..)
            | StmtKind::AssemblyBlock(..)
            | StmtKind::Switch(..)
            | StmtKind::Try(..) => {
                self.upper_bounds.clear();
                let result = self.walk_stmt(stmt);
                self.upper_bounds.clear();
                return result;
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Call(call, args, _) = &expr.kind
            && let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) = &call.kind
            && args.len() == 1
            && let Some(source) = args.exprs().next()
            && is_unsafe_typecast_hir(self.gcx, source, ty)
            && !self.cast_is_bounded(source, ty)
        {
            self.findings.push((expr.span, *ty));
        }

        match &expr.kind {
            ExprKind::Assign(lhs, _, _) | ExprKind::Delete(lhs) => {
                self.invalidate_lvalue(lhs);
            }
            ExprKind::Unary(op, target)
                if matches!(
                    op.kind,
                    UnOpKind::PreInc | UnOpKind::PostInc | UnOpKind::PreDec | UnOpKind::PostDec
                ) =>
            {
                self.invalidate_lvalue(target);
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

fn comparison_upper_bound(
    gcx: Gcx<'_>,
    cond: &Expr<'_>,
    truthy: bool,
) -> Option<(VariableId, U256)> {
    let ExprKind::Binary(lhs, op, rhs) = &cond.peel_parens().kind else { return None };

    let (id, limit, op) = if let Some(id) = resolved_variable(lhs) {
        (id, *rhs, op.kind)
    } else {
        let id = resolved_variable(rhs)?;
        let reversed = match op.kind {
            BinOpKind::Lt => BinOpKind::Gt,
            BinOpKind::Le => BinOpKind::Ge,
            BinOpKind::Gt => BinOpKind::Lt,
            BinOpKind::Ge => BinOpKind::Le,
            _ => return None,
        };
        (id, *lhs, reversed)
    };

    let limit = eval_u256(gcx, limit)?;
    let upper = match (op, truthy) {
        (BinOpKind::Le, true) | (BinOpKind::Gt, false) => limit,
        (BinOpKind::Lt, true) | (BinOpKind::Ge, false) => limit.checked_sub(U256::from(1))?,
        _ => return None,
    };
    Some((id, upper))
}

fn eval_u256(gcx: Gcx<'_>, expr: &Expr<'_>) -> Option<U256> {
    if let Ok(value) = ConstantEvaluator::new(gcx).try_eval(expr)
        && let Some(value) = value.as_u256()
    {
        return Some(value);
    }

    let ExprKind::Member(receiver, member) = &expr.peel_parens().kind else { return None };
    let ExprKind::TypeCall(hir::Type { kind: TypeKind::Elementary(ty), .. }) = &receiver.kind
    else {
        return None;
    };
    match (member.name.as_str(), ty) {
        ("max", ElementaryType::UInt(size)) => Some(uint_max(size.bits())),
        ("max", ElementaryType::Int(size)) => {
            Some((U256::from(1) << (size.bits() - 1)) - U256::from(1))
        }
        ("min", ElementaryType::UInt(_)) => Some(U256::ZERO),
        _ => None,
    }
}

fn uint_max(bits: u16) -> U256 {
    if bits == 256 { U256::MAX } else { (U256::from(1) << bits) - U256::from(1) }
}

fn resolved_variable(expr: &Expr<'_>) -> Option<VariableId> {
    let ExprKind::Ident([Res::Item(ItemId::Variable(id))]) = expr.peel_parens().kind else {
        return None;
    };
    Some(*id)
}

/// Determines if a typecast is potentially unsafe (could lose data or precision).
fn is_unsafe_typecast_hir<'hir>(
    gcx: Gcx<'hir>,
    source_expr: &hir::Expr<'hir>,
    target_type: &hir::ElementaryType,
) -> bool {
    if is_bounded_by_mask(source_expr, target_type) {
        return false;
    }

    let mut source_types = Vec::<ElementaryType>::new();
    infer_source_types(Some(&mut source_types), gcx, source_expr);

    if source_types.is_empty() {
        return false;
    };

    source_types.iter().any(|source_ty| is_unsafe_elementary_typecast(source_ty, target_type))
}

/// Returns whether a bitmask bounds an unsigned integer expression to the target type's range.
fn is_bounded_by_mask(source_expr: &hir::Expr<'_>, target_type: &ElementaryType) -> bool {
    let ElementaryType::UInt(target_size) = target_type else { return false };
    let ExprKind::Binary(lhs, op, rhs) = &source_expr.peel_parens().kind else { return false };
    if op.kind != BinOpKind::BitAnd {
        return false;
    }

    [lhs, rhs].into_iter().any(|expr| {
        matches!(
            expr.peel_parens().kind,
            ExprKind::Lit(hir::Lit { kind: LitKind::Number(mask), .. })
                if mask.bit_len() <= target_size.bits() as usize
        )
    })
}

/// Infers the elementary source type(s) of an expression.
///
/// This function traverses an expression tree to find the original "source" types.
/// For cast chains, it returns the ultimate source type, not intermediate cast results.
/// For binary operations, it collects types from both sides into the `output` vector.
///
/// # Returns
/// An `Option<ElementaryType>` containing the inferred type of the expression if it can be
/// resolved to a single source (like variables, literals, or unary expressions).
/// Returns `None` for expressions complex expressions (like binary operations).
fn infer_source_types<'hir>(
    mut output: Option<&mut Vec<ElementaryType>>,
    gcx: Gcx<'hir>,
    expr: &hir::Expr<'hir>,
) -> Option<ElementaryType> {
    let mut track = |ty: ElementaryType| -> Option<ElementaryType> {
        if let Some(output) = output.as_mut() {
            output.push(ty);
        }
        Some(ty)
    };

    match &expr.kind {
        // A type cast call: `Type(val)`
        ExprKind::Call(call_expr, args, ..) => {
            // Check if the called expression is a type, which indicates a cast.
            if let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(..), .. }) =
                &call_expr.kind
                && let Some(inner) = args.exprs().next()
            {
                // Recurse to find the original (inner-most) source type.
                return infer_source_types(output, gcx, inner);
            }
            expr_elementary_type(gcx, expr).and_then(track)
        }

        // Handle string literals explicitly; Solar records them as literal types rather than
        // elementary `string`/`bytes`.
        ExprKind::Lit(hir::Lit { kind, .. }) => match kind {
            LitKind::Str(StrKind::Hex, ..) => track(ElementaryType::Bytes),
            LitKind::Str(..) => track(ElementaryType::String),
            _ => expr_elementary_type(gcx, expr).and_then(track),
        },

        // Identifiers and other simple typed expressions.
        ExprKind::Ident(_) => expr_elementary_type(gcx, expr).and_then(track),

        // Unary operations: Recurse to find the source type of the inner expression.
        ExprKind::Unary(_, inner_expr) => infer_source_types(output, gcx, inner_expr),

        // Binary operations
        ExprKind::Binary(lhs, _, rhs) => {
            if let Some(mut output) = output {
                // Recurse on both sides to find and collect all source types.
                infer_source_types(Some(&mut output), gcx, lhs);
                infer_source_types(Some(&mut output), gcx, rhs);
            }
            None
        }

        _ => expr_elementary_type(gcx, expr).and_then(track),
    }
}

fn expr_elementary_type<'hir>(gcx: Gcx<'hir>, expr: &hir::Expr<'hir>) -> Option<ElementaryType> {
    match gcx.type_of_expr(expr.peel_parens().id)?.peel_refs().kind {
        TyKind::Elementary(ty) => Some(ty),
        TyKind::StringLiteral(true, _) => Some(ElementaryType::String),
        TyKind::StringLiteral(false, _) => Some(ElementaryType::Bytes),
        _ => None,
    }
}

/// Checks if a type cast from source_type to target_type is unsafe.
const fn is_unsafe_elementary_typecast(
    source_type: &ElementaryType,
    target_type: &ElementaryType,
) -> bool {
    match (source_type, target_type) {
        // Numeric downcasts (smaller target size)
        (ElementaryType::UInt(source_size), ElementaryType::UInt(target_size))
        | (ElementaryType::Int(source_size), ElementaryType::Int(target_size)) => {
            source_size.bits() > target_size.bits()
        }

        // Signed to unsigned conversion (potential loss of sign)
        (ElementaryType::Int(_), ElementaryType::UInt(_)) => true,

        // Unsigned to signed conversion with same or smaller size
        (ElementaryType::UInt(source_size), ElementaryType::Int(target_size)) => {
            source_size.bits() >= target_size.bits()
        }

        // Fixed bytes to smaller fixed bytes
        (ElementaryType::FixedBytes(source_size), ElementaryType::FixedBytes(target_size)) => {
            source_size.bytes() > target_size.bytes()
        }

        // Dynamic bytes to fixed bytes (potential truncation)
        (ElementaryType::Bytes | ElementaryType::String, ElementaryType::FixedBytes(_)) => true,

        // Address to smaller uint (truncation) - address is 160 bits
        (ElementaryType::Address(_), ElementaryType::UInt(target_size)) => target_size.bits() < 160,

        // Address to int (sign issues)
        (ElementaryType::Address(_), ElementaryType::Int(_)) => true,

        _ => false,
    }
}
