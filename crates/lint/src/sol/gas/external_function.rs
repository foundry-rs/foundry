use super::ExternalFunction;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, DataLocation, UnOpKind, Visibility},
    interface::{Symbol, data_structures::Never},
    sema::hir::{self, ContractId, ExprKind, FunctionId, ItemId, Res, VariableId, Visit as _},
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    rc::Rc,
};

declare_forge_lint!(
    EXTERNAL_FUNCTION,
    Severity::Gas,
    "external-function",
    "public function can be declared external"
);

#[derive(Default)]
struct ProjectIndex {
    /// `FunctionId`s referenced via an `Ident` resolution anywhere in the project. Covers
    /// direct internal calls (`foo()`) and function-pointer references (`fn = foo;`).
    referenced: HashSet<FunctionId>,
    /// `super.<name>` callsites keyed by the contract that contains them, so name matches
    /// can be scoped to the caller's inheritance chain.
    super_called: HashMap<Symbol, HashSet<ContractId>>,
}

thread_local! {
    /// Project index keyed by the [`hir::Hir`] address. The HIR lives inside the
    /// [`solar::sema::Compiler`] for the whole lint run, so its address is stable and the
    /// same index can be reused across every contract instead of rebuilt per source.
    static PROJECT_INDEX: RefCell<Option<(usize, Rc<ProjectIndex>)>> = const { RefCell::new(None) };
}

fn project_index_for<'hir>(hir: &'hir hir::Hir<'hir>) -> Rc<ProjectIndex> {
    let key = std::ptr::from_ref::<hir::Hir<'_>>(hir) as usize;
    PROJECT_INDEX.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some((cached_key, cached)) = slot.as_ref()
            && *cached_key == key
        {
            return cached.clone();
        }
        let fresh = Rc::new(build_project_index(hir));
        *slot = Some((key, fresh.clone()));
        fresh
    })
}

impl<'hir> LateLintPass<'hir> for ExternalFunction {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        contract_id: ContractId,
    ) {
        if !ctx.is_lint_enabled(EXTERNAL_FUNCTION.id) {
            return;
        }

        let contract = hir.contract(contract_id);

        // Libraries have different `external` semantics (delegatecall vs inlining); interfaces
        // have no bodies.
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract) {
            return;
        }
        if contract.linearization_failed() {
            return;
        }

        let index = project_index_for(hir);

        for fid in contract.functions() {
            let func = hir.function(fid);

            // `is_ordinary()` excludes constructor / fallback / receive / modifier.
            if func.visibility != Visibility::Public || !func.is_ordinary() {
                continue;
            }
            // Solidity only allows widening visibility on override (`external` -> `public`),
            // never tightening. Flag the base chain instead.
            if func.override_ {
                continue;
            }
            // Abstract declarations must stay `public` so derived contracts can override them.
            let Some(body) = func.body else { continue };

            // Only flag when at least one parameter is a reference type currently in `memory`;
            // value-only signatures yield negligible savings.
            let has_memory_reference_param = func.parameters.iter().any(|&pid| {
                let p = hir.variable(pid);
                p.ty.kind.is_reference_type() && p.data_location == Some(DataLocation::Memory)
            });
            if !has_memory_reference_param {
                continue;
            }

            // External params live in (immutable) calldata, so a body that writes to a param
            // cannot be moved to `external` without further refactoring.
            if body_writes_to_params(hir, &body, func.parameters) {
                continue;
            }

            let Some(name) = func.name else { continue };

            // Any same-name/arity reference in this contract or a derivative — or a
            // `super.<name>` from a derivative — counts as an internal call.
            if super_called_from_derivative(hir, contract_id, &name.name, &index.super_called) {
                continue;
            }
            if any_override_referenced(hir, contract_id, func, &index.referenced) {
                continue;
            }

            ctx.emit(&EXTERNAL_FUNCTION, name.span);
        }
    }
}

fn build_project_index<'hir>(hir: &'hir hir::Hir<'hir>) -> ProjectIndex {
    let mut builder = IndexBuilder { hir, idx: ProjectIndex::default(), current_contract: None };
    for func in hir.functions() {
        builder.current_contract = func.contract;
        let _ = builder.visit_function(func);
    }
    // State-variable initializers run in the synthesized constructor; their references count
    // as "called". Function-local var initializers are already covered by `visit_function`.
    for var in hir.variables() {
        if var.is_state_variable() {
            builder.current_contract = var.contract;
            let _ = builder.visit_var(var);
        }
    }
    builder.idx
}

