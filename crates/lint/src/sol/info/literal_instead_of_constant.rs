use super::LiteralInsteadOfConstant;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use alloy_primitives::{Address, U256};
use solar::{
    ast::{LitKind, StrKind},
    interface::Span,
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind, Hir, Visit},
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
        let _ = gcx;
        // Group the literals of the contract's own function and modifier bodies by semantic
        // value; inherited items group with their declaring contract.
        let mut collector = LiteralCollector { hir, groups: HashMap::new() };
        for item_id in hir.contract(id).items {
            if let hir::ItemId::Function(function_id) = item_id
                && let Some(body) = &hir.function(*function_id).body
            {
                for stmt in body.stmts {
                    let _ = collector.visit_stmt(stmt);
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
#[derive(PartialEq, Eq, Hash)]
enum LiteralValue {
    Number(U256),
    Address(Address),
    HexString(Vec<u8>),
}

/// Collects the grouping-relevant literals of a subtree: numbers above 2, address literals
/// and hex string literals. A bare literal indexing an array stays out, matching Aderyn.
struct LiteralCollector<'hir> {
    hir: &'hir Hir<'hir>,
    groups: HashMap<LiteralValue, Vec<Span>>,
}

impl<'hir> Visit<'hir> for LiteralCollector<'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            // A bare literal used as an index (`arr[3]`) is positional, not a magic value.
            ExprKind::Index(base, index) => {
                let _ = self.visit_expr(base);
                if let Some(index) = index
                    && !matches!(index.peel_parens().kind, ExprKind::Lit(..))
                {
                    let _ = self.visit_expr(index);
                }
                return ControlFlow::Continue(());
            }
            ExprKind::Lit(lit) => {
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
            _ => {}
        }
        self.walk_expr(expr)
    }
}
