use super::UninitializedStateVariables;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{is_builtin, referenced_item, tuple_elems},
    },
};
use solar::{
    ast::ContractKind,
    interface::{data_structures::Never, sym},
    sema::{
        Gcx, Hir,
        hir::{
            CallArgs, CallArgsKind, Contract, ContractId, DataLocation, Expr, ExprKind, Function,
            ItemId, Res, Stmt, StmtKind, TypeKind, VariableId, Visit,
        },
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

declare_forge_lint!(
    UNINITIALIZED_STATE_VARIABLES,
    Severity::Med,
    "uninitialized-state",
    "state variable is read but never written"
);

impl<'gcx> LateLintPass<'gcx> for UninitializedStateVariables {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract_id: ContractId,
    ) {
        let contract = gcx.hir.contract(contract_id);
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
        let mut collector = Collector {
            hir: &gcx.hir,
            bases,
            read: HashSet::new(),
            written: HashSet::new(),
            aliases: HashMap::new(),
        };
        // Inline assembly can write storage directly; bail out conservatively.
        if bases.iter().any(|&cid| collector.visit_contract_items(gcx.hir.contract(cid)).is_break())
        {
            return;
        }

        for var_id in bases.iter().flat_map(|&cid| gcx.hir.contract(cid).variables()) {
            let var = gcx.hir.variable(var_id);
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

struct Collector<'gcx> {
    hir: &'gcx Hir<'gcx>,
    bases: &'gcx [ContractId],
    read: HashSet<VariableId>,
    written: HashSet<VariableId>,
    /// State variables each local `storage` pointer of the current function may reference.
    aliases: Aliases,
}

/// Maps local `storage` pointers to the state variables they may reference.
type Aliases = HashMap<VariableId, HashSet<VariableId>>;

impl<'gcx> Collector<'gcx> {
    fn visit_contract_items(&mut self, contract: &'gcx Contract<'gcx>) -> ControlFlow<()> {
        contract.all_functions().try_for_each(|fid| self.visit_nested_function(fid))?;
        contract.variables().try_for_each(|vid| self.visit_nested_var(vid))?;
        contract.bases_args.iter().try_for_each(|m| self.visit_modifier(m))
    }

    /// Marks the variable at the root of an lvalue (through index/slice/member access and tuple
    /// destructuring) as written; a write through a `storage` pointer writes its targets.
    fn mark_written(&mut self, expr: &Expr<'_>) {
        match &expr.peel_parens().kind {
            ExprKind::Ident([Res::Item(id), ..]) => {
                if let Some(id) = id.as_variable() {
                    self.written.insert(id);
                    if let Some(targets) = self.aliases.get(&id) {
                        self.written.extend(targets);
                    }
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
    fn mark_storage_args(&mut self, callee: &'gcx Expr<'gcx>, args: &'gcx CallArgs<'gcx>) {
        let funcs = self.callee_candidates(callee);
        let is_storage =
            |pid: &VariableId| self.hir.variable(*pid).data_location == Some(DataLocation::Storage);
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
                            self.hir.variable(*pid).name.is_some_and(|n| n.name == arg.name.name)
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
    fn callee_candidates(&self, callee: &'gcx Expr<'gcx>) -> Vec<&'gcx Function<'gcx>> {
        match &callee.kind {
            ExprKind::Ident(reses) => {
                reses.iter().filter_map(Res::as_function).map(|f| self.hir.function(f)).collect()
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
                    .flat_map(|&cid| self.hir.contract(cid).all_functions())
                    .map(|fid| self.hir.function(fid))
                    .filter(|f| f.name.is_some_and(|n| n.name == method.name))
                    .collect()
            }
            _ => Vec::new(),
        }
    }
}

impl<'gcx> Visit<'gcx> for Collector<'gcx> {
    type BreakValue = ();

    fn hir(&self) -> &'gcx Hir<'gcx> {
        self.hir
    }

    fn visit_function(&mut self, func: &'gcx Function<'gcx>) -> ControlFlow<()> {
        self.aliases = storage_aliases(self.hir, func);
        func.modifiers.iter().try_for_each(|m| self.visit_modifier(m))?;
        func.body.iter().flat_map(|body| body.stmts).try_for_each(|stmt| self.visit_stmt(stmt))
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<()> {
        match stmt.kind {
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => {
                ControlFlow::Break(())
            }
            _ => self.walk_stmt(stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<()> {
        match &expr.kind {
            ExprKind::Ident(reses) => self.read.extend(reses.iter().filter_map(Res::as_variable)),
            // Reassigning a bare storage pointer repoints it rather than writing its target.
            ExprKind::Assign(lhs, ..) if !is_storage_pointer(self.hir, lhs) => {
                self.mark_written(lhs)
            }
            ExprKind::Delete(lhs) => self.mark_written(lhs),
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

/// Collects, flow-insensitively, the state variables each local `storage` pointer declared in
/// `func` may reference: every assignment contributes to the pointer's target set, and pointers
/// assigned from other pointers are resolved transitively.
fn storage_aliases<'gcx>(hir: &'gcx Hir<'gcx>, func: &'gcx Function<'gcx>) -> Aliases {
    struct Edges<'gcx> {
        hir: &'gcx Hir<'gcx>,
        edges: HashMap<VariableId, HashSet<VariableId>>,
    }

    impl Edges<'_> {
        /// Records `lhs = rhs`, matching tuple destructuring element-wise.
        fn record(&mut self, lhs: &Expr<'_>, rhs: &Expr<'_>) {
            match (tuple_elems(lhs), tuple_elems(rhs)) {
                (Some(targets), Some(values)) => {
                    for (target, value) in targets.iter().zip(values) {
                        if let (Some(target), Some(value)) = (target, value) {
                            self.record(target, value);
                        }
                    }
                }
                _ => {
                    if let Some(var) = referenced_item(lhs).and_then(|id| id.as_variable()) {
                        self.record_var(var, rhs);
                    }
                }
            }
        }

        fn record_var(&mut self, var: VariableId, rhs: &Expr<'_>) {
            if is_local_storage_var(self.hir, var) {
                root_vars(rhs, self.edges.entry(var).or_default());
            }
        }
    }

    impl<'gcx> Visit<'gcx> for Edges<'gcx> {
        type BreakValue = Never;

        fn hir(&self) -> &'gcx Hir<'gcx> {
            self.hir
        }

        fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Never> {
            match &stmt.kind {
                StmtKind::DeclSingle(var) => {
                    if let Some(init) = self.hir.variable(*var).initializer {
                        self.record_var(*var, init);
                    }
                }
                StmtKind::DeclMulti(vars, init) => {
                    for (var, value) in vars.iter().zip(tuple_elems(init).unwrap_or_default()) {
                        if let (Some(var), Some(value)) = (var, value) {
                            self.record_var(*var, value);
                        }
                    }
                }
                _ => {}
            }
            self.walk_stmt(stmt)
        }

        fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Never> {
            if let ExprKind::Assign(lhs, None, rhs) = &expr.kind {
                self.record(lhs, rhs);
            }
            self.walk_expr(expr)
        }
    }

    let mut edges = Edges { hir, edges: HashMap::new() };
    let _ = edges.visit_function(func);
    let edges = edges.edges;
    let mut aliases = Aliases::new();
    for &pointer in edges.keys() {
        let (mut targets, mut seen, mut stack) = (HashSet::new(), HashSet::new(), vec![pointer]);
        while let Some(var) = stack.pop() {
            if !seen.insert(var) {
                continue;
            }
            if hir.variable(var).kind.is_state() {
                targets.insert(var);
            } else if let Some(roots) = edges.get(&var) {
                stack.extend(roots);
            }
        }
        if !targets.is_empty() {
            aliases.insert(pointer, targets);
        }
    }
    aliases
}

/// The variables an expression is rooted in, through indexing, member access and ternaries.
fn root_vars(expr: &Expr<'_>, roots: &mut HashSet<VariableId>) {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(id), ..]) => roots.extend(id.as_variable()),
        ExprKind::Index(base, _) | ExprKind::Slice(base, ..) | ExprKind::Member(base, _) => {
            root_vars(base, roots)
        }
        ExprKind::Ternary(_, then, otherwise) => {
            root_vars(then, roots);
            root_vars(otherwise, roots);
        }
        _ => {}
    }
}

/// A bare local `storage` pointer.
fn is_storage_pointer(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    referenced_item(expr)
        .and_then(|id| id.as_variable())
        .is_some_and(|var| is_local_storage_var(hir, var))
}

fn is_local_storage_var(hir: &Hir<'_>, var: VariableId) -> bool {
    let var = hir.variable(var);
    !var.kind.is_state() && var.data_location == Some(DataLocation::Storage)
}
