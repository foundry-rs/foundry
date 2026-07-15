//! Slither-compatible protected-variable reachability analysis.
//!
//! Storage references are tracked as may-alias sets across internal calls and control-flow joins.
//! Calls are memoized by their storage/slot context so recursive alias propagation terminates.

use super::ProtectedVars;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::primitives::branch_always_exits},
};
use solar::{
    ast::{ContractKind, DataLocation, ElementaryType, FunctionKind, Visibility},
    interface::sym,
    sema::{
        Gcx,
        hir::{
            self, ContractId, ExprId, ExprKind, FunctionId, ItemId, NatSpecKind, Res, StmtKind,
            VariableId,
        },
        ty::{Ty, TyAbiPrinter, TyAbiPrinterMode, TyKind},
    },
};
use std::collections::{HashMap, HashSet};

type StorageRoots = HashSet<VariableId>;
type RootMap = HashMap<VariableId, StorageRoots>;

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

            let mut writes: Vec<_> = writes.into_iter().collect();
            writes.sort_unstable();
            for var_id in writes {
                let Some(requirements) = protected.get(&var_id) else { continue };
                for requirement in requirements {
                    let entry = hir.function(entry_id);
                    let span = entry.name.map_or(entry.keyword_span(), |name| name.span);
                    let contract_context = if entry.contract == Some(contract_id) {
                        String::new()
                    } else {
                        format!(" in most-derived contract `{}`", contract.name)
                    };
                    let variable = hir
                        .variable(var_id)
                        .name
                        .map_or_else(|| "<unnamed>".to_string(), |name| name.as_str().to_string());
                    match requirement {
                        ProtectionRequirement::Signature(signature) => {
                            if targets
                                .resolve(signature)
                                .is_some_and(|target_id| calls.contains(&target_id))
                            {
                                continue;
                            }
                            ctx.emit_with_msg(
                                &PROTECTED_VARS,
                                span,
                                format!(
                                    "protected variable `{variable}` is written without `{signature}`{contract_context}"
                                ),
                            );
                        }
                        ProtectionRequirement::Malformed => ctx.emit_with_msg(
                            &PROTECTED_VARS,
                            span,
                            format!(
                                "protected variable `{variable}` has a malformed write-protection annotation{contract_context}"
                            ),
                        ),
                    }
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
) -> HashMap<VariableId, Vec<ProtectionRequirement>> {
    let mut protected = HashMap::new();

    for &contract_id in bases {
        for var_id in hir.contract(contract_id).variables() {
            let var = hir.variable(var_id);
            if !var.kind.is_state() {
                continue;
            }

            let mut requirements = Vec::new();
            for item in gcx.natspec_doc_comments(var.doc) {
                let NatSpecKind::Custom { name } = item.kind else { continue };
                if name.as_str() != "security" {
                    continue;
                }
                let content = item.content();
                let requirement = if let Some(signature) = parse_write_protection(content) {
                    ProtectionRequirement::Signature(signature.to_owned())
                } else if has_write_protection_token(content) {
                    ProtectionRequirement::Malformed
                } else {
                    continue;
                };
                if !requirements.contains(&requirement) {
                    requirements.push(requirement);
                }
            }
            if !requirements.is_empty() {
                protected.insert(var_id, requirements);
            }
        }
    }

    protected
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProtectionRequirement {
    Signature(String),
    Malformed,
}

fn parse_write_protection(content: &str) -> Option<&str> {
    let index = write_protection_token(content)?;
    let value = content[index + "write-protection".len()..].strip_prefix("=\"")?;
    let (signature, _) = value.split_once('"')?;
    (!signature.is_empty()).then_some(signature)
}

fn has_write_protection_token(content: &str) -> bool {
    write_protection_token(content).is_some()
}

fn write_protection_token(content: &str) -> Option<usize> {
    content.match_indices("write-protection").find_map(|(index, token)| {
        let before = content[..index].chars().next_back();
        let after = content[index + token.len()..].chars().next();
        let is_token_character =
            |character: char| character.is_alphanumeric() || matches!(character, '_' | '-');
        (before.is_none_or(|character| !is_token_character(character))
            && after.is_none_or(|character| !is_token_character(character)))
        .then_some(index)
    })
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
                match function.kind {
                    FunctionKind::Function => {
                        let signature = callable_signature(gcx, hir, function_id);
                        this.functions.entry(signature).or_insert(function_id);
                    }
                    FunctionKind::Modifier => {
                        let signature = callable_signature(gcx, hir, function_id);
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

fn callable_signature(gcx: Gcx<'_>, hir: &hir::Hir<'_>, function_id: FunctionId) -> String {
    let function = hir.function(function_id);
    let mut signature = function.name.unwrap().as_str().to_owned();
    signature.push('(');
    for (index, &parameter) in function.parameters.iter().enumerate() {
        if index > 0 {
            signature.push(',');
        }
        let ty = gcx.type_of_item(parameter.into());
        if function.kind == FunctionKind::Modifier {
            signature.push_str(&source_type_signature(gcx, ty));
        } else {
            signature.push_str(&slither_function_parameter(gcx, ty, &mut HashSet::new()));
        }
    }
    signature.push(')');
    signature
}

/// Formats the source-level types used by Slither modifier signatures.
fn source_type_signature<'gcx>(gcx: Gcx<'gcx>, ty: Ty<'gcx>) -> String {
    ty.display(gcx)
        .to_string()
        .replace("contract ", "")
        .replace("struct ", "")
        .replace("enum ", "")
        .replace(" storage", "")
        .replace(" memory", "")
        .replace(" calldata", "")
        .replace(" external", "")
        .replace(" internal", "")
        .replace(" pure", "")
        .replace(" view", "")
        .replace(" payable", "")
        .replace("function ", "function")
        .replace("returns ", "returns")
}

/// Formats the Solidity-signature types used by Slither function lookup.
fn slither_function_parameter<'gcx>(
    gcx: Gcx<'gcx>,
    ty: Ty<'gcx>,
    seen_structs: &mut HashSet<hir::StructId>,
) -> String {
    match ty.kind {
        TyKind::Fn(_) | TyKind::Mapping(..) => source_type_signature(gcx, ty),
        TyKind::Ref(inner, _) => slither_function_parameter(gcx, inner, seen_structs),
        TyKind::DynArray(inner) => {
            format!("{}[]", slither_function_parameter(gcx, inner, seen_structs))
        }
        TyKind::Array(inner, length) => {
            format!("{}[{length}]", slither_function_parameter(gcx, inner, seen_structs))
        }
        TyKind::Struct(struct_id) => {
            if !seen_structs.insert(struct_id) {
                return source_type_signature(gcx, ty);
            }
            let fields = gcx
                .struct_field_types(struct_id)
                .iter()
                .map(|&field| slither_function_parameter(gcx, field, seen_structs))
                .collect::<Vec<_>>()
                .join(",");
            format!("({fields})")
        }
        _ => {
            let mut signature = String::new();
            TyAbiPrinter::new(gcx, &mut signature, TyAbiPrinterMode::Signature)
                .print(ty)
                .expect("writing to a String cannot fail");
            signature
        }
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

#[derive(Clone, Default, PartialEq, Eq)]
struct AliasState {
    storage: RootMap,
    slots: RootMap,
}

/// A finite call-graph key that distinguishes storage aliases without depending on values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CallContext {
    function_id: FunctionId,
    storage: Vec<(VariableId, Vec<VariableId>)>,
    slots: Vec<(VariableId, Vec<VariableId>)>,
}

#[derive(Clone, Default)]
struct LoopAliases {
    breaks: Option<AliasState>,
    continues: Option<AliasState>,
}

impl CallContext {
    fn new(function_id: FunctionId, function: &hir::Function<'_>, aliases: &AliasState) -> Self {
        let roots = |aliases: &RootMap| {
            function
                .parameters
                .iter()
                .filter_map(|&parameter| {
                    let mut roots: Vec<_> = aliases.get(&parameter)?.iter().copied().collect();
                    roots.sort_unstable();
                    Some((parameter, roots))
                })
                .collect()
        };
        Self { function_id, storage: roots(&aliases.storage), slots: roots(&aliases.slots) }
    }
}

struct EntryAnalyzer<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    bases: &'hir [ContractId],
    writes: HashSet<VariableId>,
    calls: HashSet<FunctionId>,
    aliases: AliasState,
    call_returns: HashMap<ExprId, Vec<StorageRoots>>,
    call_summaries: HashMap<CallContext, Vec<StorageRoots>>,
    seen_calls: HashSet<CallContext>,
    stack: Vec<FunctionId>,
    return_stack: Vec<Vec<StorageRoots>>,
    loop_aliases: Vec<LoopAliases>,
    assembly_depth: usize,
}

impl<'hir> EntryAnalyzer<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir hir::Hir<'hir>, bases: &'hir [ContractId]) -> Self {
        Self {
            gcx,
            hir,
            bases,
            writes: HashSet::new(),
            calls: HashSet::new(),
            aliases: AliasState::default(),
            call_returns: HashMap::new(),
            call_summaries: HashMap::new(),
            seen_calls: HashSet::new(),
            stack: Vec::new(),
            return_stack: Vec::new(),
            loop_aliases: Vec::new(),
            assembly_depth: 0,
        }
    }

    fn analyze(&mut self, entry_id: FunctionId) -> (HashSet<VariableId>, HashSet<FunctionId>) {
        let _ = self.analyze_function(entry_id);
        (std::mem::take(&mut self.writes), std::mem::take(&mut self.calls))
    }

    fn analyze_function(&mut self, function_id: FunctionId) -> Vec<StorageRoots> {
        let function = self.hir.function(function_id);
        let Some(body) = function.body else {
            return function.returns.iter().map(|_| StorageRoots::new()).collect();
        };
        self.stack.push(function_id);
        self.return_stack.push(function.returns.iter().map(|_| StorageRoots::new()).collect());
        self.analyze_modifiers(function);
        self.analyze_block(body);
        if !body.stmts.iter().any(branch_always_exits) {
            self.capture_named_returns();
        }
        let returns = self.return_stack.pop().expect("return frame must exist");
        self.stack.pop();
        returns
    }

    fn analyze_modifiers(&mut self, function: &'hir hir::Function<'hir>) {
        for modifier in function.modifiers {
            for argument in modifier.args.exprs() {
                self.analyze_expr(argument);
            }

            let Some(declared_id) = modifier.id.as_function() else { continue };
            let modifier_id = self.dispatch_function(declared_id);
            self.calls.insert(modifier_id);
            let arguments = self.ordered_call_arguments(declared_id, modifier.args, None);
            self.analyze_call(modifier_id, &arguments);
        }
    }

    fn analyze_call(
        &mut self,
        function_id: FunctionId,
        arguments: &[&'hir hir::Expr<'hir>],
    ) -> Vec<StorageRoots> {
        let function = self.hir.function(function_id);
        let saved_aliases = std::mem::take(&mut self.aliases);
        for (parameter, &argument) in function.parameters.iter().copied().zip(arguments) {
            if self.hir.variable(parameter).data_location == Some(DataLocation::Storage) {
                let roots =
                    state_lhs_vars(self.hir, argument, &saved_aliases.storage, &self.call_returns);
                if !roots.is_empty() {
                    self.aliases.storage.insert(parameter, roots);
                }
            }
            if function.is_yul {
                let roots = slot_roots(
                    self.hir,
                    argument,
                    &saved_aliases.storage,
                    &saved_aliases.slots,
                    &self.call_returns,
                );
                if !roots.is_empty() {
                    self.aliases.slots.insert(parameter, roots);
                }
            }
        }

        let context = CallContext::new(function_id, function, &self.aliases);
        if let Some(returns) = self.call_summaries.get(&context).cloned() {
            self.aliases = saved_aliases;
            return returns;
        }
        if !self.seen_calls.insert(context.clone()) {
            self.aliases = saved_aliases;
            return Vec::new();
        }

        let returns = self.analyze_function(function_id);
        self.call_summaries.insert(context, returns.clone());
        self.aliases = saved_aliases;
        returns
    }

    fn analyze_block(&mut self, block: hir::Block<'hir>) {
        for statement in block.stmts {
            self.analyze_stmt(statement);
            if branch_always_exits(statement) {
                break;
            }
        }
    }

    fn analyze_stmt(&mut self, statement: &'hir hir::Stmt<'hir>) {
        match statement.kind {
            StmtKind::DeclSingle(variable_id) => {
                let variable = self.hir.variable(variable_id);
                if let Some(initializer) = variable.initializer {
                    self.analyze_expr(initializer);
                    self.set_storage_alias(variable_id, initializer);
                    if self.assembly_depth > 0 {
                        self.set_slot_alias(variable_id, initializer);
                    }
                }
            }
            StmtKind::DeclMulti(variables, expression) => {
                self.analyze_expr(expression);
                self.set_decl_aliases(variables, expression);
            }
            StmtKind::Emit(expression)
            | StmtKind::Revert(expression)
            | StmtKind::Expr(expression) => self.analyze_expr(expression),
            StmtKind::Return(Some(expression)) => {
                self.analyze_expr(expression);
                self.set_return_aliases(expression);
            }
            StmtKind::Return(None) => self.capture_named_returns(),
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => self.analyze_block(block),
            StmtKind::AssemblyBlock(block) => {
                self.assembly_depth += 1;
                self.analyze_block(block);
                self.assembly_depth -= 1;
            }
            StmtKind::Loop(block, source) => {
                let before = self.aliases.clone();
                let flow = self.analyze_loop_iteration(block);
                let iteration = merge_optional_alias_state(&self.aliases, &flow.continues);
                let exits = if source == hir::LoopSource::DoWhile {
                    iteration
                } else {
                    merge_alias_states(&before, &iteration)
                };
                let mut input = merge_optional_alias_state(&exits, &flow.breaks);
                loop {
                    self.aliases = input.clone();
                    let flow = self.analyze_loop_iteration(block);
                    let iteration = merge_optional_alias_state(&self.aliases, &flow.continues);
                    let next = merge_optional_alias_state(
                        &merge_alias_states(&exits, &iteration),
                        &flow.breaks,
                    );
                    if next == input {
                        self.aliases = next;
                        break;
                    }
                    input = next;
                }
            }
            StmtKind::If(condition, then_statement, else_statement) => {
                self.analyze_expr(condition);
                let before = self.aliases.clone();
                self.analyze_stmt(then_statement);
                let then_aliases = self.aliases.clone();
                let then_exits = branch_always_exits(then_statement);
                self.aliases = before;
                let else_exits = if let Some(else_statement) = else_statement {
                    self.analyze_stmt(else_statement);
                    branch_always_exits(else_statement)
                } else {
                    false
                };
                let else_aliases = self.aliases.clone();
                self.aliases = match (then_exits, else_exits) {
                    (true, false) => else_aliases,
                    (false, true) => then_aliases,
                    _ => merge_alias_states(&then_aliases, &else_aliases),
                };
            }
            StmtKind::Try(try_statement) => {
                self.analyze_expr(&try_statement.expr);
                let before = self.aliases.clone();
                let mut merged = before.clone();
                for clause in try_statement.clauses {
                    self.aliases = before.clone();
                    self.analyze_block(clause.block);
                    merged = merge_alias_states(&merged, &self.aliases);
                }
                self.aliases = merged;
            }
            StmtKind::Switch(switch) => {
                self.analyze_expr(switch.selector);
                let before = self.aliases.clone();
                let mut merged = before.clone();
                for case in switch.cases {
                    self.aliases = before.clone();
                    self.analyze_block(case.body);
                    merged = merge_alias_states(&merged, &self.aliases);
                }
                self.aliases = merged;
            }
            StmtKind::Break => {
                if let Some(flow) = self.loop_aliases.last_mut() {
                    merge_alias_state_into(&mut flow.breaks, &self.aliases);
                }
            }
            StmtKind::Continue => {
                if let Some(flow) = self.loop_aliases.last_mut() {
                    merge_alias_state_into(&mut flow.continues, &self.aliases);
                }
            }
            StmtKind::Placeholder | StmtKind::Err(_) => {}
        }
    }

    fn analyze_loop_iteration(&mut self, block: hir::Block<'hir>) -> LoopAliases {
        self.loop_aliases.push(LoopAliases::default());
        self.analyze_block(block);
        self.loop_aliases.pop().expect("loop alias frame must exist")
    }

    fn analyze_expr(&mut self, expression: &'hir hir::Expr<'hir>) {
        match &expression.peel_parens().kind {
            ExprKind::Assign(lhs, operator, rhs) => {
                self.analyze_expr(rhs);
                self.analyze_lhs(lhs);
                self.apply_assignment(lhs, rhs, operator.is_some());
            }
            ExprKind::Delete(inner) => {
                self.analyze_lhs(inner);
                self.record_write(inner);
            }
            ExprKind::Unary(operator, inner) => {
                self.analyze_expr(inner);
                if operator.kind.has_side_effects() {
                    self.record_write(inner);
                }
            }
            ExprKind::Call(callee, args, options) => {
                self.analyze_expr(callee);
                if let Some(options) = options {
                    for option in options.args {
                        self.analyze_expr(&option.value);
                    }
                }
                for argument in args.exprs() {
                    self.analyze_expr(argument);
                }

                if let ExprKind::Member(base, member) = &callee.peel_parens().kind
                    && matches!(member.as_str(), "push" | "pop")
                    && is_dynamic_array_or_bytes(self.gcx, base)
                {
                    self.record_write(base);
                    if member.as_str() == "push" && args.is_empty() {
                        let roots = self.storage_roots(base);
                        self.store_call_returns(expression.id, vec![roots]);
                    }
                }

                if is_storage_write_builtin(callee)
                    && let Some(slot) = args.exprs().next()
                {
                    let roots = slot_roots(
                        self.hir,
                        slot,
                        &self.aliases.storage,
                        &self.aliases.slots,
                        &self.call_returns,
                    );
                    self.writes.extend(roots);
                }

                if let Some((declared_id, function_id, receiver)) =
                    self.resolved_internal_call(callee)
                {
                    self.calls.insert(function_id);
                    let arguments = self.ordered_call_arguments(declared_id, *args, receiver);
                    let returns = self.analyze_call(function_id, &arguments);
                    self.store_call_returns(expression.id, returns);
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
            ExprKind::Member(base, _) | ExprKind::YulMember(base, _) | ExprKind::Payable(base) => {
                self.analyze_expr(base)
            }
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
            ExprKind::Ident(_) | ExprKind::Lit(_) | ExprKind::Err(_) => {}
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
            ExprKind::Member(base, _) | ExprKind::YulMember(base, _) | ExprKind::Payable(base) => {
                self.analyze_lhs(base)
            }
            ExprKind::Tuple(expressions) => {
                for expression in expressions.iter().copied().flatten() {
                    self.analyze_lhs(expression);
                }
            }
            ExprKind::Call(..) => self.analyze_expr(expression),
            _ => {}
        }
    }

    fn record_write(&mut self, expression: &hir::Expr<'_>) {
        self.writes.extend(self.storage_roots(expression));
    }

    fn storage_roots(&self, expression: &hir::Expr<'_>) -> StorageRoots {
        state_lhs_vars(self.hir, expression, &self.aliases.storage, &self.call_returns)
    }

    fn set_storage_alias(&mut self, variable_id: VariableId, initializer: &'hir hir::Expr<'hir>) {
        let variable = self.hir.variable(variable_id);
        if variable.kind.is_state() || variable.data_location != Some(DataLocation::Storage) {
            self.aliases.storage.remove(&variable_id);
            return;
        }

        let roots = self.storage_roots(initializer);
        self.set_storage_alias_roots(variable_id, roots);
    }

    fn set_storage_alias_roots(&mut self, variable_id: VariableId, roots: StorageRoots) {
        let variable = self.hir.variable(variable_id);
        if !variable.kind.is_state()
            && variable.data_location == Some(DataLocation::Storage)
            && !roots.is_empty()
        {
            self.aliases.storage.insert(variable_id, roots);
        } else {
            self.aliases.storage.remove(&variable_id);
        }
    }

    fn set_slot_alias(&mut self, variable_id: VariableId, initializer: &'hir hir::Expr<'hir>) {
        let roots = slot_roots(
            self.hir,
            initializer,
            &self.aliases.storage,
            &self.aliases.slots,
            &self.call_returns,
        );
        if roots.is_empty() {
            self.aliases.slots.remove(&variable_id);
        } else {
            self.aliases.slots.insert(variable_id, roots);
        }
    }

    fn apply_assignment(
        &mut self,
        lhs: &'hir hir::Expr<'hir>,
        rhs: &'hir hir::Expr<'hir>,
        compound: bool,
    ) {
        if !compound && let ExprKind::Tuple(expressions) = &lhs.peel_parens().kind {
            let outputs = expressions.len();
            for (index, expression) in expressions.iter().copied().enumerate() {
                let Some(expression) = expression else { continue };
                if let Some(local) = lhs_local_var(self.hir, expression) {
                    let roots = self.storage_roots_for_output(rhs, index, outputs);
                    self.set_storage_alias_roots(local, roots);
                    if self.assembly_depth > 0 {
                        self.set_slot_alias(local, rhs);
                    }
                } else {
                    self.record_write(expression);
                }
            }
            return;
        }

        if !compound && let Some(local) = lhs_local_var(self.hir, lhs) {
            self.set_storage_alias(local, rhs);
            if self.assembly_depth > 0 {
                self.set_slot_alias(local, rhs);
            }
            return;
        }

        self.record_write(lhs);
    }

    fn set_decl_aliases(
        &mut self,
        variables: &[Option<VariableId>],
        expression: &'hir hir::Expr<'hir>,
    ) {
        for (index, variable_id) in variables.iter().copied().enumerate() {
            let Some(variable_id) = variable_id else { continue };
            let roots = self.storage_roots_for_output(expression, index, variables.len());
            self.set_storage_alias_roots(variable_id, roots);
            if self.assembly_depth > 0 {
                self.set_slot_alias(variable_id, expression);
            }
        }
    }

    fn set_return_aliases(&mut self, expression: &'hir hir::Expr<'hir>) {
        let Some(&function_id) = self.stack.last() else { return };
        let returns = self.hir.function(function_id).returns;
        let roots: Vec<_> = returns
            .iter()
            .enumerate()
            .map(|(index, _)| self.storage_roots_for_output(expression, index, returns.len()))
            .collect();
        let Some(frame) = self.return_stack.last_mut() else { return };
        for (returned, roots) in frame.iter_mut().zip(roots) {
            returned.extend(roots);
        }
    }

    fn capture_named_returns(&mut self) {
        let Some(&function_id) = self.stack.last() else { return };
        let function = self.hir.function(function_id);
        let aliases = if function.is_yul { &self.aliases.slots } else { &self.aliases.storage };
        let roots: Vec<_> = function
            .returns
            .iter()
            .map(|return_id| aliases.get(return_id).cloned().unwrap_or_default())
            .collect();
        let Some(frame) = self.return_stack.last_mut() else { return };
        for (returned, roots) in frame.iter_mut().zip(roots) {
            returned.extend(roots);
        }
    }

    fn storage_roots_for_output(
        &self,
        expression: &hir::Expr<'_>,
        index: usize,
        outputs: usize,
    ) -> StorageRoots {
        if let ExprKind::Tuple(expressions) = &expression.peel_parens().kind
            && outputs > 1
        {
            return expressions
                .get(index)
                .and_then(|expression| *expression)
                .map_or_else(StorageRoots::new, |expression| self.storage_roots(expression));
        }
        if let ExprKind::Call(..) = expression.peel_parens().kind {
            return self
                .call_returns
                .get(&expression.id)
                .and_then(|returns| returns.get(index))
                .cloned()
                .unwrap_or_default();
        }
        if outputs == 1 && index == 0 {
            self.storage_roots(expression)
        } else {
            StorageRoots::new()
        }
    }

    fn store_call_returns(&mut self, expression_id: ExprId, returns: Vec<StorageRoots>) {
        if returns.is_empty() {
            return;
        }
        let stored = self.call_returns.entry(expression_id).or_default();
        if stored.len() < returns.len() {
            stored.resize_with(returns.len(), StorageRoots::new);
        }
        for (stored, returned) in stored.iter_mut().zip(returns) {
            stored.extend(returned);
        }
    }

    fn ordered_call_arguments(
        &self,
        declared_id: FunctionId,
        arguments: hir::CallArgs<'hir>,
        receiver: Option<&'hir hir::Expr<'hir>>,
    ) -> Vec<&'hir hir::Expr<'hir>> {
        let function = self.hir.function(declared_id);
        let parameters = &function.parameters[usize::from(receiver.is_some())..];
        let mut ordered = Vec::with_capacity(arguments.len() + usize::from(receiver.is_some()));
        ordered.extend(receiver);
        match arguments.kind {
            hir::CallArgsKind::Unnamed(expressions) => ordered.extend(expressions),
            hir::CallArgsKind::Named(named) => {
                for &parameter in parameters {
                    let Some(parameter_name) = self.hir.variable(parameter).name else { continue };
                    if let Some(argument) = named.iter().find(|arg| arg.name == parameter_name) {
                        ordered.push(&argument.value);
                    }
                }
            }
        }
        ordered
    }

    fn resolved_internal_call(
        &self,
        callee: &'hir hir::Expr<'hir>,
    ) -> Option<(FunctionId, FunctionId, Option<&'hir hir::Expr<'hir>>)> {
        let resolved = self.gcx.resolved_callee(callee.id);
        let (function_id, attached) = if let Some(resolved) = resolved {
            let Res::Item(ItemId::Function(function_id)) = resolved.res else { return None };
            (function_id, resolved.attached)
        } else {
            let ExprKind::Ident(resolutions) = &callee.peel_parens().kind else { return None };
            let mut functions = resolutions.iter().filter_map(|resolution| match resolution {
                Res::Item(ItemId::Function(function_id)) => Some(*function_id),
                _ => None,
            });
            let function_id = functions.next()?;
            if functions.next().is_some() {
                return None;
            }
            (function_id, false)
        };

        match &callee.peel_parens().kind {
            ExprKind::Ident(_) => Some((function_id, self.dispatch_function(function_id), None)),
            ExprKind::Member(base, _) if attached => Some((function_id, function_id, Some(base))),
            ExprKind::Member(base, _)
                if self.is_library_function(function_id) || is_static_internal_base(base) =>
            {
                Some((function_id, function_id, None))
            }
            _ => None,
        }
    }

    fn is_library_function(&self, function_id: FunctionId) -> bool {
        self.hir
            .function(function_id)
            .contract
            .is_some_and(|contract_id| self.hir.contract(contract_id).kind.is_library())
    }

    fn dispatch_function(&self, function_id: FunctionId) -> FunctionId {
        let function = self.hir.function(function_id);
        if !function.virtual_ {
            return function_id;
        }

        let signature = callable_signature(self.gcx, self.hir, function_id);
        for &contract_id in self.bases {
            for candidate_id in self.hir.contract(contract_id).functions() {
                let candidate = self.hir.function(candidate_id);
                if candidate.kind == function.kind
                    && callable_signature(self.gcx, self.hir, candidate_id) == signature
                {
                    return candidate_id;
                }
            }
        }
        function_id
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
    storage_aliases: &RootMap,
    call_returns: &HashMap<ExprId, Vec<StorageRoots>>,
) -> StorageRoots {
    let mut variables = StorageRoots::new();
    collect_state_lhs_vars(hir, expression, storage_aliases, call_returns, &mut variables);
    variables
}

fn collect_state_lhs_vars(
    hir: &hir::Hir<'_>,
    expression: &hir::Expr<'_>,
    storage_aliases: &RootMap,
    call_returns: &HashMap<ExprId, Vec<StorageRoots>>,
    variables: &mut StorageRoots,
) {
    match &expression.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            for resolution in *resolutions {
                let Res::Item(ItemId::Variable(variable_id)) = resolution else { continue };
                if hir.variable(*variable_id).kind.is_state() {
                    variables.insert(*variable_id);
                } else if let Some(roots) = storage_aliases.get(variable_id) {
                    variables.extend(roots);
                }
            }
        }
        ExprKind::Index(base, _)
        | ExprKind::Slice(base, ..)
        | ExprKind::Member(base, _)
        | ExprKind::YulMember(base, _) => {
            collect_state_lhs_vars(hir, base, storage_aliases, call_returns, variables);
        }
        ExprKind::Payable(base) | ExprKind::Unary(_, base) | ExprKind::Delete(base) => {
            collect_state_lhs_vars(hir, base, storage_aliases, call_returns, variables);
        }
        ExprKind::Tuple(expressions) => {
            for expression in expressions.iter().copied().flatten() {
                collect_state_lhs_vars(hir, expression, storage_aliases, call_returns, variables);
            }
        }
        ExprKind::Ternary(_, if_true, if_false) => {
            collect_state_lhs_vars(hir, if_true, storage_aliases, call_returns, variables);
            collect_state_lhs_vars(hir, if_false, storage_aliases, call_returns, variables);
        }
        ExprKind::Call(..) => {
            if let Some(returns) = call_returns.get(&expression.id) {
                for roots in returns {
                    variables.extend(roots);
                }
            }
        }
        _ => {}
    }
}

fn slot_roots(
    hir: &hir::Hir<'_>,
    expression: &hir::Expr<'_>,
    storage_aliases: &RootMap,
    slot_aliases: &RootMap,
    call_returns: &HashMap<ExprId, Vec<StorageRoots>>,
) -> StorageRoots {
    let mut variables = StorageRoots::new();
    collect_slot_roots(
        hir,
        expression,
        storage_aliases,
        slot_aliases,
        call_returns,
        &mut variables,
    );
    variables
}

fn collect_slot_roots(
    hir: &hir::Hir<'_>,
    expression: &hir::Expr<'_>,
    storage_aliases: &RootMap,
    slot_aliases: &RootMap,
    call_returns: &HashMap<ExprId, Vec<StorageRoots>>,
    variables: &mut StorageRoots,
) {
    if let ExprKind::Call(..) = &expression.peel_parens().kind
        && let Some(returns) = call_returns.get(&expression.id)
    {
        for roots in returns {
            variables.extend(roots);
        }
    }
    let mut recurse = |expression| {
        collect_slot_roots(hir, expression, storage_aliases, slot_aliases, call_returns, variables)
    };
    match &expression.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            for resolution in *resolutions {
                let Res::Item(ItemId::Variable(variable_id)) = resolution else { continue };
                if let Some(roots) = slot_aliases.get(variable_id) {
                    variables.extend(roots);
                }
            }
        }
        ExprKind::YulMember(base, member) if member.as_str() == "slot" => {
            variables.extend(state_lhs_vars(hir, base, storage_aliases, call_returns));
        }
        ExprKind::Array(expressions) => {
            for expression in *expressions {
                recurse(expression);
            }
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            recurse(lhs);
            recurse(rhs);
        }
        ExprKind::Call(callee, args, options) => {
            recurse(callee);
            if let Some(options) = options {
                for option in options.args {
                    recurse(&option.value);
                }
            }
            for argument in args.exprs() {
                recurse(argument);
            }
        }
        ExprKind::Index(base, index) => {
            recurse(base);
            if let Some(index) = index {
                recurse(index);
            }
        }
        ExprKind::Slice(base, start, end) => {
            recurse(base);
            if let Some(start) = start {
                recurse(start);
            }
            if let Some(end) = end {
                recurse(end);
            }
        }
        ExprKind::Member(base, _)
        | ExprKind::YulMember(base, _)
        | ExprKind::Payable(base)
        | ExprKind::Unary(_, base)
        | ExprKind::Delete(base) => recurse(base),
        ExprKind::Ternary(condition, if_true, if_false) => {
            recurse(condition);
            recurse(if_true);
            recurse(if_false);
        }
        ExprKind::Tuple(expressions) => {
            for expression in expressions.iter().copied().flatten() {
                recurse(expression);
            }
        }
        ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Lit(_)
        | ExprKind::Err(_) => {}
    }
}

