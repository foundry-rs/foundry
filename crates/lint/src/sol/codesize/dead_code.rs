use super::DeadCode;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, FunctionKind, LitKind, Visibility},
    interface::{Symbol, data_structures::Never, sym},
    sema::hir::{
        CallArgs, ContractId, Expr, ExprKind, Function, FunctionId, Hir, ItemId, Modifier, Res,
        SourceId, Type, TypeKind, Variable, VariableId, Visit,
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    sync::{Arc, LazyLock, Mutex},
};

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
        hir: &'hir Hir<'hir>,
        source_id: SourceId,
    ) {
        let analysis = dead_code_analysis(hir);

        for function_id in hir.function_ids() {
            let function = hir.function(function_id);
            if function.source != source_id || !is_dead_code_candidate(hir, function, function_id) {
                continue;
            }

            if function.virtual_ && analysis.overridden.contains(&function_id) {
                continue;
            }

            if !analysis.reachable.contains(&function_id) {
                ctx.emit(&DEAD_CODE, function.span);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct AnalysisCacheKey {
    hir: usize,
    sources: usize,
    contracts: usize,
    functions: usize,
    variables: usize,
}

#[derive(Debug)]
struct DeadCodeAnalysis {
    reachable: HashSet<FunctionId>,
    overridden: HashSet<FunctionId>,
}

const ANALYSIS_CACHE_LIMIT: usize = 16;

static ANALYSIS_CACHE: LazyLock<Mutex<HashMap<AnalysisCacheKey, Arc<DeadCodeAnalysis>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn dead_code_analysis(hir: &Hir<'_>) -> Arc<DeadCodeAnalysis> {
    let key = AnalysisCacheKey {
        hir: std::ptr::from_ref(hir).addr(),
        sources: hir.source_ids().len(),
        contracts: hir.contract_ids().len(),
        functions: hir.function_ids().len(),
        variables: hir.variable_ids().len(),
    };

    {
        let cache = ANALYSIS_CACHE.lock().unwrap();
        if let Some(analysis) = cache.get(&key).cloned() {
            return analysis;
        }
    }

    let analysis = Arc::new(DeadCodeAnalysis {
        reachable: Reachability::compute(hir),
        overridden: collect_overridden_functions(hir),
    });

    let mut cache = ANALYSIS_CACHE.lock().unwrap();
    // `check_nested_source` is called once per source and constructs fresh pass instances each
    // time, so cache per-HIR analysis across those calls. Keep it bounded for long-running test
    // processes that lint many independent HIRs in one process.
    if cache.len() >= ANALYSIS_CACHE_LIMIT {
        cache.clear();
    }
    cache.entry(key).or_insert_with(|| analysis).clone()
}

fn is_dead_code_candidate(hir: &Hir<'_>, function: &Function<'_>, function_id: FunctionId) -> bool {
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

fn is_entry_point(function: &Function<'_>) -> bool {
    function.body.is_some()
        && (function.visibility >= Visibility::Public
            || matches!(
                function.kind,
                FunctionKind::Constructor | FunctionKind::Fallback | FunctionKind::Receive
            ))
}

fn collect_overridden_functions(hir: &Hir<'_>) -> HashSet<FunctionId> {
    let mut overridden = HashSet::new();

    for function_id in hir.function_ids() {
        let function = hir.function(function_id);
        if !function.override_ {
            continue;
        }

        let Some(contract_id) = function.contract else { continue };
        let contract = hir.contract(contract_id);

        let bases = if function.overrides.is_empty() {
            contract.linearized_bases.get(1..).unwrap_or_default()
        } else {
            function.overrides
        };

        for &base_id in bases {
            for base_function_id in matching_overridden_functions(hir, base_id, function) {
                if hir.function(base_function_id).virtual_ {
                    overridden.insert(base_function_id);
                }
            }
        }
    }

    overridden
}

fn matching_overridden_functions(
    hir: &Hir<'_>,
    contract_id: ContractId,
    overriding: &Function<'_>,
) -> Vec<FunctionId> {
    let Some(name) = overriding.name else { return Vec::new() };
    hir.contract(contract_id)
        .all_functions()
        .filter(|&function_id| {
            let function = hir.function(function_id);
            function.name.is_some_and(|ident| ident.name == name.name)
                && same_parameter_types(hir, function.parameters, overriding.parameters)
        })
        .collect()
}

fn matching_functions(
    hir: &Hir<'_>,
    contract_id: ContractId,
    name: Symbol,
    arity: usize,
) -> Vec<FunctionId> {
    hir.contract(contract_id)
        .all_functions()
        .filter(|&function_id| {
            let function = hir.function(function_id);
            function.name.is_some_and(|ident| ident.name == name)
                && function.parameters.len() == arity
        })
        .collect()
}

fn same_parameter_types(hir: &Hir<'_>, lhs: &[VariableId], rhs: &[VariableId]) -> bool {
    lhs.len() == rhs.len()
        && lhs
            .iter()
            .zip(rhs)
            .all(|(&lhs, &rhs)| same_type(hir, &hir.variable(lhs).ty, &hir.variable(rhs).ty))
}

fn same_type(hir: &Hir<'_>, lhs: &Type<'_>, rhs: &Type<'_>) -> bool {
    match (&lhs.kind, &rhs.kind) {
        (TypeKind::Elementary(lhs), TypeKind::Elementary(rhs)) => lhs == rhs,
        (TypeKind::Array(lhs), TypeKind::Array(rhs)) => {
            same_type(hir, &lhs.element, &rhs.element)
                && same_array_size(lhs.size.map(Expr::peel_parens), rhs.size.map(Expr::peel_parens))
        }
        (TypeKind::Function(lhs), TypeKind::Function(rhs)) => {
            lhs.visibility == rhs.visibility
                && lhs.state_mutability == rhs.state_mutability
                && same_parameter_types(hir, lhs.parameters, rhs.parameters)
                && same_parameter_types(hir, lhs.returns, rhs.returns)
        }
        (TypeKind::Mapping(lhs), TypeKind::Mapping(rhs)) => {
            same_type(hir, &lhs.key, &rhs.key) && same_type(hir, &lhs.value, &rhs.value)
        }
        (TypeKind::Custom(lhs), TypeKind::Custom(rhs)) => lhs == rhs,
        (TypeKind::Err(_), TypeKind::Err(_)) => true,
        _ => false,
    }
}

fn same_array_size(lhs: Option<&Expr<'_>>, rhs: Option<&Expr<'_>>) -> bool {
    match (lhs, rhs) {
        (None, None) => true,
        (Some(lhs), Some(rhs)) => same_expr(lhs, rhs),
        _ => false,
    }
}

fn same_expr(lhs: &Expr<'_>, rhs: &Expr<'_>) -> bool {
    match (&lhs.peel_parens().kind, &rhs.peel_parens().kind) {
        (ExprKind::Lit(lhs), ExprKind::Lit(rhs)) => same_lit_kind(&lhs.kind, &rhs.kind),
        (ExprKind::Ident(lhs), ExprKind::Ident(rhs)) => lhs == rhs,
        (ExprKind::Unary(lhs_op, lhs), ExprKind::Unary(rhs_op, rhs)) => {
            lhs_op.kind == rhs_op.kind && same_expr(lhs, rhs)
        }
        (ExprKind::Binary(lhs_l, lhs_op, lhs_r), ExprKind::Binary(rhs_l, rhs_op, rhs_r)) => {
            lhs_op.kind == rhs_op.kind && same_expr(lhs_l, rhs_l) && same_expr(lhs_r, rhs_r)
        }
        _ => false,
    }
}

fn same_lit_kind(lhs: &LitKind<'_>, rhs: &LitKind<'_>) -> bool {
    match (lhs, rhs) {
        (LitKind::Str(lhs_kind, lhs_value, _), LitKind::Str(rhs_kind, rhs_value, _)) => {
            lhs_kind == rhs_kind && lhs_value == rhs_value
        }
        (LitKind::Number(lhs), LitKind::Number(rhs)) => lhs == rhs,
        (LitKind::Rational(lhs), LitKind::Rational(rhs)) => lhs == rhs,
        (LitKind::Address(lhs), LitKind::Address(rhs)) => lhs == rhs,
        (LitKind::Bool(lhs), LitKind::Bool(rhs)) => lhs == rhs,
        (LitKind::Err(_), LitKind::Err(_)) => true,
        _ => false,
    }
}

struct Reachability<'hir> {
    hir: &'hir Hir<'hir>,
    reachable: HashSet<FunctionId>,
    current_contract: Option<ContractId>,
}

impl<'hir> Reachability<'hir> {
    fn compute(hir: &'hir Hir<'hir>) -> HashSet<FunctionId> {
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

    fn mark_function(&mut self, function_id: FunctionId) {
        if self.reachable.insert(function_id) {
            let _ = self.visit_nested_function(function_id);
        }
    }

    fn resolve_callee(&self, callee: &'hir Expr<'hir>, args: CallArgs<'hir>) -> Vec<FunctionId> {
        let arity = args.len();
        match &callee.peel_parens().kind {
            ExprKind::Ident(resolutions) => resolutions
                .iter()
                .filter_map(|resolution| match resolution {
                    Res::Item(ItemId::Function(function_id))
                        if self.hir.function(*function_id).parameters.len() == arity =>
                    {
                        Some(*function_id)
                    }
                    _ => None,
                })
                .collect(),
            ExprKind::Member(base, member) => {
                self.resolve_member_callee(base.peel_parens(), member.name, arity)
            }
            _ => Vec::new(),
        }
    }

    fn resolve_member_callee(
        &self,
        base: &'hir Expr<'hir>,
        member: Symbol,
        arity: usize,
    ) -> Vec<FunctionId> {
        let mut functions = Vec::new();
        if is_super(base) {
            let Some(contract_id) = self.current_contract else { return functions };
            let contract = self.hir.contract(contract_id);
            for &base_id in contract.linearized_bases.iter().skip(1) {
                functions.extend(matching_functions(self.hir, base_id, member, arity));
            }
            return functions;
        }

        for contract_id in self.resolve_static_contracts(base) {
            functions.extend(matching_functions(self.hir, contract_id, member, arity));
        }
        functions
    }

    fn resolve_static_contracts(&self, expr: &'hir Expr<'hir>) -> Vec<ContractId> {
        match &expr.peel_parens().kind {
            ExprKind::Ident(resolutions) => resolutions
                .iter()
                .filter_map(|resolution| match resolution {
                    Res::Item(ItemId::Contract(contract_id)) => Some(*contract_id),
                    _ => None,
                })
                .collect(),
            ExprKind::Type(Type {
                kind: TypeKind::Custom(ItemId::Contract(contract_id)), ..
            })
            | ExprKind::TypeCall(Type {
                kind: TypeKind::Custom(ItemId::Contract(contract_id)),
                ..
            }) => vec![*contract_id],
            // Casts like `C(addr).foo()` are instance calls, not static base calls. Resolving them
            // here would only add false edges for public/external calls, which are not dead-code
            // candidates.
            _ => Vec::new(),
        }
    }
}

fn is_super(expr: &Expr<'_>) -> bool {
    let ExprKind::Ident(resolutions) = &expr.peel_parens().kind else { return false };
    resolutions.iter().any(|resolution| match resolution {
        Res::Builtin(builtin) => builtin.name() == sym::super_,
        _ => false,
    })
}

impl<'hir> Visit<'hir> for Reachability<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_function(&mut self, function: &'hir Function<'hir>) -> ControlFlow<Self::BreakValue> {
        let previous_contract = self.current_contract;
        self.current_contract = function.contract;
        let result = self.walk_function(function);
        self.current_contract = previous_contract;
        result
    }

    fn visit_modifier(&mut self, modifier: &'hir Modifier<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ItemId::Function(function_id) = modifier.id {
            self.mark_function(function_id);
        }
        self.walk_modifier(modifier)
    }

    fn visit_var(&mut self, variable: &'hir Variable<'hir>) -> ControlFlow<Self::BreakValue> {
        let previous_contract = self.current_contract;
        if variable.function.is_none() {
            self.current_contract = variable.contract;
        }
        let result = self.walk_var(variable);
        self.current_contract = previous_contract;
        result
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Call(callee, args, opts) = &expr.kind {
            for function_id in self.resolve_callee(callee, *args) {
                self.mark_function(function_id);
            }

            match &callee.peel_parens().kind {
                // The call resolver above filters overloaded identifiers by arity, so do not also
                // visit the callee as a bare function reference and mark every overload reachable.
                ExprKind::Ident(_) => {}
                ExprKind::Member(base, _) => self.visit_expr(base)?,
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

        if let ExprKind::Ident(resolutions) = &expr.kind {
            for function_id in resolutions.iter().filter_map(|resolution| match resolution {
                Res::Item(ItemId::Function(function_id)) => Some(*function_id),
                _ => None,
            }) {
                self.mark_function(function_id);
            }
        }

        self.walk_expr(expr)
    }
}
