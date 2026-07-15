use super::ProtectedVars;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, DataLocation, ElementaryType, FunctionKind, Visibility},
    interface::sym,
    sema::{
        Gcx,
        hir::{
            self, ContractId, ExprKind, FunctionId, ItemId, NatSpecKind, Res, StmtKind, VariableId,
        },
        ty::TyKind,
    },
};
use std::collections::{HashMap, HashSet};

declare_forge_lint!(
    PROTECTED_VARS,
    Severity::High,
    "protected-vars",
    "protected variable is written without its required protection"
);

impl<'hir> LateLintPass<'hir> for ProtectedVars {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        contract_id: ContractId,
    ) {
        let contract = hir.contract(contract_id);
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract)
            || contract.linearization_failed()
            || !is_most_derived_contract(hir, contract_id)
        {
            return;
        }

        let protected = protected_variables(gcx, hir, contract.linearized_bases);
        if protected.is_empty() {
            return;
        }

        let targets = ProtectionTargets::new(gcx, hir, contract.linearized_bases);
        for entry_id in effective_entry_points(gcx, hir, contract.linearized_bases) {
            let mut analyzer = EntryAnalyzer::new(gcx, hir, contract.linearized_bases);
            let (writes, calls) = analyzer.analyze(entry_id);

            for var_id in writes {
                let Some(requirements) = protected.get(&var_id) else { continue };
                for requirement in requirements {
                    let Some(target_id) = targets.resolve(requirement) else { continue };
                    if calls.contains(&target_id) {
                        continue;
                    }

                    let entry = hir.function(entry_id);
                    let span = entry.name.map_or(entry.keyword_span(), |name| name.span);
                    let variable = hir
                        .variable(var_id)
                        .name
                        .map_or_else(|| "<unnamed>".to_string(), |name| name.as_str().to_string());
                    ctx.emit_with_msg(
                        &PROTECTED_VARS,
                        span,
                        format!(
                            "protected variable `{variable}` is written without `{requirement}`"
                        ),
                    );
                }
            }
        }
    }
}

/// Slither analyzes the effective entry points of leaf contracts so inherited declarations are
/// interpreted in the context in which they are ultimately deployed.
fn is_most_derived_contract(hir: &hir::Hir<'_>, contract_id: ContractId) -> bool {
    !hir.contract_ids().any(|candidate_id| {
        candidate_id != contract_id
            && hir
                .contract(candidate_id)
                .linearized_bases
                .get(1..)
                .is_some_and(|bases| bases.contains(&contract_id))
    })
}

fn protected_variables(
    gcx: Gcx<'_>,
    hir: &hir::Hir<'_>,
    bases: &[ContractId],
) -> HashMap<VariableId, Vec<String>> {
    let mut protected = HashMap::new();

    for &contract_id in bases {
        for var_id in hir.contract(contract_id).variables() {
            let var = hir.variable(var_id);
            if !var.kind.is_state() {
                continue;
            }

            let requirements: Vec<_> = gcx
                .natspec_doc_comments(var.doc)
                .iter()
                .filter_map(|item| {
                    let NatSpecKind::Custom { name } = item.kind else { return None };
                    if name.as_str() != "security" {
                        return None;
                    }
                    parse_write_protection(item.content()).map(str::to_owned)
                })
                .collect();
            if !requirements.is_empty() {
                protected.insert(var_id, requirements);
            }
        }
    }

    protected
}

fn parse_write_protection(content: &str) -> Option<&str> {
    let value = content.split_once("write-protection=\"")?.1;
    let (signature, _) = value.split_once('"')?;
    (!signature.is_empty()).then_some(signature)
}

struct ProtectionTargets {
    functions: HashMap<String, FunctionId>,
    modifiers: HashMap<String, FunctionId>,
}

impl ProtectionTargets {
    fn new(gcx: Gcx<'_>, hir: &hir::Hir<'_>, bases: &[ContractId]) -> Self {
        let mut this = Self { functions: HashMap::new(), modifiers: HashMap::new() };

        // Linearization starts at the most-derived contract. Keeping the first signature excludes
        // shadowed base members, matching the effective member set used by Slither.
        for &contract_id in bases {
            for function_id in hir.contract(contract_id).functions() {
                let function = hir.function(function_id);
                if function.name.is_none() {
                    continue;
                }
                let signature = gcx.item_signature(ItemId::Function(function_id)).to_string();
                match function.kind {
                    FunctionKind::Function => {
                        this.functions.entry(signature).or_insert(function_id);
                    }
                    FunctionKind::Modifier => {
                        this.modifiers.entry(signature).or_insert(function_id);
                    }
                    FunctionKind::Constructor | FunctionKind::Fallback | FunctionKind::Receive => {}
                }
            }
        }

        this
    }

