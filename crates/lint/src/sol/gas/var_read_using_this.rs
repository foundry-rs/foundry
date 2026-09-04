use super::VarReadUsingThis;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint, analysis::is_builtin},
};
use solar::{
    ast::{ContractKind, StateMutability},
    interface::{Symbol, data_structures::Never, diagnostics::Applicability, sym},
    sema::{
        Gcx,
        hir::{self, CallArgs, Expr, ExprId, ExprKind, Function, Stmt, StmtKind, Visit as _},
    },
};
use std::{collections::HashMap, ops::ControlFlow};

declare_forge_lint!(
    VAR_READ_USING_THIS,
    Severity::Gas,
    "var-read-using-this",
    "reading a state variable via `this` causes an unnecessary STATICCALL; access it directly"
);

impl<'gcx> LateLintPass<'gcx> for VarReadUsingThis {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract_id: hir::ContractId,
    ) {
        let contract = gcx.hir.contract(contract_id);
        // `this` only exists in (abstract) contracts: libraries have none, interfaces no bodies.
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract) {
            return;
        }

        // Externally callable functions reachable through `this.<name>(...)`, grouped by name so
        // overloads and inherited overrides can be resolved by arity.
        let mut callable = HashMap::<_, Vec<_>>::new();
        for fid in
            contract.linearized_bases.iter().flat_map(|&cid| gcx.hir.contract(cid).functions())
        {
            let func = gcx.hir.function(fid);
            if let Some(name) = func.name
                && func.is_part_of_external_interface()
            {
                callable.entry(name.name).or_default().push(func);
            }
        }

        let mut finder = ThisReadFinder { ctx, hir: &gcx.hir, callable, try_target: None };
        // State variable initializers run in the synthesized constructor.
        for var_id in contract.variables() {
            let _ = finder.visit_nested_var(var_id);
        }
        // Only bodies defined in this contract; inherited ones are walked with their own contract.
        for fid in contract.all_functions() {
            let _ = finder.visit_nested_function(fid);
        }
    }
}

struct ThisReadFinder<'a, 'gcx> {
    ctx: &'a LintContext<'a, 'a>,
    hir: &'gcx hir::Hir<'gcx>,
    callable: HashMap<Symbol, Vec<&'gcx Function<'gcx>>>,
    /// The expression tried by the enclosing `try` statement, which must stay an external call.
    try_target: Option<ExprId>,
}

impl<'gcx> hir::Visit<'gcx> for ThisReadFinder<'_, 'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Self::BreakValue> {
        if let StmtKind::Try(try_stmt) = &stmt.kind {
            self.try_target = Some(try_stmt.expr.id);
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        if self.try_target != Some(expr.id) {
            self.check_call(expr);
        }
        self.walk_expr(expr)
    }
}

impl ThisReadFinder<'_, '_> {
    /// Flags `this.<name>(args)` when `<name>` resolves to a `view`/`pure` function of the
    /// current contract.
    fn check_call(&self, expr: &Expr<'_>) {
        let ExprKind::Call(callee, args, opts) = &expr.kind else { return };
        let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return };
        if !is_builtin(base, sym::this) {
            return;
        }
        let Some(candidates) = self.callable.get(&member.name) else { return };
        // Solar's HIR `Member` is name-based, so overloads are resolved by arity. When same-arity
        // overloads mix mutability (`f(uint256) view` vs `f(address)`), bail to avoid flagging
        // the mutating one.
        let same_arity: Vec<_> =
            candidates.iter().filter(|f| f.parameters.len() == args.len()).collect();
        let Some(func) = same_arity.first() else { return };
        if !same_arity
            .iter()
            .all(|f| matches!(f.state_mutability, StateMutability::View | StateMutability::Pure))
        {
            return;
        }
        // With call options like `{gas: ...}` the external call is deliberate: flag the gas waste
        // without an auto-fix.
        let suggestion =
            if opts.is_some() { None } else { suggestion(self.ctx, func, member.name, args) };
        match suggestion {
            Some(suggestion) => {
                self.ctx.emit_with_suggestion(&VAR_READ_USING_THIS, expr.span, suggestion);
            }
            None => self.ctx.emit(&VAR_READ_USING_THIS, expr.span),
        }
    }
}

fn suggestion(
    ctx: &LintContext,
    func: &Function<'_>,
    name: Symbol,
    args: &CallArgs<'_>,
) -> Option<Suggestion> {
    if !func.is_getter() {
        // Ordinary `view`/`pure` functions may be `external`, requiring a refactor to call them.
        return Some(
            Suggestion::example(format!("call directly without `this.`: `{name}(...)`"))
                .with_desc("avoid the STATICCALL by invoking the function directly"),
        );
    }
    // Struct getters destructure their fields, so a direct read is not equivalent.
    if func.returns.len() != 1 {
        return Some(
            Suggestion::example(format!("read the state variable directly: `{name}`"))
                .with_desc("read the state variable directly instead of via `this.`"),
        );
    }
    if args.is_empty() {
        return Some(
            Suggestion::fix(name.to_string(), Applicability::MachineApplicable)
                .with_desc("consider reading the state variable directly"),
        );
    }
    // Mapping/array getter: `name[arg1][arg2]...`.
    let mut indexed = name.to_string();
    for arg in args.exprs() {
        indexed += &format!("[{}]", ctx.span_to_snippet(arg.span)?.trim());
    }
    Some(
        Suggestion::fix(indexed, Applicability::MaybeIncorrect)
            .with_desc("consider accessing storage directly"),
    )
}
