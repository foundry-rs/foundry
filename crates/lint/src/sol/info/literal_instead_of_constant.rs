use super::LiteralInsteadOfConstant;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use alloy_primitives::{Address, U256};
use solar::{
    ast::{BinOpKind, LitKind, StrKind, UnOpKind},
    interface::Span,
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind, Hir, Lit, Stmt, StmtKind, Visit},
        ty::TyKind,
    },
};
use std::{collections::HashMap, convert::Infallible, ops::ControlFlow};

declare_forge_lint!(
    LITERAL_INSTEAD_OF_CONSTANT,
    Severity::Info,
    "literal-instead-of-constant",
    "this literal appears multiple times in the contract; declare a named constant for it"
);

impl<'hir> LateLintPass<'hir> for LiteralInsteadOfConstant {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        id: hir::ContractId,
    ) {
        // Group the literals of the contract's own functions and modifiers by semantic value;
        // inherited items group with their declaring contract. Collection covers the executable
        // expressions: the body statements, and the modifier and base-constructor arguments of
        // the header. Parameter and return types stay out, so a fixed array size in a signature
        // is a type annotation rather than a repeated value.
        let mut collector = LiteralCollector { gcx, hir, groups: HashMap::new() };
        for item_id in hir.contract(id).items {
            if let hir::ItemId::Function(function_id) = item_id {
                let function = hir.function(*function_id);
                for modifier in function.modifiers {
                    let _ = collector.visit_call_args(&modifier.args);
                }
                if let Some(body) = &function.body {
                    for stmt in body.stmts {
                        let _ = collector.visit_stmt(stmt);
                    }
                }
            }
        }
        // A value used in one single place is fine: only repetitions report. Emissions are
        // sorted by position so the output does not depend on the map's iteration order.
        let mut repeated: Vec<Span> = Vec::new();
        for spans in collector.groups.into_values() {
            if spans.len() > 1 {
                repeated.extend(spans);
            }
        }
        repeated.sort_by_key(|span| span.lo());
        for span in repeated {
            ctx.emit(&LITERAL_INSTEAD_OF_CONSTANT, span);
        }
    }
}

/// The semantic value of a literal, the grouping key: two spellings of the same number
/// (`100`, `0x64`, `1e2`) or the same unit-scaled amount (`1 ether`, `1e18`) are one value.
/// A numeric literal under a value-changing unary operator denotes a DISTINCT constant, so
/// `-5` and `~5` never group with the bare `5`.
#[derive(PartialEq, Eq, Hash)]
enum LiteralValue {
    Number(U256),
    NegNumber(U256),
    BitNotNumber(U256),
    Address(Address),
    HexString(Vec<u8>),
}

/// Collects the grouping-relevant literals of a subtree: numbers above 2, address literals
/// and hex string literals. A bare literal indexing an array-like value or bounding a slice
/// stays out as positional, matching Aderyn; a mapping key counts, it is configuration data.
struct LiteralCollector<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    groups: HashMap<LiteralValue, Vec<Span>>,
}

impl<'hir> LiteralCollector<'hir> {
    /// Records one literal under its semantic grouping key.
    fn record_lit(&mut self, lit: &Lit<'_>) {
        let key = match &lit.kind {
            // `0`, `1` and `2` are structural rather than configuration values.
            LitKind::Number(v) if *v > U256::from(2u64) => Some(LiteralValue::Number(*v)),
            LitKind::Address(address) => Some(LiteralValue::Address(*address)),
            LitKind::Str(StrKind::Hex, bytes, _) => {
                Some(LiteralValue::HexString(bytes.as_byte_str().to_vec()))
            }
            _ => None,
        };
        if let Some(key) = key {
            self.groups.entry(key).or_default().push(lit.span);
        }
    }

    /// Whether an indexed base is a mapping, whose keys are configuration values rather than
    /// positions. The type checker tells mappings apart from arrays, bytes and fixed bytes.
    fn base_is_mapping(&self, base: &Expr<'_>) -> bool {
        matches!(
            self.gcx.type_of_expr(base.peel_parens().id).map(|ty| ty.peel_refs().kind),
            Some(TyKind::Mapping(..))
        )
    }
}

