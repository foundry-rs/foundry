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

impl<'gcx> LateLintPass<'gcx> for LiteralInsteadOfConstant {
    fn check_nested_contract(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, id: hir::ContractId) {
        // Group the literals of the contract's own functions and modifiers by semantic value;
        // inherited items group with their declaring contract. Collection covers the executable
        // expressions: the body statements, and the modifier and base-constructor arguments of
        // the header. Parameter and return types stay out, so a fixed array size in a signature
        // is a type annotation rather than a repeated value.
        let mut collector = LiteralCollector { gcx, groups: HashMap::new() };
        let functions = gcx.hir.contract(id).items.iter().filter_map(|item| item.as_function());
        for function in functions.map(|id| gcx.hir.function(id)) {
            for modifier in function.modifiers {
                let _ = collector.visit_modifier(modifier);
            }
            for stmt in function.body.iter().flat_map(|body| body.stmts) {
                let _ = collector.visit_stmt(stmt);
            }
        }
        // A value used in one single place is fine: only repetitions report. Emissions are
        // sorted by position so the output does not depend on the map's iteration order.
        let mut repeated: Vec<Span> =
            collector.groups.into_values().filter(|spans| spans.len() > 1).flatten().collect();
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
    Number(Option<UnOpKind>, U256),
    Address(Address),
    HexString(Vec<u8>),
}

/// Collects the grouping-relevant literals of a subtree: numbers above 2, address literals
/// and hex string literals. A bare literal indexing an array-like value or bounding a slice
/// stays out as positional, matching Aderyn; a mapping key counts, it is configuration data.
struct LiteralCollector<'gcx> {
    gcx: Gcx<'gcx>,
    groups: HashMap<LiteralValue, Vec<Span>>,
}

impl<'gcx> LiteralCollector<'gcx> {
    /// Records one literal under its semantic grouping key, `op` being the value-changing
    /// unary operator applied to it, if any.
    fn record_lit(&mut self, lit: &Lit<'_>, span: Span, op: Option<UnOpKind>) {
        let key = match &lit.kind {
            // `0`, `1` and `2` are structural rather than configuration values.
            LitKind::Number(v) if *v > U256::from(2u64) => LiteralValue::Number(op, *v),
            LitKind::Address(address) => LiteralValue::Address(*address),
            LitKind::Str(StrKind::Hex, bytes, _) => {
                LiteralValue::HexString(bytes.as_byte_str().to_vec())
            }
            _ => return,
        };
        self.groups.entry(key).or_default().push(span);
    }

    /// Walks `expr` unless it is a bare literal in a positional role.
    fn visit_unless_bare_lit(&mut self, expr: &'gcx Expr<'gcx>) {
        if !is_lit(expr) {
            let _ = self.visit_expr(expr);
        }
    }
}

fn is_lit(expr: &Expr<'_>) -> bool {
    matches!(expr.peel_parens().kind, ExprKind::Lit(..))
}

impl<'gcx> Visit<'gcx> for LiteralCollector<'gcx> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Self::BreakValue> {
        // Yul literals commonly encode structural values such as memory offsets, masks, and
        // selectors, where extracting a constant would not necessarily improve readability.
        if matches!(stmt.kind, StmtKind::AssemblyBlock(_)) {
            return ControlFlow::Continue(());
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        let is_shift =
            |op: &hir::BinOp| matches!(op.kind, BinOpKind::Shl | BinOpKind::Shr | BinOpKind::Sar);
        let is_value_changing =
            |op: &hir::UnOp| matches!(op.kind, UnOpKind::Neg | UnOpKind::BitNot);
        match &expr.kind {
            // A bare literal indexing an array-like value (`arr[3]`) is positional, not a
            // magic value; a mapping key (`m[500]`) is configuration data and counts.
            ExprKind::Index(base, index) => {
                let _ = self.visit_expr(base);
                if let Some(index) = index {
                    let is_mapping = matches!(
                        self.gcx.type_of_expr(base.peel_parens().id).map(|ty| ty.peel_refs().kind),
                        Some(TyKind::Mapping(..))
                    );
                    if is_mapping {
                        let _ = self.visit_expr(index);
                    } else {
                        self.visit_unless_bare_lit(index);
                    }
                }
            }
            // A bare literal shift amount (`x << 128`, `acc >>= 128`) and bare slice bounds
            // (`d[555:600]`) are structural too: slices only exist on array-like values.
            ExprKind::Binary(lhs, op, rhs) | ExprKind::Assign(lhs, Some(op), rhs)
                if is_shift(op) =>
            {
                let _ = self.visit_expr(lhs);
                self.visit_unless_bare_lit(rhs);
            }
            ExprKind::Slice(base, start, end) => {
                let _ = self.visit_expr(base);
                for bound in [start, end].into_iter().flatten() {
                    self.visit_unless_bare_lit(bound);
                }
            }
            ExprKind::Unary(op, operand) if is_value_changing(op) => {
                match &operand.peel_parens().kind {
                    // `-5` / `~5`: record the operator-qualified value without descending into the
                    // operand, which would re-record the bare magnitude.
                    ExprKind::Lit(lit) => self.record_lit(lit, expr.span, Some(op.kind)),
                    // A nested unary over a literal (`-(-5)`, `~~5`) folds to a value that is
                    // neither this operator's nor the bare literal's; canonicalizing it is not
                    // worth it, so the chain is skipped rather than miss-keyed. A non-literal
                    // operand deeper down (`-(-(x + 500))`) still records its own literals.
                    ExprKind::Unary(inner, inner_operand)
                        if is_value_changing(inner) && is_lit(inner_operand) => {}
                    _ => return self.walk_expr(expr),
                }
            }
            ExprKind::Lit(lit) => self.record_lit(lit, lit.span, None),
            _ => return self.walk_expr(expr),
        }
        ControlFlow::Continue(())
    }
}
