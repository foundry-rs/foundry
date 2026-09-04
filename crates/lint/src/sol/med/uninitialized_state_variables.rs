use super::UninitializedStateVariables;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::is_builtin},
};
use solar::{
    ast::ContractKind,
    interface::sym,
    sema::{
        Gcx, Hir,
        hir::{
            CallArgs, CallArgsKind, Contract, ContractId, DataLocation, Expr, ExprKind, Function,
            ItemId, Res, Stmt, StmtKind, TypeKind, VariableId, Visit,
        },
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    UNINITIALIZED_STATE_VARIABLES,
    Severity::Med,
    "uninitialized-state",
    "state variable is read but never written"
);

impl<'hir> LateLintPass<'hir> for UninitializedStateVariables {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        contract_id: ContractId,
    ) {
        let contract = hir.contract(contract_id);
        // Abstract contracts and interfaces are not deployed; a failed C3 linearization leaves
        // `linearized_bases` incomplete, so skip rather than produce unsound results.
        if matches!(contract.kind, ContractKind::Interface | ContractKind::AbstractContract)
            || contract.linearization_failed()
        {
            return;
        }

        // Every read and write in the whole inheritance chain (`linearized_bases[0]` is the
        // contract itself) determines whether a variable is ever written.
        let bases = contract.linearized_bases;
        let mut collector = Collector { hir, bases, read: HashSet::new(), written: HashSet::new() };
        // Inline assembly can write storage directly; bail out conservatively.
        if bases.iter().any(|&cid| collector.visit_contract_items(hir.contract(cid)).is_break()) {
            return;
        }

        for var_id in bases.iter().flat_map(|&cid| hir.contract(cid).variables()) {
            let var = hir.variable(var_id);
            if !var.is_constant()
                && !var.is_immutable()
                && !matches!(var.ty.kind, TypeKind::Mapping(_))
                && var.initializer.is_none()
                && collector.read.contains(&var_id)
                && !collector.written.contains(&var_id)
            {
                ctx.emit(&UNINITIALIZED_STATE_VARIABLES, var.span);
            }
        }
    }
}

struct Collector<'hir> {
    hir: &'hir Hir<'hir>,
    bases: &'hir [ContractId],
    read: HashSet<VariableId>,
    written: HashSet<VariableId>,
}

impl<'hir> Collector<'hir> {
    fn visit_contract_items(&mut self, contract: &'hir Contract<'hir>) -> ControlFlow<()> {
        contract.all_functions().try_for_each(|fid| self.visit_nested_function(fid))?;
        contract.variables().try_for_each(|vid| self.visit_nested_var(vid))?;
        contract.bases_args.iter().try_for_each(|m| self.visit_modifier(m))
    }

    /// Marks the variable at the root of an lvalue (through index/slice/member access and tuple
    /// destructuring) as written.
    fn mark_written(&mut self, expr: &Expr<'_>) {
        match &expr.peel_parens().kind {
            ExprKind::Ident([Res::Item(id), ..]) => {
                if let Some(id) = id.as_variable() {
                    self.written.insert(id);
                }
            }
            ExprKind::Tuple(exprs) => exprs.iter().flatten().for_each(|e| self.mark_written(e)),
            ExprKind::Index(base, _) | ExprKind::Slice(base, ..) | ExprKind::Member(base, _) => {
                self.mark_written(base)
            }
            _ => {}
        }
    }

    /// Internal functions that take a `storage` parameter mutate the corresponding argument in
    /// place. Overloads are not resolved, so an argument counts as written when *any* candidate
    /// of the callee's name has a `storage` parameter in that position (or of that name).
    fn mark_storage_args(&mut self, callee: &'hir Expr<'hir>, args: &'hir CallArgs<'hir>) {
        let hir = self.hir;
        let funcs = self.callee_candidates(callee);
        let is_storage =
            |pid: &VariableId| hir.variable(*pid).data_location == Some(DataLocation::Storage);
        match args.kind {
            CallArgsKind::Unnamed(exprs) => {
                for (i, arg) in exprs.iter().enumerate() {
                    if funcs.iter().any(|f| f.parameters.get(i).is_some_and(is_storage)) {
                        self.mark_written(arg);
                    }
                }
            }
            CallArgsKind::Named(named) => {
                for arg in named {
                    if funcs.iter().any(|f| {
                        f.parameters.iter().any(|pid| {
                            hir.variable(*pid).name.is_some_and(|n| n.name == arg.name.name)
                                && is_storage(pid)
                        })
                    }) {
                        self.mark_written(&arg.value);
                    }
                }
            }
        }
    }

    /// Functions a call may dispatch to: `f(..)`, `Contract.f(..)`, or `super.f(..)`, which
    /// resolves through the parent MRO entries only (never the current contract).
    fn callee_candidates(&self, callee: &'hir Expr<'hir>) -> Vec<&'hir Function<'hir>> {
        let hir = self.hir;
        match &callee.kind {
            ExprKind::Ident(reses) => {
                reses.iter().filter_map(Res::as_function).map(|f| hir.function(f)).collect()
            }
            ExprKind::Member(base, method) => {
                let contracts = if is_builtin(base, sym::super_) {
                    self.bases.get(1..).unwrap_or_default().to_vec()
                } else {
                    match &base.peel_parens().kind {
                        ExprKind::Ident(reses) => reses
                            .iter()
                            .filter_map(|r| match r {
                                Res::Item(ItemId::Contract(cid)) => Some(*cid),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    }
                };
                contracts
                    .iter()
                    .flat_map(|&cid| hir.contract(cid).all_functions())
                    .map(|fid| hir.function(fid))
                    .filter(|f| f.name.is_some_and(|n| n.name == method.name))
                    .collect()
            }
            _ => Vec::new(),
        }
    }
}

impl<'hir> Visit<'hir> for Collector<'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<()> {
        match stmt.kind {
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => {
                ControlFlow::Break(())
            }
            _ => self.walk_stmt(stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<()> {
        match &expr.kind {
            ExprKind::Ident(reses) => self.read.extend(reses.iter().filter_map(Res::as_variable)),
            ExprKind::Assign(lhs, ..) | ExprKind::Delete(lhs) => self.mark_written(lhs),
            ExprKind::Unary(op, lhs) if op.kind.has_side_effects() => self.mark_written(lhs),
            ExprKind::Call(callee, args, _) => {
                // The receiver of a member call covers `push`/`pop` and `using for` library
                // dispatch with a `T storage self` parameter.
                if let ExprKind::Member(base, _) = &callee.kind {
                    self.mark_written(base);
                }
                self.mark_storage_args(callee, args);
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}