fn merge_alias_states(lhs: &AliasState, rhs: &AliasState) -> AliasState {
    AliasState {
        storage: merge_root_maps(&lhs.storage, &rhs.storage),
        slots: merge_root_maps(&lhs.slots, &rhs.slots),
    }
}

fn merge_optional_alias_state(state: &AliasState, other: &Option<AliasState>) -> AliasState {
    other.as_ref().map_or_else(|| state.clone(), |other| merge_alias_states(state, other))
}

fn merge_alias_state_into(destination: &mut Option<AliasState>, state: &AliasState) {
    *destination = Some(
        destination
            .as_ref()
            .map_or_else(|| state.clone(), |current| merge_alias_states(current, state)),
    );
}

fn merge_root_maps(lhs: &RootMap, rhs: &RootMap) -> RootMap {
    let mut merged = lhs.clone();
    for (&variable_id, roots) in rhs {
        merged.entry(variable_id).or_default().extend(roots);
    }
    merged
}

fn is_storage_write_builtin(callee: &hir::Expr<'_>) -> bool {
    let ExprKind::Ident(resolutions) = &callee.peel_parens().kind else { return false };
    resolutions.iter().any(|resolution| {
        matches!(
            resolution,
            Res::Builtin(builtin) if matches!(builtin.name().as_str(), "sstore" | "tstore")
        )
    })
}

fn is_static_internal_base(base: &hir::Expr<'_>) -> bool {
    let ExprKind::Ident(resolutions) = &base.peel_parens().kind else { return false };
    resolutions.iter().any(|resolution| {
        matches!(resolution, Res::Item(ItemId::Contract(_)) | Res::Namespace(_))
            || matches!(
                resolution,
                Res::Builtin(builtin) if builtin.name() == sym::super_
            )
    })
}

fn is_dynamic_array_or_bytes(gcx: Gcx<'_>, expression: &hir::Expr<'_>) -> bool {
    gcx.type_of_expr(expression.peel_parens().id).is_some_and(|ty| {
        matches!(
            ty.peel_refs().kind,
            TyKind::DynArray(_) | TyKind::Elementary(ElementaryType::Bytes)
        )
    })
}