    fn resolve(&self, signature: &str) -> Option<FunctionId> {
        self.functions.get(signature).or_else(|| self.modifiers.get(signature)).copied()
    }
}

fn effective_entry_points(
    gcx: Gcx<'_>,
    hir: &hir::Hir<'_>,
    bases: &[ContractId],
) -> Vec<FunctionId> {
    let mut seen_functions = HashSet::new();
    let mut seen_fallback = false;
    let mut seen_receive = false;
    let mut entries = Vec::new();

    for &contract_id in bases {
        for function_id in hir.contract(contract_id).all_functions() {
            let function = hir.function(function_id);
            match function.kind {
                FunctionKind::Function => {
                    if !matches!(function.visibility, Visibility::Public | Visibility::External) {
                        continue;
                    }
                    let signature = gcx.item_signature(ItemId::Function(function_id));
                    if seen_functions.insert(signature) {
                        entries.push(function_id);
                    }
                }
                FunctionKind::Fallback if !seen_fallback => {
                    seen_fallback = true;
                    entries.push(function_id);
                }
                FunctionKind::Receive if !seen_receive => {
                    seen_receive = true;
                    entries.push(function_id);
                }
                FunctionKind::Constructor
                | FunctionKind::Modifier
                | FunctionKind::Fallback
                | FunctionKind::Receive => {}
            }
        }
    }

    entries
}

struct EntryAnalyzer<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    bases: &'hir [ContractId],
    writes: HashSet<VariableId>,
    calls: HashSet<FunctionId>,
    storage_aliases: HashMap<VariableId, VariableId>,
    stack: Vec<FunctionId>,
}

