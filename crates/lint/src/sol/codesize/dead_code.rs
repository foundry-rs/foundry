use super::DeadCode;
use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{self, ContractKind, ElementaryType, FunctionKind, LitKind, Visibility},
    interface::{Symbol, data_structures::Never, source_map::FileName, sym},
    sema::hir::{
        CallArgs, ContractId, Expr, ExprKind, Function, FunctionId, Hir, ItemId, Modifier, Res,
        SourceId, Type, TypeKind, Variable, VariableId, Visit,
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    path::Path,
};

declare_forge_lint!(
    DEAD_CODE,
    Severity::CodeSize,
    "dead-code",
    "internal or private function is never used"
);

impl<'ast> ProjectLintPass<'ast> for DeadCode {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]) {
        if !ctx.is_lint_enabled(DEAD_CODE.id()) {
            return;
        }

        let gcx = ctx.gcx();
        let hir = &gcx.hir;
        let input_sources = input_sources(hir, sources);
        if input_sources.emitted.is_empty() {
            return;
        }

        let using_for = collect_using_for(hir, sources, &input_sources);
        let analysis = dead_code_analysis(hir, &input_sources.included, &using_for);

        for function_id in hir.function_ids() {
            let function = hir.function(function_id);
            let Some(&source_idx) = input_sources.emitted.get(&function.source) else { continue };

            if !is_dead_code_candidate(hir, function, function_id) {
                continue;
            }

            if function.virtual_ && analysis.overridden.contains(&function_id) {
                continue;
            }

            if !analysis.reachable.contains(&function_id) {
                ctx.emit(&sources[source_idx], &DEAD_CODE, function.span);
            }
        }
    }
}

#[derive(Debug)]
struct DeadCodeAnalysis {
    reachable: HashSet<FunctionId>,
    overridden: HashSet<FunctionId>,
}

fn dead_code_analysis(
    hir: &Hir<'_>,
    included_sources: &HashSet<SourceId>,
    using_for: &UsingFor,
) -> DeadCodeAnalysis {
    DeadCodeAnalysis {
        reachable: Reachability::compute(hir, included_sources, using_for),
        overridden: collect_overridden_functions(hir, included_sources),
    }
}

#[derive(Default)]
struct InputSources {
    emitted: HashMap<SourceId, usize>,
    included: HashSet<SourceId>,
}

fn input_sources<'ast>(hir: &Hir<'_>, sources: &[ProjectSource<'ast>]) -> InputSources {
    let source_indices_by_path: HashMap<&Path, usize> =
        sources.iter().enumerate().map(|(idx, source)| (source.path.as_path(), idx)).collect();

    let mut input_sources = InputSources::default();
    for (source_id, source) in hir.sources_enumerated() {
        let FileName::Real(path) = &source.file.name else { continue };
        let Some(&source_idx) = source_indices_by_path.get(path.as_path()) else { continue };

        if sources[source_idx].is_test_or_script {
            continue;
        }
        input_sources.emitted.insert(source_id, source_idx);
        input_sources.included.insert(source_id);
    }
    input_sources
}

#[derive(Default)]
struct UsingFor {
    global_functions: Vec<FunctionId>,
    source_functions: HashMap<SourceId, Vec<FunctionId>>,
    contract_functions: HashMap<ContractId, Vec<FunctionId>>,
}

struct UsingLookup {
    contracts_by_source_name: HashMap<(SourceId, Symbol), ContractId>,
    contracts_by_name: HashMap<Symbol, Vec<ContractId>>,
    free_functions_by_name: HashMap<Symbol, Vec<FunctionId>>,
    contract_functions_by_name: HashMap<(ContractId, Symbol), Vec<FunctionId>>,
}