/// HIR visitor that records every `FunctionId` referenced via an `Ident` and every name on
/// the right-hand side of a `super.<name>` access. Stmt/expr recursion is handled by
/// `hir::Visit`'s default walks so adding a new HIR variant only updates this in one place.
struct IndexBuilder<'hir> {
    hir: &'hir hir::Hir<'hir>,
    idx: ProjectIndex,
    /// Contract being walked; used to attribute `super.<name>` calls to the caller.
    current_contract: Option<ContractId>,
}

impl<'hir> hir::Visit<'hir> for IndexBuilder<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            ExprKind::Ident(reses) => {
                for res in *reses {
                    if let Res::Item(ItemId::Function(fid)) = res {
                        self.idx.referenced.insert(*fid);
                    }
                }
            }
            ExprKind::Member(base, member) => {
                if let Some(cid) = self.current_contract
                    && let ExprKind::Ident(reses) = &base.peel_parens().kind
                    && reses.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == solar::interface::sym::super_))
                {
                    self.idx.super_called.entry(member.name).or_default().insert(cid);
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

/// Returns `true` if any strict descendant of `base_contract_id` contains a `super.<name>`
/// call (the only callsites that can resolve into `base_contract_id`).
fn super_called_from_derivative(
    hir: &hir::Hir<'_>,
    base_contract_id: ContractId,
    name: &Symbol,
    super_called: &HashMap<Symbol, HashSet<ContractId>>,
) -> bool {
    let Some(callers) = super_called.get(name) else { return false };
    callers.iter().any(|&caller_cid| {
        caller_cid != base_contract_id
            && hir.contract(caller_cid).linearized_bases.contains(&base_contract_id)
    })
}

/// Returns `true` if any function in `contract_id` or a derivative shares `base`'s name and
/// arity and is present in `referenced` (a call to an override conceptually targets the
/// base's slot). Match is name + arity only — solar's HIR `TypeKind` has no structural
/// equality — so same-arity overloads are conflated, yielding only false negatives.
fn any_override_referenced(
    hir: &hir::Hir<'_>,
    contract_id: ContractId,
    base: &hir::Function<'_>,
    referenced: &HashSet<FunctionId>,
) -> bool {
    let Some(base_name) = base.name else { return false };
    let arity = base.parameters.len();

    for (other_cid, other_contract) in hir.contracts_enumerated() {
        if other_cid != contract_id && !other_contract.linearized_bases.contains(&contract_id) {
            continue;
        }
        for fid in other_contract.functions() {
            if referenced.contains(&fid) {
                let other = hir.function(fid);
                if let Some(other_name) = other.name
                    && other_name.name == base_name.name
                    && other.parameters.len() == arity
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Returns `true` if `body` writes to any of `params`, including via member/index access
/// (`p.x = 1`, `p[0] = 1`), `delete p`, or `++`/`--`. Stmt/expr recursion is handled by
/// `hir::Visit`'s default walks.
fn body_writes_to_params<'hir>(
    hir: &'hir hir::Hir<'hir>,
    body: &hir::Block<'hir>,
    params: &[VariableId],
) -> bool {
    let mut finder = ParamWriteFinder { hir, params };
    body.stmts.iter().any(|stmt| finder.visit_stmt(stmt).is_break())
}

struct ParamWriteFinder<'a, 'hir> {
    hir: &'hir hir::Hir<'hir>,
    params: &'a [VariableId],
}

impl<'hir> hir::Visit<'hir> for ParamWriteFinder<'_, 'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        let writes_to_param = match &expr.kind {
            ExprKind::Assign(lhs, _, _) => expr_root_is_param(lhs, self.params),
            ExprKind::Delete(inner) => expr_root_is_param(inner, self.params),
            ExprKind::Unary(op, inner)
                if matches!(
                    op.kind,
                    UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                ) =>
            {
                expr_root_is_param(inner, self.params)
            }
            _ => false,
        };
        if writes_to_param {
            return ControlFlow::Break(());
        }
        self.walk_expr(expr)
    }
}

/// Returns `true` if the root of `expr` — after peeling parens / members / indexes / slices —
/// is an `Ident` resolving to one of `params`.
fn expr_root_is_param(expr: &hir::Expr<'_>, params: &[VariableId]) -> bool {
    let mut cur = expr.peel_parens();
    loop {
        match &cur.kind {
            ExprKind::Member(base, _) | ExprKind::Payable(base) => cur = base.peel_parens(),
            ExprKind::Index(base, _) | ExprKind::Slice(base, _, _) => cur = base.peel_parens(),
            ExprKind::Ident(reses) => {
                return reses.iter().any(
                    |r| matches!(r, Res::Item(ItemId::Variable(vid)) if params.contains(vid)),
                );
            }
            _ => return false,
        }
    }
}