impl<'hir> EntryAnalyzer<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir hir::Hir<'hir>, bases: &'hir [ContractId]) -> Self {
        Self {
            gcx,
            hir,
            bases,
            writes: HashSet::new(),
            calls: HashSet::new(),
            storage_aliases: HashMap::new(),
            stack: Vec::new(),
        }
    }

    fn analyze(&mut self, entry_id: FunctionId) -> (HashSet<VariableId>, HashSet<FunctionId>) {
        self.analyze_function(entry_id);
        (std::mem::take(&mut self.writes), std::mem::take(&mut self.calls))
    }

    fn analyze_function(&mut self, function_id: FunctionId) {
        if self.stack.contains(&function_id) {
            return;
        }

        let function = self.hir.function(function_id);
        let Some(body) = function.body else { return };
        self.stack.push(function_id);
        self.analyze_modifiers(function);
        self.analyze_block(body);
        self.stack.pop();
    }

    fn analyze_modifiers(&mut self, function: &'hir hir::Function<'hir>) {
        for modifier in function.modifiers {
            for argument in modifier.args.exprs() {
                self.analyze_expr(argument);
            }

            let Some(modifier_id) = modifier.id.as_function() else { continue };
            self.calls.insert(modifier_id);
            self.analyze_call(modifier_id, &modifier.args);
        }
    }

    fn analyze_call(&mut self, function_id: FunctionId, args: &hir::CallArgs<'hir>) {
        if self.stack.contains(&function_id) {
            return;
        }

        let function = self.hir.function(function_id);
        let saved_aliases = std::mem::take(&mut self.storage_aliases);
        for (parameter, argument) in function.parameters.iter().copied().zip(args.exprs()) {
            if self.hir.variable(parameter).data_location == Some(DataLocation::Storage)
                && let Some(root) =
                    state_lhs_vars(self.hir, argument, &saved_aliases).into_iter().next()
            {
                self.storage_aliases.insert(parameter, root);
            }
        }

        self.analyze_function(function_id);
        self.storage_aliases = saved_aliases;
    }

    fn analyze_block(&mut self, block: hir::Block<'hir>) {
        for statement in block.stmts {
            self.analyze_stmt(statement);
        }
    }

    fn analyze_stmt(&mut self, statement: &'hir hir::Stmt<'hir>) {
        match statement.kind {
            StmtKind::DeclSingle(variable_id) => {
                let variable = self.hir.variable(variable_id);
                if let Some(initializer) = variable.initializer {
                    self.analyze_expr(initializer);
                    self.set_storage_alias(variable_id, initializer);
                }
            }
            StmtKind::DeclMulti(_, expression)
            | StmtKind::Emit(expression)
            | StmtKind::Revert(expression)
            | StmtKind::Expr(expression) => self.analyze_expr(expression),
            StmtKind::Return(Some(expression)) => self.analyze_expr(expression),
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => self.analyze_block(block),
            StmtKind::Loop(block, _) => {
                let aliases = self.storage_aliases.clone();
                self.analyze_block(block);
                self.storage_aliases = aliases;
            }
            StmtKind::If(condition, then_statement, else_statement) => {
                self.analyze_expr(condition);
                let aliases = self.storage_aliases.clone();
                self.analyze_stmt(then_statement);
                self.storage_aliases = aliases.clone();
                if let Some(else_statement) = else_statement {
                    self.analyze_stmt(else_statement);
                }
                self.storage_aliases = aliases;
            }
            StmtKind::Try(try_statement) => {
                self.analyze_expr(&try_statement.expr);
                let aliases = self.storage_aliases.clone();
                for clause in try_statement.clauses {
                    self.storage_aliases = aliases.clone();
                    self.analyze_block(clause.block);
                }
                self.storage_aliases = aliases;
            }
            StmtKind::Return(None)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Placeholder
            | StmtKind::AssemblyBlock(_)
            | StmtKind::Switch(_)
            | StmtKind::Err(_) => {}
        }
    }

    fn analyze_expr(&mut self, expression: &'hir hir::Expr<'hir>) {
        match &expression.peel_parens().kind {
            ExprKind::Assign(lhs, _, rhs) => {
                self.analyze_expr(rhs);
                self.record_write(lhs);
                if let Some(local) = lhs_local_var(self.hir, lhs) {
                    self.set_storage_alias(local, rhs);
                } else {
                    self.analyze_lhs(lhs);
                }
            }
            ExprKind::Delete(inner) => {
                self.record_write(inner);
                self.analyze_lhs(inner);
            }
            ExprKind::Unary(operator, inner) => {
                if operator.kind.has_side_effects() {
                    self.record_write(inner);
                }
                self.analyze_expr(inner);
            }
            ExprKind::Call(callee, args, options) => {
                if let ExprKind::Member(base, member) = &callee.peel_parens().kind
                    && matches!(member.as_str(), "push" | "pop")
                    && is_dynamic_array_or_bytes(self.gcx, base)
                {
                    self.record_write(base);
                }

                self.analyze_expr(callee);
                if let Some(options) = options {
                    for option in options.args {
                        self.analyze_expr(&option.value);
                    }
                }
                for argument in args.exprs() {
                    self.analyze_expr(argument);
                }

                for function_id in
                    resolved_internal_functions(self.hir, callee, args.len(), self.bases)
                {
                    self.calls.insert(function_id);
                    self.analyze_call(function_id, args);
                }
            }
            ExprKind::Binary(lhs, _, rhs) => {
                self.analyze_expr(lhs);
                self.analyze_expr(rhs);
            }
            ExprKind::Index(base, index) => {
                self.analyze_expr(base);
                if let Some(index) = index {
                    self.analyze_expr(index);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_expr(base);
                if let Some(start) = start {
                    self.analyze_expr(start);
                }
                if let Some(end) = end {
                    self.analyze_expr(end);
                }
            }
            ExprKind::Member(base, _) | ExprKind::Payable(base) => self.analyze_expr(base),
            ExprKind::Ternary(condition, if_true, if_false) => {
                self.analyze_expr(condition);
                self.analyze_expr(if_true);
                self.analyze_expr(if_false);
            }
            ExprKind::Array(expressions) => {
                for expression in *expressions {
                    self.analyze_expr(expression);
                }
            }
            ExprKind::Tuple(expressions) => {
                for expression in expressions.iter().copied().flatten() {
                    self.analyze_expr(expression);
                }
            }
            ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
            ExprKind::Ident(_) | ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => {}
        }
    }

    fn analyze_lhs(&mut self, expression: &'hir hir::Expr<'hir>) {
        match &expression.peel_parens().kind {
            ExprKind::Index(base, index) => {
                self.analyze_lhs(base);
                if let Some(index) = index {
                    self.analyze_expr(index);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_lhs(base);
                if let Some(start) = start {
                    self.analyze_expr(start);
                }
                if let Some(end) = end {
                    self.analyze_expr(end);
                }
            }
            ExprKind::Member(base, _) | ExprKind::Payable(base) => self.analyze_lhs(base),
            ExprKind::Tuple(expressions) => {
                for expression in expressions.iter().copied().flatten() {
                    self.analyze_lhs(expression);
                }
            }
            _ => {}
        }
    }

    fn record_write(&mut self, expression: &hir::Expr<'_>) {
        self.writes.extend(state_lhs_vars(self.hir, expression, &self.storage_aliases));
    }

    fn set_storage_alias(&mut self, variable_id: VariableId, initializer: &'hir hir::Expr<'hir>) {
        let variable = self.hir.variable(variable_id);
        if variable.kind.is_state() || variable.data_location != Some(DataLocation::Storage) {
            self.storage_aliases.remove(&variable_id);
            return;
        }

        if let Some(root) =
            state_lhs_vars(self.hir, initializer, &self.storage_aliases).into_iter().next()
        {
            self.storage_aliases.insert(variable_id, root);
        } else {
            self.storage_aliases.remove(&variable_id);
        }
    }
}

fn lhs_local_var(hir: &hir::Hir<'_>, expression: &hir::Expr<'_>) -> Option<VariableId> {
    let ExprKind::Ident(resolutions) = &expression.peel_parens().kind else { return None };
    resolutions.iter().find_map(|resolution| match resolution {
        Res::Item(ItemId::Variable(variable_id)) if !hir.variable(*variable_id).kind.is_state() => {
            Some(*variable_id)
        }
        _ => None,
    })
}

fn state_lhs_vars(
    hir: &hir::Hir<'_>,
    expression: &hir::Expr<'_>,
    storage_aliases: &HashMap<VariableId, VariableId>,
) -> Vec<VariableId> {
    let mut variables = Vec::new();
    collect_state_lhs_vars(hir, expression, storage_aliases, &mut variables);
    variables
}

fn collect_state_lhs_vars(
    hir: &hir::Hir<'_>,
    expression: &hir::Expr<'_>,
    storage_aliases: &HashMap<VariableId, VariableId>,
    variables: &mut Vec<VariableId>,
) {
    match &expression.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            for resolution in *resolutions {
                let Res::Item(ItemId::Variable(variable_id)) = resolution else { continue };
                let root = if hir.variable(*variable_id).kind.is_state() {
                    Some(*variable_id)
                } else {
                    storage_aliases.get(variable_id).copied()
                };
                if let Some(root) = root
                    && !variables.contains(&root)
                {
                    variables.push(root);
                }
            }
        }
        ExprKind::Index(base, _) | ExprKind::Slice(base, ..) | ExprKind::Member(base, _) => {
            collect_state_lhs_vars(hir, base, storage_aliases, variables);
        }
        ExprKind::Payable(base) | ExprKind::Unary(_, base) | ExprKind::Delete(base) => {
            collect_state_lhs_vars(hir, base, storage_aliases, variables);
        }
        ExprKind::Tuple(expressions) => {
            for expression in expressions.iter().copied().flatten() {
                collect_state_lhs_vars(hir, expression, storage_aliases, variables);
            }
        }
        _ => {}
    }
}