fn collect_using_for<'ast>(
    hir: &Hir<'_>,
    sources: &[ProjectSource<'ast>],
    input_sources: &InputSources,
) -> UsingFor {
    let lookup = UsingLookup::new(hir);
    let mut using_for = UsingFor::default();

    for (&source_id, &source_idx) in &input_sources.emitted {
        for item in sources[source_idx].ast.items.iter() {
            match &item.kind {
                ast::ItemKind::Using(using) => {
                    let functions = lookup.resolve_using_functions(hir, using);
                    if using.global {
                        using_for.global_functions.extend(functions);
                    } else {
                        using_for.source_functions.entry(source_id).or_default().extend(functions);
                    }
                }
                ast::ItemKind::Contract(contract) => {
                    let Some(&contract_id) =
                        lookup.contracts_by_source_name.get(&(source_id, contract.name.name))
                    else {
                        continue;
                    };
                    for item in contract.body.iter() {
                        if let ast::ItemKind::Using(using) = &item.kind {
                            let functions = lookup.resolve_using_functions(hir, using);
                            using_for
                                .contract_functions
                                .entry(contract_id)
                                .or_default()
                                .extend(functions);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    using_for
}

impl UsingLookup {
    fn new(hir: &Hir<'_>) -> Self {
        let mut this = Self {
            contracts_by_source_name: HashMap::new(),
            contracts_by_name: HashMap::new(),
            free_functions_by_name: HashMap::new(),
            contract_functions_by_name: HashMap::new(),
        };

        for contract_id in hir.contract_ids() {
            let contract = hir.contract(contract_id);
            this.contracts_by_source_name
                .insert((contract.source, contract.name.name), contract_id);
            this.contracts_by_name.entry(contract.name.name).or_default().push(contract_id);

            for function_id in contract.functions() {
                if let Some(name) = hir.function(function_id).name {
                    this.contract_functions_by_name
                        .entry((contract_id, name.name))
                        .or_default()
                        .push(function_id);
                }
            }
        }

        for function_id in hir.function_ids() {
            let function = hir.function(function_id);
            if function.contract.is_none()
                && let Some(name) = function.name
            {
                this.free_functions_by_name.entry(name.name).or_default().push(function_id);
            }
        }

        this
    }

    fn resolve_using_functions(
        &self,
        hir: &Hir<'_>,
        using: &ast::UsingDirective<'_>,
    ) -> Vec<FunctionId> {
        match &using.list {
            ast::UsingList::Single(path) => self.resolve_using_path(hir, path),
            ast::UsingList::Multiple(items) => {
                items.iter().flat_map(|(path, _)| self.resolve_using_path(hir, path)).collect()
            }
        }
    }

    fn resolve_using_path(&self, hir: &Hir<'_>, path: &ast::PathSlice) -> Vec<FunctionId> {
        match path.segments() {
            [name] => {
                let mut functions =
                    self.free_functions_by_name.get(&name.name).cloned().unwrap_or_default();
                for &contract_id in self.contracts_by_name.get(&name.name).into_iter().flatten() {
                    functions.extend(hir.contract(contract_id).functions());
                }
                functions
            }
            [contract_name, function_name] => self
                .contracts_by_name
                .get(&contract_name.name)
                .into_iter()
                .flatten()
                .flat_map(|&contract_id| {
                    self.contract_functions_by_name
                        .get(&(contract_id, function_name.name))
                        .into_iter()
                        .flatten()
                        .copied()
                })
                .collect(),
            _ => Vec::new(),
        }
    }
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

fn collect_overridden_functions(
    hir: &Hir<'_>,
    included_sources: &HashSet<SourceId>,
) -> HashSet<FunctionId> {
    let mut overridden = HashSet::new();

    for function_id in hir.function_ids() {
        let function = hir.function(function_id);
        if !function.override_ || !included_sources.contains(&function.source) {
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
                let base_function = hir.function(base_function_id);
                if included_sources.contains(&base_function.source) && base_function.virtual_ {
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
    args: CallArgs<'_>,
) -> Vec<FunctionId> {
    select_matching_functions(
        hir,
        hir.contract(contract_id).all_functions().filter(|&function_id| {
            let function = hir.function(function_id);
            function.name.is_some_and(|ident| ident.name == name)
        }),
        args,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MatchQuality {
    Exact,
    Possible,
}

fn select_matching_functions(
    hir: &Hir<'_>,
    candidates: impl IntoIterator<Item = FunctionId>,
    args: CallArgs<'_>,
) -> Vec<FunctionId> {
    let mut exact = Vec::new();
    let mut possible = Vec::new();

    for function_id in candidates {
        match function_matches_args(hir, hir.function(function_id), args) {
            Some(MatchQuality::Exact) => exact.push(function_id),
            Some(MatchQuality::Possible) => possible.push(function_id),
            None => {}
        }
    }

    if exact.is_empty() { possible } else { exact }
}

fn select_matching_member_functions(
    hir: &Hir<'_>,
    candidates: impl IntoIterator<Item = FunctionId>,
    receiver: &Expr<'_>,
    args: CallArgs<'_>,
) -> Vec<FunctionId> {
    let mut exact = Vec::new();
    let mut possible = Vec::new();

    for function_id in candidates {
        match function_matches_member_args(hir, hir.function(function_id), receiver, args) {
            Some(MatchQuality::Exact) => exact.push(function_id),
            Some(MatchQuality::Possible) => possible.push(function_id),
            None => {}
        }
    }

    if exact.is_empty() { possible } else { exact }
}

fn function_matches_args(
    hir: &Hir<'_>,
    function: &Function<'_>,
    args: CallArgs<'_>,
) -> Option<MatchQuality> {
    parameters_match_args(hir, function.parameters, args)
}

fn function_matches_member_args(
    hir: &Hir<'_>,
    function: &Function<'_>,
    receiver: &Expr<'_>,
    args: CallArgs<'_>,
) -> Option<MatchQuality> {
    let (&receiver_param, parameters) = function.parameters.split_first()?;
    if parameters.len() != args.len() {
        return None;
    }

    let mut quality = MatchQuality::Exact;
    check_arg_match(hir, receiver, receiver_param, &mut quality)?;
    parameters_match_args_with_quality(hir, parameters, args, &mut quality)?;

    Some(quality)
}

fn parameters_match_args(
    hir: &Hir<'_>,
    parameters: &[VariableId],
    args: CallArgs<'_>,
) -> Option<MatchQuality> {
    if parameters.len() != args.len() {
        return None;
    }

    let mut quality = MatchQuality::Exact;
    parameters_match_args_with_quality(hir, parameters, args, &mut quality)?;
    Some(quality)
}

fn parameters_match_args_with_quality(
    hir: &Hir<'_>,
    parameters: &[VariableId],
    args: CallArgs<'_>,
    quality: &mut MatchQuality,
) -> Option<()> {
    match args.kind {
        solar::sema::hir::CallArgsKind::Unnamed(exprs) => {
            for (expr, &param) in exprs.iter().zip(parameters) {
                check_arg_match(hir, expr, param, quality)?;
            }
        }
        solar::sema::hir::CallArgsKind::Named(named_args) => {
            for arg in named_args {
                let param = parameters.iter().copied().find(|&param| {
                    hir.variable(param).name.is_some_and(|ident| ident.name == arg.name.name)
                })?;
                check_arg_match(hir, &arg.value, param, quality)?;
            }
        }
    }

    Some(())
}

fn check_arg_match(
    hir: &Hir<'_>,
    expr: &Expr<'_>,
    param: VariableId,
    quality: &mut MatchQuality,
) -> Option<()> {
    match expr_matches_type(hir, expr, &hir.variable(param).ty) {
        Some(MatchQuality::Exact) => Some(()),
        Some(MatchQuality::Possible) => {
            *quality = MatchQuality::Possible;
            Some(())
        }
        None => None,
    }
}

fn expr_matches_type(hir: &Hir<'_>, expr: &Expr<'_>, ty: &Type<'_>) -> Option<MatchQuality> {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Ident(resolutions) => {
            let mut variables = resolutions.iter().filter_map(|res| match res {
                Res::Item(ItemId::Variable(variable_id)) => Some(*variable_id),
                _ => None,
            });
            let Some(variable_id) = variables.next() else { return Some(MatchQuality::Possible) };
            if variables.next().is_some() {
                return Some(MatchQuality::Possible);
            }
            type_matches_param(hir, &hir.variable(variable_id).ty, ty)
        }
        ExprKind::Call(callee, ..) => {
            if let Some(expr_ty) = call_expr_type(callee.peel_parens()) {
                type_matches_param(hir, expr_ty, ty)
            } else {
                Some(MatchQuality::Possible)
            }
        }
        ExprKind::Lit(lit) => lit_matches_type(&lit.kind, ty),
        ExprKind::New(expr_ty) => type_matches_param(hir, expr_ty, ty),
        ExprKind::Payable(_) => match ty.kind {
            TypeKind::Elementary(ElementaryType::Address(true)) => Some(MatchQuality::Exact),
            TypeKind::Elementary(ElementaryType::Address(false)) => Some(MatchQuality::Possible),
            _ => None,
        },
        ExprKind::Ternary(_, lhs, rhs) => {
            match (expr_matches_type(hir, lhs, ty)?, expr_matches_type(hir, rhs, ty)?) {
                (MatchQuality::Exact, MatchQuality::Exact) => Some(MatchQuality::Exact),
                _ => Some(MatchQuality::Possible),
            }
        }
        ExprKind::Err(_) => Some(MatchQuality::Possible),
        _ => Some(MatchQuality::Possible),
    }
}

fn type_matches_param(
    hir: &Hir<'_>,
    arg_ty: &Type<'_>,
    param_ty: &Type<'_>,
) -> Option<MatchQuality> {
    if same_type(hir, arg_ty, param_ty) {
        return Some(MatchQuality::Exact);
    }

    match (&arg_ty.kind, &param_ty.kind) {
        (
            TypeKind::Elementary(ElementaryType::UInt(arg_size)),
            TypeKind::Elementary(ElementaryType::UInt(param_size)),
        )
        | (
            TypeKind::Elementary(ElementaryType::Int(arg_size)),
            TypeKind::Elementary(ElementaryType::Int(param_size)),
        ) if arg_size.bits() <= param_size.bits() => Some(MatchQuality::Possible),
        (
            TypeKind::Elementary(ElementaryType::Address(true)),
            TypeKind::Elementary(ElementaryType::Address(false)),
        ) => Some(MatchQuality::Possible),
        (TypeKind::Err(_), _) | (_, TypeKind::Err(_)) => Some(MatchQuality::Possible),
        _ => None,
    }
}

const fn call_expr_type<'hir>(callee: &'hir Expr<'hir>) -> Option<&'hir Type<'hir>> {
    match &callee.kind {
        ExprKind::Type(ty) | ExprKind::TypeCall(ty) | ExprKind::New(ty) => Some(ty),
        _ => None,
    }
}

const fn lit_matches_type(lit: &LitKind<'_>, ty: &Type<'_>) -> Option<MatchQuality> {
    let TypeKind::Elementary(elementary) = ty.kind else {
        return match lit {
            LitKind::Err(_) => Some(MatchQuality::Possible),
            _ => None,
        };
    };

    match (lit, elementary) {
        (LitKind::Bool(_), ElementaryType::Bool)
        | (LitKind::Address(_), ElementaryType::Address(false)) => Some(MatchQuality::Exact),
        (LitKind::Address(_), ElementaryType::Address(true)) => Some(MatchQuality::Possible),
        (LitKind::Str(..), ElementaryType::String | ElementaryType::Bytes) => {
            Some(MatchQuality::Possible)
        }
        (
            LitKind::Number(_) | LitKind::Rational(_),
            ElementaryType::Int(_)
            | ElementaryType::UInt(_)
            | ElementaryType::Fixed(_, _)
            | ElementaryType::UFixed(_, _),
        ) => Some(MatchQuality::Possible),
        (LitKind::Err(_), _) => Some(MatchQuality::Possible),
        _ => None,
    }
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

struct Reachability<'hir, 'a> {
    hir: &'hir Hir<'hir>,
    included_sources: HashSet<SourceId>,
    using_for: &'a UsingFor,
    reachable: HashSet<FunctionId>,
    current_contract: Option<ContractId>,
    current_source: Option<SourceId>,
}

impl<'hir, 'a> Reachability<'hir, 'a> {
    fn compute(
        hir: &'hir Hir<'hir>,
        included_sources: &HashSet<SourceId>,
        using_for: &'a UsingFor,
    ) -> HashSet<FunctionId> {
        let mut this = Self {
            hir,
            included_sources: included_sources.clone(),
            using_for,
            reachable: HashSet::new(),
            current_contract: None,
            current_source: None,
        };

        for function_id in hir.function_ids() {
            if this.is_included_function(function_id) && is_entry_point(hir.function(function_id)) {
                this.mark_function(function_id);
            }
        }

        for variable_id in hir.variable_ids() {
            let variable = hir.variable(variable_id);
            if this.is_included_source(variable.source)
                && variable.function.is_none()
                && variable.initializer.is_some()
            {
                let _ = this.visit_nested_var(variable_id);
            }
        }

        this.reachable
    }

    fn is_included_source(&self, source_id: SourceId) -> bool {
        self.included_sources.contains(&source_id)
    }

    fn is_included_function(&self, function_id: FunctionId) -> bool {
        self.is_included_source(self.hir.function(function_id).source)
    }

    fn mark_function(&mut self, function_id: FunctionId) {
        if !self.is_included_function(function_id) {
            return;
        }
        if self.reachable.insert(function_id) {
            let _ = self.visit_nested_function(function_id);
        }
    }

    fn resolve_callee(&self, callee: &'hir Expr<'hir>, args: CallArgs<'hir>) -> Vec<FunctionId> {
        match &callee.peel_parens().kind {
            ExprKind::Ident(resolutions) => select_matching_functions(
                self.hir,
                resolutions.iter().filter_map(|resolution| match resolution {
                    Res::Item(ItemId::Function(function_id)) => Some(*function_id),
                    _ => None,
                }),
                args,
            ),
            ExprKind::Member(base, member) => {
                self.resolve_member_callee(base.peel_parens(), member.name, args)
            }
            _ => Vec::new(),
        }
    }

    fn resolve_member_callee(
        &self,
        base: &'hir Expr<'hir>,
        member: Symbol,
        args: CallArgs<'hir>,
    ) -> Vec<FunctionId> {
        let mut functions = Vec::new();
        if is_super(base) {
            let Some(contract_id) = self.current_contract else { return functions };
            let contract = self.hir.contract(contract_id);
            for &base_id in contract.linearized_bases.iter().skip(1) {
                functions = matching_functions(self.hir, base_id, member, args);
                if !functions.is_empty() {
                    return functions;
                }
            }
            return functions;
        }

        for contract_id in self.resolve_static_contracts(base) {
            functions.extend(matching_functions(self.hir, contract_id, member, args));
        }
        functions.extend(self.resolve_using_for(base, member, args));
        functions
    }

    fn resolve_using_for(
        &self,
        base: &'hir Expr<'hir>,
        member: Symbol,
        args: CallArgs<'hir>,
    ) -> Vec<FunctionId> {
        let Some(source_id) = self.current_source else { return Vec::new() };
        let mut seen = HashSet::new();
        let candidates = self
            .using_for
            .global_functions
            .iter()
            .chain(self.using_for.source_functions.get(&source_id).into_iter().flatten())
            .chain(
                self.current_contract
                    .and_then(|contract_id| self.using_for.contract_functions.get(&contract_id))
                    .into_iter()
                    .flatten(),
            )
            .copied()
            .filter(|&function_id| seen.insert(function_id))
            .filter(|&function_id| {
                self.hir.function(function_id).name.is_some_and(|ident| ident.name == member)
            });

        select_matching_member_functions(self.hir, candidates, base, args)
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

impl<'hir> Visit<'hir> for Reachability<'hir, '_> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_function(&mut self, function: &'hir Function<'hir>) -> ControlFlow<Self::BreakValue> {
        let previous_contract = self.current_contract;
        let previous_source = self.current_source;
        self.current_contract = function.contract;
        self.current_source = Some(function.source);
        let result = self.walk_function(function);
        self.current_contract = previous_contract;
        self.current_source = previous_source;
        result
    }

    fn visit_modifier(&mut self, modifier: &'hir Modifier<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ItemId::Function(function_id) = modifier.id {
            self.mark_function(function_id);
        }
        self.walk_modifier(modifier)
    }

    fn visit_var(&mut self, variable: &'hir Variable<'hir>) -> ControlFlow<Self::BreakValue> {
        if !self.is_included_source(variable.source) {
            return ControlFlow::Continue(());
        }

        let previous_contract = self.current_contract;
        let previous_source = self.current_source;
        if variable.function.is_none() {
            self.current_contract = variable.contract;
        }
        self.current_source = Some(variable.source);
        let result = self.walk_var(variable);
        self.current_contract = previous_contract;
        self.current_source = previous_source;
        result
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Call(callee, args, opts) = &expr.kind {
            for function_id in self.resolve_callee(callee, *args) {
                self.mark_function(function_id);
            }

            match &callee.peel_parens().kind {
                // The call resolver above handles overloaded identifiers, so do not also visit the
                // callee as a bare function reference and mark every overload reachable.
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
