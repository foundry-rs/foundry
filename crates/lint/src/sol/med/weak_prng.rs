use super::WeakPrng;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOp, BinOpKind, Expr, ExprKind, IndexKind, ItemFunction, visit::Visit},
    interface::SpannedOption,
};
use std::ops::ControlFlow;

declare_forge_lint!(
    WEAK_PRNG,
    Severity::Med,
    "weak-prng",
    "weak randomness derived from a predictable on-chain value"
);

impl<'ast> EarlyLintPass<'ast> for WeakPrng {
    fn check_item_function(&mut self, ctx: &LintContext, func: &'ast ItemFunction<'ast>) {
        if let Some(body) = &func.body {
            let mut checker = WeakPrngChecker { ctx };
            let _ = checker.visit_block(body);
        }
    }
}

struct WeakPrngChecker<'a, 's> {
    ctx: &'a LintContext<'s, 'a>,
}

impl<'ast> Visit<'ast> for WeakPrngChecker<'_, '_> {
    type BreakValue = ();

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        if is_randomness_expr(expr) {
            self.ctx.emit(&WEAK_PRNG, expr.span);
            ControlFlow::Continue(())
        } else {
            self.walk_expr(expr)
        }
    }
}

fn is_randomness_expr(expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Binary(lhs, BinOp { kind: BinOpKind::Rem, .. }, rhs) => {
            contains_predictable_source(lhs) || contains_predictable_source(rhs)
        }
        ExprKind::Call(callee, args) => {
            let callee = callee.peel_parens();
            (is_keccak256(callee) || is_abi_encode_packed(callee))
                && args.exprs().any(contains_predictable_source)
        }
        _ => false,
    }
}

fn contains_predictable_source(expr: &Expr<'_>) -> bool {
    let expr = expr.peel_parens();
    if is_predictable_source(expr) {
        return true;
    }

    match &expr.kind {
        ExprKind::Array(elems) => elems.iter().any(|elem| contains_predictable_source(elem)),
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            contains_predictable_source(lhs) || contains_predictable_source(rhs)
        }
        ExprKind::Call(callee, args) => {
            contains_predictable_source(callee) || args.exprs().any(contains_predictable_source)
        }
        ExprKind::CallOptions(callee, options) => {
            contains_predictable_source(callee)
                || options.iter().any(|option| contains_predictable_source(&option.value))
        }
        ExprKind::Delete(inner) | ExprKind::Member(inner, _) | ExprKind::Unary(_, inner) => {
            contains_predictable_source(inner)
        }
        ExprKind::Index(base, index) => {
            contains_predictable_source(base)
                || match index {
                    IndexKind::Index(Some(index)) => contains_predictable_source(index),
                    IndexKind::Range(start, end) => {
                        start.as_ref().is_some_and(|start| contains_predictable_source(start))
                            || end.as_ref().is_some_and(|end| contains_predictable_source(end))
                    }
                    _ => false,
                }
        }
        ExprKind::Payable(args) => args.exprs().any(contains_predictable_source),
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            contains_predictable_source(cond)
                || contains_predictable_source(then_expr)
                || contains_predictable_source(else_expr)
        }
        ExprKind::Tuple(elems) => elems.iter().any(|elem| {
            if let SpannedOption::Some(inner) = elem.as_ref() {
                contains_predictable_source(inner)
            } else {
                false
            }
        }),
        _ => false,
    }
}

fn is_predictable_source(expr: &Expr<'_>) -> bool {
    is_block_member(expr) || is_blockhash_call(expr)
}

fn is_block_member(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Member(base, member)
            if matches!(member.as_str(), "timestamp" | "prevrandao" | "difficulty")
            && is_ident(base.peel_parens(), "block")
    )
}

fn is_blockhash_call(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Call(callee, _) if is_ident(callee.peel_parens(), "blockhash")
    )
}

fn is_keccak256(expr: &Expr<'_>) -> bool {
    is_ident(expr, "keccak256")
}

fn is_abi_encode_packed(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Member(base, member)
            if member.as_str() == "encodePacked" && is_ident(base.peel_parens(), "abi")
    )
}

fn is_ident(expr: &Expr<'_>, name: &str) -> bool {
    matches!(&expr.kind, ExprKind::Ident(ident) if ident.as_str() == name)
}