fn is_dynamic_array_or_bytes(gcx: Gcx<'_>, expression: &hir::Expr<'_>) -> bool {
    gcx.type_of_expr(expression.peel_parens().id).is_some_and(|ty| {
        matches!(
            ty.peel_refs().kind,
            TyKind::DynArray(_) | TyKind::Array(..) | TyKind::Elementary(ElementaryType::Bytes)
        )
    })
}

fn resolved_internal_functions(
    hir: &hir::Hir<'_>,
    callee: &hir::Expr<'_>,
    argument_count: usize,
    bases: &[ContractId],
) -> Vec<FunctionId> {
    match &callee.peel_parens().kind {
        ExprKind::Ident(resolutions) => resolutions
            .iter()
            .filter_map(|resolution| match resolution {
                Res::Item(ItemId::Function(function_id))
                    if hir.function(*function_id).parameters.len() == argument_count =>
                {
                    Some(*function_id)
                }
                _ => None,
            })
            .collect(),
        ExprKind::Member(base, member) => {
            let ExprKind::Ident(resolutions) = &base.peel_parens().kind else { return Vec::new() };
            let is_super = resolutions.iter().any(
                |resolution| matches!(resolution, Res::Builtin(builtin) if builtin.name() == sym::super_),
            );
            let contracts: Vec<_> = if is_super {
                bases.get(1..).unwrap_or_default().to_vec()
            } else {
                resolutions
                    .iter()
                    .filter_map(|resolution| match resolution {
                        Res::Item(ItemId::Contract(contract_id)) => Some(*contract_id),
                        _ => None,
                    })
                    .collect()
            };

            contracts
                .into_iter()
                .flat_map(|contract_id| hir.contract(contract_id).functions())
                .filter(|&function_id| {
                    let function = hir.function(function_id);
                    function.kind == FunctionKind::Function
                        && function.parameters.len() == argument_count
                        && function.name.is_some_and(|name| name.as_str() == member.as_str())
                })
                .collect()
        }
        _ => Vec::new(),
    }
}
