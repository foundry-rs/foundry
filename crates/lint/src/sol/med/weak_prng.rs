use super::WeakPrng;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOp, BinOpKind, Expr, ExprKind, IndexKind, LitKind, SourceUnit, visit::Visit},
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
    fn check_full_source_unit(&mut self, ctx: &LintContext<'ast, '_>, ast: &'ast SourceUnit<'ast>) {
        if ctx.is_lint_enabled(WEAK_PRNG.id) {
            let mut checker = WeakPrngChecker { ctx };
            let _ = checker.visit_source_unit(ast);
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
            is_randomness_modulo(lhs, rhs)
        }
        ExprKind::Call(callee, args) => {
            let callee = callee.peel_parens();
            is_keccak256(callee) && args.exprs().any(contains_predictable_source)
        }
        _ => false,
    }
}

fn is_randomness_modulo(lhs: &Expr<'_>, rhs: &Expr<'_>) -> bool {
    if is_timestamp_time_bucket(lhs, rhs) {
        return false;
    }
    contains_predictable_source(lhs) || contains_predictable_source(rhs)
}

fn is_timestamp_time_bucket(lhs: &Expr<'_>, rhs: &Expr<'_>) -> bool {
    is_block_timestamp(lhs.peel_parens()) && is_time_bucket_modulus(rhs)
}

fn is_timestamp_time_bucket_expr(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Binary(lhs, BinOp { kind: BinOpKind::Rem, .. }, rhs)
            if is_timestamp_time_bucket(lhs, rhs)
    )
}

fn is_time_bucket_modulus(expr: &Expr<'_>) -> bool {
    const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

    const_eval_u64(expr)
        .is_some_and(|value| value >= SECONDS_PER_DAY && value % SECONDS_PER_DAY == 0)
}

fn const_eval_u64(expr: &Expr<'_>) -> Option<u64> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit, sub) => {
            if let LitKind::Number(value) = &lit.kind {
                let base = u64::try_from(value).ok()?;
                let multiplier = sub.map(|s| s.value()).unwrap_or(1);
                base.checked_mul(multiplier)
            } else {
                None
            }
        }
        ExprKind::Binary(lhs, BinOp { kind, .. }, rhs) => {
            let lhs = const_eval_u64(lhs)?;
            let rhs = const_eval_u64(rhs)?;
            match kind {
                BinOpKind::Add => lhs.checked_add(rhs),
                BinOpKind::Sub => lhs.checked_sub(rhs),
                BinOpKind::Mul => lhs.checked_mul(rhs),
                BinOpKind::Div => lhs.checked_div(rhs),
                _ => None,
            }
        }
        _ => None,
    }
}

fn contains_predictable_source(expr: &Expr<'_>) -> bool {
    let expr = expr.peel_parens();
    if is_timestamp_time_bucket_expr(expr) {
        return false;
    }
    if is_predictable_source(expr) {
        return true;
    }

    match &expr.kind {
        ExprKind::Array(elems) => elems.iter().any(|elem| contains_predictable_source(elem)),
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            contains_predictable_source(lhs) || contains_predictable_source(rhs)
        }
        ExprKind::Call(callee, args) if is_abi_encode(callee.peel_parens()) => {
            args.exprs().any(contains_predictable_source)
        }
        ExprKind::Call(callee, args) => {
            contains_predictable_source(callee) || args.exprs().any(contains_predictable_source)
        }
        ExprKind::CallOptions(callee, options) => {
            contains_predictable_source(callee)
                || options.iter().any(|option| contains_predictable_source(option.value))
        }
        ExprKind::Member(inner, _) | ExprKind::Unary(_, inner) => {
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
    is_block_member_with(expr, |member| {
        matches!(member, "timestamp" | "number" | "coinbase" | "prevrandao" | "difficulty")
    })
}

fn is_block_timestamp(expr: &Expr<'_>) -> bool {
    is_block_member_with(expr, |member| member == "timestamp")
}

fn is_block_member_with(expr: &Expr<'_>, predicate: impl FnOnce(&str) -> bool) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Member(base, member)
            if predicate(member.as_str())
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

fn is_abi_encode(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Member(base, member)
            if matches!(
                member.as_str(),
                "encode"
                    | "encodePacked"
                    | "encodeWithSelector"
                    | "encodeWithSignature"
                    | "encodeCall"
            ) && is_ident(base.peel_parens(), "abi")
    )
}

fn is_ident(expr: &Expr<'_>, name: &str) -> bool {
    matches!(&expr.kind, ExprKind::Ident(ident) if ident.as_str() == name)
}
