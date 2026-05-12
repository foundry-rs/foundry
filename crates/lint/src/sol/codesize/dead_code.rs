use super::DeadCode;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, FunctionKind, Visibility},
    interface::sym,
    sema::hir::{self, Visit as _},
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    DEAD_CODE,
    Severity::CodeSize,
    "dead-code",
    "internal or private function is never used"
);

impl<'hir> LateLintPass<'hir> for DeadCode {
    fn check_nested_source(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        source_id: hir::SourceId,
    ) {
        let reachable = Reachability::compute(hir);
        let overridden = collect_overridden_functions(hir);

        for function_id in hir.function_ids() {
            let function = hir.function(function_id);
            if function.source != source_id || !is_dead_code_candidate(hir, function, function_id) {
                continue;
            }

            if function.virtual_ && overridden.contains(&function_id) {
                continue;
            }

            if !reachable.contains(&function_id) {
                ctx.emit(&DEAD_CODE, function.span);
            }
        }
    }
}

fn is_dead_code_candidate(
    hir: &hir::Hir<'_>,
    function: &hir::Function<'_>,
    function_id: hir::FunctionId,
) -> bool {
    if function.kind != FunctionKind::Function
        || function.visibility >= Visibility::Public
        || function.body.is_none()
        || function.is_getter()
    {
        return false;
    }

    if let Some(contract_id) = function.contract {
        let contract = hir.contract(contract_id);
        if contract.kind == ContractKind::Library {
            return false;
        }

        // Constructors/fallback/receive functions are stored separately by the contract, but keep
        // this defensive check close to the candidate filter.
        if contract.ctor == Some(function_id)
            || contract.fallback == Some(function_id)
            || contract.receive == Some(function_id)
        {
            return false;
        }
    }

    true
}

fn is_entry_point(function: &hir::Function<'_>) -> bool {
    function.body.is_some()
        && (function.visibility >= Visibility::Public
            || matches!(
                function.kind,
                FunctionKind::Constructor | FunctionKind::Fallback | FunctionKind::Receive
            ))
}

fn collect_overridden_functions(hir: &hir::Hir<'_>) -> HashSet<hir::FunctionId> {
    let mut overridden = HashSet::new();

    for function_id in hir.function_ids() {
        let function = hir.function(function_id);
        if !function.override_ {
            continue;
        }

        let Some(contract_id) = function.contract else { continue };
        let Some(name) = function.name else { continue };
        let arity = function.parameters.len();
        let contract = hir.contract(contract_id);

        let bases = if function.overrides.is_empty() {
            contract.linearized_bases.get(1..).unwrap_or_default()
        } else {
            function.overrides
        };

        for &base_id in bases {
            for base_function_id in matching_functions(hir, base_id, name.name, arity) {
                if hir.function(base_function_id).virtual_ {
                    overridden.insert(base_function_id);
                }
            }
        }
    }

    overridden
}

fn matching_functions(
    hir: &hir::Hir<'_>,
    contract_id: hir::ContractId,
    name: solar::interface::Symbol,
    arity: usize,
) -> Vec<hir::FunctionId> {
    hir.contract(contract_id)
        .all_functions()
        .filter(|&function_id| {
            let function = hir.function(function_id);
            function.name.is_some_and(|ident| ident.name == name)
                && function.parameters.len() == arity
        })
        .collect()
}

struct Reachability<'hir> {
    hir: &'hir hir::Hir<'hir>,
    reachable: HashSet<hir::FunctionId>,
    current_contract: Option<hir::ContractId>,
}

impl<'hir> Reachability<'hir> {
    fn compute(hir: &'hir hir::Hir<'hir>) -> HashSet<hir::FunctionId> {
        let mut this = Self { hir, reachable: HashSet::new(), current_contract: None };

        for function_id in hir.function_ids() {
            if is_entry_point(hir.function(function_id)) {
                this.mark_function(function_id);
            }
        }

        for variable_id in hir.variable_ids() {
            let variable = hir.variable(variable_id);
            if variable.function.is_none() && variable.initializer.is_some() {
                let _ = this.visit_nested_var(variable_id);
            }
        }

        this.reachable
    }

