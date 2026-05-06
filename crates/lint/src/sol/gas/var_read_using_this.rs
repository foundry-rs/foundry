use super::VarReadUsingThis;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, StateMutability},
    interface::{Symbol, diagnostics::Applicability, sym},
    sema::hir::{self, ExprKind, Res, StmtKind},
};
use std::collections::HashMap;

declare_forge_lint!(
    VAR_READ_USING_THIS,
    Severity::Gas,
    "var-read-using-this",
    "reading a state variable via `this` causes an unnecessary STATICCALL; access it directly"
);

impl<'hir> LateLintPass<'hir> for VarReadUsingThis {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        contract_id: hir::ContractId,
    ) {
        let contract = hir.contract(contract_id);

        // `this` only exists inside contracts (concrete or abstract).
        // Libraries have no `this`; interfaces have no function bodies.
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract) {
            return;
        }

        // Collect every externally-callable function reachable on `this.X(...)`,
        // grouped by name. Includes overloads (same name, different parameter types)
        // as well as inherited overrides; `match_this_call` resolves them by arity
        // and conservatively skips mixed-mutability overload sets.
        let mut callable: HashMap<Symbol, Vec<&'hir hir::Function<'hir>>> = HashMap::new();
        for &cid in contract.linearized_bases {
            for fid in hir.contract(cid).functions() {
                let func = hir.function(fid);
                let Some(name) = func.name else { continue };
                if !func.is_part_of_external_interface() {
                    continue;
                }
                callable.entry(name.name).or_default().push(func);
            }
        }

        if callable.is_empty() {
            return;
        }

        // Walk state variable initializers (these run in the synthesized constructor).
        for var_id in contract.variables() {
            let var = hir.variable(var_id);
            if let Some(init) = var.initializer {
                walk_expr(ctx, init, &callable);
            }
        }

        // Walk only function/modifier bodies *defined in this contract*; inherited
        // bodies are walked when their defining contract is visited.
        for fid in contract.all_functions() {
            let func = hir.function(fid);

            // Modifier invocations on the function (also covers base-class constructor
            // calls, which solar stores in the same field for constructors).
            for modifier in func.modifiers {
                for arg in modifier.args.exprs() {
                    walk_expr(ctx, arg, &callable);
                }
            }

            if let Some(body) = func.body {
                walk_block(ctx, hir, body, &callable);
            }
        }
    }
}

fn walk_block<'hir>(
    ctx: &LintContext,
    hir: &'hir hir::Hir<'hir>,
    block: hir::Block<'hir>,
    callable: &HashMap<Symbol, Vec<&'hir hir::Function<'hir>>>,
) {
    for stmt in block.stmts {
        walk_stmt(ctx, hir, stmt, callable);
    }
}

fn walk_stmt<'hir>(
    ctx: &LintContext,
    hir: &'hir hir::Hir<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
    callable: &HashMap<Symbol, Vec<&'hir hir::Function<'hir>>>,
) {
    match &stmt.kind {
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            walk_block(ctx, hir, *block, callable);
        }
        StmtKind::If(cond, then_stmt, else_stmt) => {
            walk_expr(ctx, cond, callable);
            walk_stmt(ctx, hir, then_stmt, callable);
            if let Some(else_stmt) = else_stmt {
                walk_stmt(ctx, hir, else_stmt, callable);
            }
        }
        StmtKind::Try(stmt_try) => {
            // `try` requires an external call by Solidity's rules, so the outer
            // `this.X(...)` cannot be replaced with a direct read. Skip flagging the
            // top-level call but still recurse into its arguments so nested
            // `this.X(...)` reads are caught.
            let try_expr = &stmt_try.expr;
            if let ExprKind::Call(callee, args, named_args) = &try_expr.kind {
                walk_expr(ctx, callee, callable);
                for e in args.exprs() {
                    walk_expr(ctx, e, callable);
                }
                if let Some(nargs) = named_args {
                    for arg in *nargs {
                        walk_expr(ctx, &arg.value, callable);
                    }
                }
            } else {
                walk_expr(ctx, try_expr, callable);
            }
            for clause in stmt_try.clauses {
                walk_block(ctx, hir, clause.block, callable);
            }
        }
        StmtKind::DeclSingle(var_id) => {
            if let Some(init) = hir.variable(*var_id).initializer {
                walk_expr(ctx, init, callable);
            }
        }
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr)
        | StmtKind::Return(Some(expr))
        | StmtKind::Expr(expr) => walk_expr(ctx, expr, callable),
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::Err(_) => {}
    }
}