impl<'hir> Visit<'hir> for LiteralCollector<'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        // A Yul `case 500 {}` label is a literal like any other, but `case.constant` is a
        // `Lit` rather than an expression, so the default visitor never reaches it.
        if let StmtKind::Switch(switch) = &stmt.kind {
            for case in switch.cases {
                if let Some(constant) = &case.constant {
                    self.record_lit(constant);
                }
            }
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            // A bare literal indexing an array-like value (`arr[3]`) is positional, not a
            // magic value; a mapping key (`m[500]`) is configuration data and counts.
            ExprKind::Index(base, index) => {
                let _ = self.visit_expr(base);
                if let Some(index) = index
                    && (self.base_is_mapping(base)
                        || !matches!(index.peel_parens().kind, ExprKind::Lit(..)))
                {
                    let _ = self.visit_expr(index);
                }
                return ControlFlow::Continue(());
            }
            // A bare literal shift amount is structural rather than a configuration value
            // (`x << 128`, `acc >>= 128`): the shifted operand is walked normally, the
            // amount only when it is computed, so the literals inside it still count.
            ExprKind::Binary(lhs, op, rhs)
                if matches!(op.kind, BinOpKind::Shl | BinOpKind::Shr | BinOpKind::Sar) =>
            {
                let _ = self.visit_expr(lhs);
                if !matches!(rhs.peel_parens().kind, ExprKind::Lit(..)) {
                    let _ = self.visit_expr(rhs);
                }
                return ControlFlow::Continue(());
            }
            ExprKind::Assign(lhs, Some(op), rhs)
                if matches!(op.kind, BinOpKind::Shl | BinOpKind::Shr | BinOpKind::Sar) =>
            {
                let _ = self.visit_expr(lhs);
                if !matches!(rhs.peel_parens().kind, ExprKind::Lit(..)) {
                    let _ = self.visit_expr(rhs);
                }
                return ControlFlow::Continue(());
            }
            // Slice bounds share the positional rationale: slices only exist on array-like
            // values, so a bare literal bound (`d[555:600]`) is never a magic value.
            ExprKind::Slice(base, start, end) => {
                let _ = self.visit_expr(base);
                // Each bound present is walked unless it is a bare literal.
                for bound in [start, end].into_iter().flatten() {
                    if !matches!(bound.peel_parens().kind, ExprKind::Lit(..)) {
                        let _ = self.visit_expr(bound);
                    }
                }
                return ControlFlow::Continue(());
            }
            // A value-changing unary operator on a numeric literal denotes a DISTINCT constant, so
            // `-5`/`~5` must not group with the bare `5`.
            ExprKind::Unary(op, operand) if matches!(op.kind, UnOpKind::Neg | UnOpKind::BitNot) => {
                match &operand.peel_parens().kind {
                    // `-5` / `~5`: record the operator-qualified value; do not descend into the
                    // operand, which would re-record the bare magnitude.
                    ExprKind::Lit(lit) => {
                        if let LitKind::Number(v) = &lit.kind
                            && *v > U256::from(2u64)
                        {
                            let key = if op.kind == UnOpKind::Neg {
                                LiteralValue::NegNumber(*v)
                            } else {
                                LiteralValue::BitNotNumber(*v)
                            };
                            self.groups.entry(key).or_default().push(expr.span);
                        }
                        return ControlFlow::Continue(());
                    }
                    // A nested unary over a literal (`-(-5)`, `~~5`) folds to a value that is
                    // neither this operator's nor the bare literal's; canonicalizing it is not
                    // worth it, so the chain is skipped rather than miss-keyed. A non-literal
                    // operand deeper down (`-(-(x + 500))`) falls through so its own literals
                    // are still recorded.
                    ExprKind::Unary(inner, inner_operand)
                        if matches!(inner.kind, UnOpKind::Neg | UnOpKind::BitNot)
                            && matches!(inner_operand.peel_parens().kind, ExprKind::Lit(..)) =>
                    {
                        return ControlFlow::Continue(());
                    }
                    // Any other operand (`-x`, `-(a + 5)`): walk normally so a literal inside it is
                    // still recorded with its own value.
                    _ => {}
                }
            }
            ExprKind::Lit(lit) => self.record_lit(lit),
            _ => {}
        }
        self.walk_expr(expr)
    }
}