    fn mark_function(&mut self, function_id: hir::FunctionId) {
        if self.reachable.insert(function_id) {
            let _ = self.visit_nested_function(function_id);
        }
    }

    fn resolve_callee(&self, callee: &'hir hir::Expr<'hir>, arity: usize) -> Vec<hir::FunctionId> {
        match &callee.peel_parens().kind {
            hir::ExprKind::Ident(resolutions) => resolutions
                .iter()
                .filter_map(|resolution| match resolution {
                    hir::Res::Item(hir::ItemId::Function(function_id))
                        if self.hir.function(*function_id).parameters.len() == arity =>
                    {
                        Some(*function_id)
                    }
                    _ => None,
                })
                .collect(),
            hir::ExprKind::Member(base, member) => {
                self.resolve_member_callee(base.peel_parens(), member.name, arity)
            }
            _ => Vec::new(),
        }
    }

    fn resolve_member_callee(
        &self,
        base: &'hir hir::Expr<'hir>,
        member: solar::interface::Symbol,
        arity: usize,
    ) -> Vec<hir::FunctionId> {
        let hir::ExprKind::Ident(resolutions) = &base.kind else { return Vec::new() };

        let mut functions = Vec::new();
        for resolution in *resolutions {
            match resolution {
                hir::Res::Item(hir::ItemId::Contract(contract_id)) => {
                    functions.extend(matching_functions(self.hir, *contract_id, member, arity));
                }
                hir::Res::Builtin(builtin) if builtin.name() == sym::super_ => {
                    let Some(contract_id) = self.current_contract else { continue };
                    let contract = self.hir.contract(contract_id);
                    for &base_id in contract.linearized_bases.iter().skip(1) {
                        functions.extend(matching_functions(self.hir, base_id, member, arity));
                    }
                }
                _ => {}
            }
        }
        functions
    }
}

impl<'hir> hir::Visit<'hir> for Reachability<'hir> {
    type BreakValue = solar::interface::data_structures::Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_function(
        &mut self,
        function: &'hir hir::Function<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        let previous_contract = self.current_contract;
        self.current_contract = function.contract;
        let result = self.walk_function(function);
        self.current_contract = previous_contract;
        result
    }

    fn visit_modifier(
        &mut self,
        modifier: &'hir hir::Modifier<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        if let hir::ItemId::Function(function_id) = modifier.id {
            self.mark_function(function_id);
        }
        self.walk_modifier(modifier)
    }

    fn visit_var(&mut self, variable: &'hir hir::Variable<'hir>) -> ControlFlow<Self::BreakValue> {
        let previous_contract = self.current_contract;
        if variable.function.is_none() {
            self.current_contract = variable.contract;
        }
        let result = self.walk_var(variable);
        self.current_contract = previous_contract;
        result
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let hir::ExprKind::Call(callee, args, opts) = &expr.kind {
            for function_id in self.resolve_callee(callee, args.len()) {
                self.mark_function(function_id);
            }

            match &callee.peel_parens().kind {
                // The call resolver above filters overloaded identifiers by arity, so do not also
                // visit the callee as a bare function reference and mark every overload reachable.
                hir::ExprKind::Ident(_) => {}
                hir::ExprKind::Member(base, _) => self.visit_expr(base)?,
                _ => self.visit_expr(callee)?,
            }
            if let Some(opts) = opts {
                for opt in *opts {
                    self.visit_expr(&opt.value)?;
                }
            }
            self.visit_call_args(args)?;
            return ControlFlow::Continue(());
        }

        if let hir::ExprKind::Ident(resolutions) = &expr.kind {
            for function_id in resolutions.iter().filter_map(|resolution| match resolution {
                hir::Res::Item(hir::ItemId::Function(function_id)) => Some(*function_id),
                _ => None,
            }) {
                self.mark_function(function_id);
            }
        }

        self.walk_expr(expr)
    }
}