fn walk_expr<'hir>(
    ctx: &LintContext,
    expr: &'hir hir::Expr<'hir>,
    callable: &HashMap<Symbol, Vec<&'hir hir::Function<'hir>>>,
) {
    // Check the outer expression first so we report the entire `this.X(args)` call,
    // then keep walking children to also find nested matches like `this.foo(this.bar)`.
    if let Some(matched) = match_this_call(expr, callable) {
        emit(ctx, expr, &matched);
    }

    match &expr.kind {
        ExprKind::Array(exprs) => {
            for e in *exprs {
                walk_expr(ctx, e, callable);
            }
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            walk_expr(ctx, lhs, callable);
            walk_expr(ctx, rhs, callable);
        }
        ExprKind::Call(callee, args, named_args) => {
            walk_expr(ctx, callee, callable);
            for e in args.exprs() {
                walk_expr(ctx, e, callable);
            }
            if let Some(nargs) = named_args {
                for arg in *nargs {
                    walk_expr(ctx, &arg.value, callable);
                }
            }
        }
        ExprKind::Delete(inner) | ExprKind::Payable(inner) | ExprKind::Unary(_, inner) => {
            walk_expr(ctx, inner, callable);
        }
        ExprKind::Index(base, index) => {
            walk_expr(ctx, base, callable);
            if let Some(i) = index {
                walk_expr(ctx, i, callable);
            }
        }
        ExprKind::Slice(base, start, end) => {
            walk_expr(ctx, base, callable);
            if let Some(s) = start {
                walk_expr(ctx, s, callable);
            }
            if let Some(e) = end {
                walk_expr(ctx, e, callable);
            }
        }
        ExprKind::Member(base, _) => walk_expr(ctx, base, callable),
        ExprKind::Ternary(cond, then_e, else_e) => {
            walk_expr(ctx, cond, callable);
            walk_expr(ctx, then_e, callable);
            walk_expr(ctx, else_e, callable);
        }
        ExprKind::Tuple(exprs) => {
            for e in exprs.iter().flatten() {
                walk_expr(ctx, e, callable);
            }
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => {}
    }
}

#[derive(Clone, Copy)]
struct MatchedCall<'hir> {
    func: &'hir hir::Function<'hir>,
    args: hir::CallArgs<'hir>,
    /// Whether the call expression carries options like `{gas: ...}`.
    has_call_options: bool,
    /// The name on the right-hand side of the `this.<name>` member access.
    member_name: Symbol,
}

/// Returns `Some(...)` if `expr` is a `this.<name>(args)` call where `<name>` resolves
/// (via overload resolution by arity) to a `view`/`pure` external-interface function on
/// the current contract.
fn match_this_call<'hir>(
    expr: &'hir hir::Expr<'hir>,
    callable: &HashMap<Symbol, Vec<&'hir hir::Function<'hir>>>,
) -> Option<MatchedCall<'hir>> {
    let ExprKind::Call(callee, args, named_args) = &expr.kind else { return None };

    // Allow `(this.foo)(args)` by peeling parens around the callee.
    let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return None };

    // Allow `(this).foo(args)` by peeling parens around the base.
    let ExprKind::Ident(resolutions) = &base.peel_parens().kind else { return None };
    let is_this = resolutions.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == sym::this));
    if !is_this {
        return None;
    }

    let candidates = callable.get(&member.name)?;
    let arity = args.len();

    // Solar's HIR `Member(base, ident)` is name-based and does not carry the resolved
    // function id, so we approximate overload resolution by arity. To avoid false
    // positives when same-arity overloads mix mutability (e.g. `f(uint256) view` vs
    // `f(address)` mutating), require ALL same-arity overloads to be `view`/`pure`.
    let mut found: Option<&'hir hir::Function<'hir>> = None;
    for f in candidates.iter().copied() {
        if f.parameters.len() != arity {
            continue;
        }
        if !matches!(f.state_mutability, StateMutability::View | StateMutability::Pure) {
            return None;
        }
        if found.is_none() {
            found = Some(f);
        }
    }
    let func = found?;

    Some(MatchedCall {
        func,
        args: *args,
        has_call_options: named_args.is_some(),
        member_name: member.name,
    })
}

fn emit(ctx: &LintContext, expr: &hir::Expr<'_>, matched: &MatchedCall<'_>) {
    // When the call carries options like `{gas: ...}`, the developer is intentionally
    // reaching for the external-call machinery; flag the gas waste but do not auto-fix.
    if matched.has_call_options {
        ctx.emit(&VAR_READ_USING_THIS, expr.span);
        return;
    }

    if let Some(suggestion) = build_suggestion(ctx, matched) {
        ctx.emit_with_suggestion(&VAR_READ_USING_THIS, expr.span, suggestion);
    } else {
        ctx.emit(&VAR_READ_USING_THIS, expr.span);
    }
}

fn build_suggestion(ctx: &LintContext, matched: &MatchedCall<'_>) -> Option<Suggestion> {
    let name = matched.member_name.as_str();

    if matched.func.is_getter() {
        // Skip suggestions for struct-typed getters (multi-return) — the synthesized
        // getter destructures struct fields, so a direct rewrite is not equivalent.
        if matched.func.returns.len() != 1 {
            return Some(
                Suggestion::example(format!("read the state variable directly: `{name}`"))
                    .with_desc("read the state variable directly instead of via `this.`"),
            );
        }

        if matched.args.is_empty() {
            // Simple state variable getter: `this.foo()` -> `foo`
            return Some(
                Suggestion::fix(name.to_string(), Applicability::MachineApplicable)
                    .with_desc("consider reading the state variable directly"),
            );
        }

        // Mapping/array getter: rebuild as `name[arg1][arg2]...` from arg snippets.
        let mut indexed = String::from(name);
        for arg in matched.args.exprs() {
            let snippet = ctx.span_to_snippet(arg.span)?;
            indexed.push('[');
            indexed.push_str(snippet.trim());
            indexed.push(']');
        }
        return Some(
            Suggestion::fix(indexed, Applicability::MaybeIncorrect)
                .with_desc("consider accessing storage directly"),
        );
    }

    // Ordinary `view` / `pure` function: cannot auto-fix (visibility may be `external`,
    // requiring a refactor to extract an internal helper). Show a generic example.
    Some(
        Suggestion::example(format!("call directly without `this.`: `{name}(...)`"))
            .with_desc("avoid the STATICCALL by invoking the function directly"),
    )
}
