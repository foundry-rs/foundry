//! Slither-compatible protected-variable control-flow analysis.
//!
//! Storage references are tracked as may-alias sets across internal calls and control-flow joins.
//! Calls are memoized by their storage, slot, and guard context so recursive propagation
//! terminates.

use super::ProtectedVars;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::primitives::branch_always_exits},
};
use solar::{
    ast::{BinOpKind, ContractKind, DataLocation, ElementaryType, FunctionKind, Visibility},
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
            let writes = analyzer.analyze(entry_id);

            let mut writes: Vec<_> = writes.into_iter().collect();
            writes.sort_unstable_by_key(|(variable_id, _)| *variable_id);
            for (var_id, guards) in writes {
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
                            if targets.resolve(signature).is_some_and(|target| guards.contains(&target)) {
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

#[derive(Clone, Default, PartialEq, Eq)]
struct FlowState {
    aliases: AliasState,
    guards: HashSet<FunctionId>,
}

/// A finite call-graph key that distinguishes storage aliases without depending on values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CallContext {
    function_id: FunctionId,
    storage: Vec<(VariableId, Vec<VariableId>)>,
    slots: Vec<(VariableId, Vec<VariableId>)>,
    guards: Vec<FunctionId>,
}

#[derive(Clone, Default)]
struct LoopFlow {
    breaks: Option<FlowState>,
    continues: Option<FlowState>,
    completes: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct CallSummary {
    returns: Vec<StorageRoots>,
    guards: HashSet<FunctionId>,
    completes: bool,
}

struct FunctionSummary {
    returns: Vec<StorageRoots>,
    completes: bool,
}

#[derive(Clone, Copy)]
struct ModifierContinuation<'hir> {
    modifiers: &'hir [hir::Modifier<'hir>],
    next: usize,
    body: hir::Block<'hir>,
}

impl CallContext {
    fn new(
        function_id: FunctionId,
        function: &hir::Function<'_>,
        aliases: &AliasState,
        guards: &HashSet<FunctionId>,
    ) -> Self {
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
        let mut guards: Vec<_> = guards.iter().copied().collect();
        guards.sort_unstable();
        Self { function_id, storage: roots(&aliases.storage), slots: roots(&aliases.slots), guards }
    }
}

struct EntryAnalyzer<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    bases: &'hir [ContractId],
    writes: HashMap<VariableId, HashSet<FunctionId>>,
    aliases: AliasState,
    guards: HashSet<FunctionId>,
    call_returns: HashMap<ExprId, Vec<StorageRoots>>,
    call_summaries: HashMap<CallContext, CallSummary>,
    seen_calls: HashSet<CallContext>,
    evaluated_calls: HashSet<CallContext>,
    stack: Vec<FunctionId>,
    return_stack: Vec<Vec<StorageRoots>>,
    return_flow: Vec<Option<FlowState>>,
    loop_flow: Vec<LoopFlow>,
    modifier_continuations: Vec<ModifierContinuation<'hir>>,
    assembly_depth: usize,
}

impl<'hir> EntryAnalyzer<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir hir::Hir<'hir>, bases: &'hir [ContractId]) -> Self {
        Self {
            gcx,
            hir,
            bases,
            writes: HashMap::new(),
            aliases: AliasState::default(),
            guards: HashSet::new(),
            call_returns: HashMap::new(),
            call_summaries: HashMap::new(),
            seen_calls: HashSet::new(),
            evaluated_calls: HashSet::new(),
            stack: Vec::new(),
            return_stack: Vec::new(),
            return_flow: Vec::new(),
            loop_flow: Vec::new(),
            modifier_continuations: Vec::new(),
            assembly_depth: 0,
        }
    }

    fn analyze(&mut self, entry_id: FunctionId) -> HashMap<VariableId, HashSet<FunctionId>> {
        let mut previous_writes = HashMap::new();
        loop {
            self.reset_analysis_pass();
            let previous_summaries = self.call_summaries.clone();
            let _ = self.analyze_function(entry_id);
            if self.writes == previous_writes && self.call_summaries == previous_summaries {
                return std::mem::take(&mut self.writes);
            }
            previous_writes = self.writes.clone();
        }
    }

    fn reset_analysis_pass(&mut self) {
        self.writes.clear();
        self.aliases = AliasState::default();
        self.guards.clear();
        self.call_returns.clear();
        self.seen_calls.clear();
        self.evaluated_calls.clear();
        self.stack.clear();
        self.return_stack.clear();
        self.return_flow.clear();
        self.loop_flow.clear();
        self.modifier_continuations.clear();
        self.assembly_depth = 0;
    }

    fn analyze_function(&mut self, function_id: FunctionId) -> FunctionSummary {
        let function = self.hir.function(function_id);
        let Some(body) = function.body else {
            return FunctionSummary {
                returns: function.returns.iter().map(|_| StorageRoots::new()).collect(),
                completes: true,
            };
        };
        self.stack.push(function_id);
        self.return_stack.push(function.returns.iter().map(|_| StorageRoots::new()).collect());
        self.return_flow.push(None);
        let completes = self.analyze_modifier_chain(function.modifiers, 0, body);
        let falls_through = completes && !body.stmts.iter().any(branch_always_exits);
        if falls_through {
            self.capture_named_returns();
        }
        let completes = completes || self.return_flow.last().is_some_and(Option::is_some);
        let returns = self.return_stack.pop().expect("return frame must exist");
        self.return_flow.pop().expect("return flow frame must exist");
        self.stack.pop();
        FunctionSummary { returns, completes }
    }

    fn analyze_modifier_chain(
        &mut self,
        modifiers: &'hir [hir::Modifier<'hir>],
        index: usize,
        body: hir::Block<'hir>,
    ) -> bool {
        let Some(modifier) = modifiers.get(index) else {
            let previous_returns = self.return_flow.last_mut().and_then(Option::take);
            let falls_through = self.analyze_block(body);
            let body_returns = self.return_flow.last_mut().and_then(Option::take);

            // Returns from the function body resume in each enclosing modifier postlude. They
            // therefore become ordinary placeholder completions here and must not also escape
            // the whole modifier chain through `return_flow`: a reverting postlude can still
            // prevent the call from completing. Keep only returns captured in modifier prefixes
            // outside the body continuation.
            *self.return_flow.last_mut().expect("return flow frame must exist") = previous_returns;

            let mut completions = body_returns;
            if falls_through {
                merge_flow_state_into(&mut completions, &self.flow_state());
            }
            if let Some(state) = completions {
                self.set_flow_state(state);
                return true;
            }
            return false;
        };
        for argument in modifier.args.exprs() {
            if !self.analyze_expr(argument) {
                return false;
            }
        }

        let Some(declared_id) = modifier.id.as_function() else { return false };
        let modifier_id = self.dispatch_function(declared_id);
        self.guards.insert(modifier_id);
        let arguments = self.ordered_call_arguments(declared_id, modifier.args, None);
        let source_aliases = self.aliases.clone();
        self.bind_call_arguments(modifier_id, &arguments, &source_aliases);

        let Some(modifier_body) = self.hir.function(modifier_id).body else { return false };
        self.modifier_continuations.push(ModifierContinuation { modifiers, next: index + 1, body });
        let completes = self.analyze_block(modifier_body);
        self.modifier_continuations.pop();
        completes
    }

    fn analyze_call(
        &mut self,
        function_id: FunctionId,
        arguments: &[&'hir hir::Expr<'hir>],
    ) -> CallSummary {
        let function = self.hir.function(function_id);
        let saved_aliases = std::mem::take(&mut self.aliases);
        self.bind_call_arguments(function_id, arguments, &saved_aliases);

        let context = CallContext::new(function_id, function, &self.aliases, &self.guards);
        if self.seen_calls.contains(&context) {
            let summary = self.call_summaries.get(&context).cloned().unwrap_or_else(|| {
                CallSummary { returns: Vec::new(), guards: self.guards.clone(), completes: false }
            });
            self.aliases = saved_aliases;
            self.guards = summary.guards.clone();
            return summary;
        }
        if self.evaluated_calls.contains(&context)
            && let Some(summary) = self.call_summaries.get(&context).cloned()
        {
            self.aliases = saved_aliases;
            self.guards = summary.guards.clone();
            return summary;
        }

        self.seen_calls.insert(context.clone());
        self.evaluated_calls.insert(context.clone());
        let function_summary = self.analyze_function(function_id);
        let summary = CallSummary {
            returns: function_summary.returns,
            guards: self.guards.clone(),
            completes: function_summary.completes,
        };
        self.seen_calls.remove(&context);
        self.call_summaries.insert(context, summary.clone());
        self.aliases = saved_aliases;
        summary
    }

    fn bind_call_arguments(
        &mut self,
        function_id: FunctionId,
        arguments: &[&'hir hir::Expr<'hir>],
        source_aliases: &AliasState,
    ) {
        let function = self.hir.function(function_id);
        for (parameter, &argument) in function.parameters.iter().copied().zip(arguments) {
            if self.hir.variable(parameter).data_location == Some(DataLocation::Storage) {
                let roots =
                    state_lhs_vars(self.hir, argument, &source_aliases.storage, &self.call_returns);
                if roots.is_empty() {
                    self.aliases.storage.remove(&parameter);
                } else {
                    self.aliases.storage.insert(parameter, roots);
                }
            }
            if function.is_yul {
                let roots = slot_roots(
                    self.hir,
                    argument,
                    &source_aliases.storage,
                    &source_aliases.slots,
                    &self.call_returns,
                );
                if roots.is_empty() {
                    self.aliases.slots.remove(&parameter);
                } else {
                    self.aliases.slots.insert(parameter, roots);
                }
            }
        }
    }

    fn analyze_block(&mut self, block: hir::Block<'hir>) -> bool {
        for statement in block.stmts {
            if !self.analyze_stmt(statement) {
                return false;
            }
        }
        true
    }

    fn analyze_stmt(&mut self, statement: &'hir hir::Stmt<'hir>) -> bool {
        match statement.kind {
            StmtKind::DeclSingle(variable_id) => {
                let variable = self.hir.variable(variable_id);
                if let Some(initializer) = variable.initializer {
                    if !self.analyze_expr(initializer) {
                        return false;
                    }
                    self.set_storage_alias(variable_id, initializer);
                    if self.assembly_depth > 0 {
                        self.set_slot_alias(variable_id, initializer);
                    }
                }
                true
            }
            StmtKind::DeclMulti(variables, expression) => {
                if !self.analyze_expr(expression) {
                    return false;
                }
                self.set_decl_aliases(variables, expression);
                true
            }
            StmtKind::Emit(expression) | StmtKind::Expr(expression) => {
                self.analyze_expr(expression) && !branch_always_exits(statement)
            }
            StmtKind::Revert(expression) => {
                let _ = self.analyze_expr(expression);
                false
            }
            StmtKind::Return(Some(expression)) => {
                if self.analyze_expr(expression) {
                    self.set_return_aliases(expression);
                    self.capture_return_flow();
                }
                false
            }
            StmtKind::Return(None) => {
                self.capture_named_returns();
                self.capture_return_flow();
                false
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => self.analyze_block(block),
            StmtKind::AssemblyBlock(block) => {
                self.assembly_depth += 1;
                let continues = self.analyze_block(block);
                self.assembly_depth -= 1;
                continues
            }
            StmtKind::Loop(block, source) => {
                let mut head = self.flow_state();
                let mut exits = None;
                loop {
                    self.set_flow_state(head.clone());
                    let flow = self.analyze_loop_iteration(block);
                    let normal = flow.completes.then(|| self.flow_state());
                    let continue_flow = if source == hir::LoopSource::DoWhile {
                        flow.continues.as_ref().and_then(|continues| {
                            self.analyze_do_while_continue(block, continues.clone())
                        })
                    } else {
                        None
                    };
                    if let Some(breaks) = &flow.breaks {
                        merge_flow_state_into(&mut exits, breaks);
                    }
                    if let Some((continue_flow, _)) = &continue_flow
                        && let Some(breaks) = &continue_flow.breaks
                    {
                        merge_flow_state_into(&mut exits, breaks);
                    }

                    let mut backedges = None;
                    if let Some(normal) = &normal {
                        merge_flow_state_into(&mut backedges, normal);
                    }
                    if let Some((continue_flow, completion)) = &continue_flow {
                        if let Some(completion) = completion {
                            merge_flow_state_into(&mut backedges, completion);
                        }
                        if let Some(continues) = &continue_flow.continues {
                            merge_flow_state_into(&mut backedges, continues);
                        }
                    } else if let Some(continues) = &flow.continues {
                        merge_flow_state_into(&mut backedges, continues);
                    }
                    let Some(backedges) = backedges else { break };
                    let next = merge_flow_states(&head, &backedges);
                    if next == head {
                        break;
                    }
                    head = next;
                }
                if let Some(exits) = exits {
                    self.set_flow_state(exits);
                    true
                } else {
                    false
                }
            }
            StmtKind::If(condition, then_statement, else_statement) => {
                if !self.analyze_expr(condition) {
                    return false;
                }
                let before = self.flow_state();
                let then_continues = self.analyze_stmt(then_statement);
                let then_state = self.flow_state();
                self.set_flow_state(before);
                let else_continues = if let Some(else_statement) = else_statement {
                    self.analyze_stmt(else_statement)
                } else {
                    true
                };
                let else_state = self.flow_state();
                let merged = match (then_continues, else_continues) {
                    (false, true) => else_state,
                    (true, false) => then_state,
                    _ => merge_flow_states(&then_state, &else_state),
                };
                self.set_flow_state(merged);
                then_continues || else_continues
            }
            StmtKind::Try(try_statement) => {
                if !self.analyze_expr(&try_statement.expr) {
                    return false;
                }
                let before = self.flow_state();
                let mut merged = None;
                for clause in try_statement.clauses {
                    self.set_flow_state(before.clone());
                    if self.analyze_block(clause.block) {
                        merge_flow_state_into(&mut merged, &self.flow_state());
                    }
                }
                if let Some(merged) = merged {
                    self.set_flow_state(merged);
                    true
                } else {
                    false
                }
            }
            StmtKind::Switch(switch) => {
                if !self.analyze_expr(switch.selector) {
                    return false;
                }
                let before = self.flow_state();
                let has_default = switch.cases.last().is_some_and(|case| case.constant.is_none());
                let mut merged = (!has_default).then_some(before.clone());
                for case in switch.cases {
                    self.set_flow_state(before.clone());
                    if self.analyze_block(case.body) {
                        merge_flow_state_into(&mut merged, &self.flow_state());
                    }
                }
                if let Some(merged) = merged {
                    self.set_flow_state(merged);
                    true
                } else {
                    false
                }
            }
            StmtKind::Break => {
                let state = self.flow_state();
                if let Some(flow) = self.loop_flow.last_mut() {
                    merge_flow_state_into(&mut flow.breaks, &state);
                }
                false
            }
            StmtKind::Continue => {
                let state = self.flow_state();
                if let Some(flow) = self.loop_flow.last_mut() {
                    merge_flow_state_into(&mut flow.continues, &state);
                }
                false
            }
            StmtKind::Placeholder => {
                if let Some(continuation) = self.modifier_continuations.last().copied() {
                    self.analyze_modifier_chain(
                        continuation.modifiers,
                        continuation.next,
                        continuation.body,
                    )
                } else {
                    true
                }
            }
            StmtKind::Err(_) => true,
        }
    }

    fn analyze_loop_iteration(&mut self, block: hir::Block<'hir>) -> LoopFlow {
        self.loop_flow.push(LoopFlow::default());
        let completes = self.analyze_block(block);
        let mut flow = self.loop_flow.pop().expect("loop flow frame must exist");
        flow.completes = completes;
        flow
    }

    fn analyze_do_while_continue(
        &mut self,
        block: hir::Block<'hir>,
        state: FlowState,
    ) -> Option<(LoopFlow, Option<FlowState>)> {
        let epilogue = block.stmts.last().filter(|stmt| is_loop_termination_if(stmt))?;
        self.set_flow_state(state);
        self.loop_flow.push(LoopFlow::default());
        let completes = self.analyze_stmt(epilogue);
        let completion = completes.then(|| self.flow_state());
        let mut flow = self.loop_flow.pop().expect("loop flow frame must exist");
        flow.completes = completes;
        Some((flow, completion))
    }

    fn flow_state(&self) -> FlowState {
        FlowState { aliases: self.aliases.clone(), guards: self.guards.clone() }
    }

    fn set_flow_state(&mut self, state: FlowState) {
        self.aliases = state.aliases;
        self.guards = state.guards;
    }

    fn capture_return_flow(&mut self) {
        let state = self.flow_state();
        if let Some(exits) = self.return_flow.last_mut() {
            merge_flow_state_into(exits, &state);
        }
    }

    fn analyze_expr(&mut self, expression: &'hir hir::Expr<'hir>) -> bool {
        match &expression.peel_parens().kind {
            ExprKind::Assign(lhs, operator, rhs) => {
                if !self.analyze_expr(rhs) {
                    return false;
                }
                if !self.analyze_lhs(lhs) {
                    return false;
                }
                self.apply_assignment(lhs, rhs, operator.is_some());
                true
            }
            ExprKind::Delete(inner) => {
                if !self.analyze_lhs(inner) {
                    return false;
                }
                self.record_write(inner);
                true
            }
            ExprKind::Unary(operator, inner) => {
                if !self.analyze_expr(inner) {
                    return false;
                }
                if operator.kind.has_side_effects() {
                    self.record_write(inner);
                }
                true
            }
            ExprKind::Call(callee, args, options) => {
                if !self.analyze_expr(callee) {
                    return false;
                }
                if let Some(options) = options {
                    for option in options.args {
                        if !self.analyze_expr(&option.value) {
                            return false;
                        }
                    }
                }
                for argument in args.exprs() {
                    if !self.analyze_expr(argument) {
                        return false;
                    }
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

                if is_persistent_storage_write_builtin(callee)
                    && let Some(slot) = args.exprs().next()
                {
                    let roots = slot_roots(
                        self.hir,
                        slot,
                        &self.aliases.storage,
                        &self.aliases.slots,
                        &self.call_returns,
                    );
                    self.record_roots(roots);
                }

                if let Some((declared_id, function_id, receiver)) =
                    self.resolved_internal_call(callee)
                {
                    self.guards.insert(function_id);
                    let arguments = self.ordered_call_arguments(declared_id, *args, receiver);
                    let summary = self.analyze_call(function_id, &arguments);
                    self.store_call_returns(expression.id, summary.returns);
                    return summary.completes;
                }
                true
            }
            ExprKind::Binary(lhs, operator, rhs) => {
                if !self.analyze_expr(lhs) {
                    return false;
                }
                if matches!(operator.kind, BinOpKind::And | BinOpKind::Or) {
                    let short_circuit = self.flow_state();
                    if self.analyze_expr(rhs) {
                        let evaluated = self.flow_state();
                        self.set_flow_state(merge_flow_states(&short_circuit, &evaluated));
                    } else {
                        self.set_flow_state(short_circuit);
                    }
                    true
                } else {
                    self.analyze_expr(rhs)
                }
            }
            ExprKind::Index(base, index) => {
                if !self.analyze_expr(base) {
                    return false;
                }
                if let Some(index) = index { self.analyze_expr(index) } else { true }
            }
            ExprKind::Slice(base, start, end) => {
                if !self.analyze_expr(base) {
                    return false;
                }
                if let Some(start) = start
                    && !self.analyze_expr(start)
                {
                    return false;
                }
                if let Some(end) = end { self.analyze_expr(end) } else { true }
            }
            ExprKind::Member(base, _) | ExprKind::YulMember(base, _) | ExprKind::Payable(base) => {
                self.analyze_expr(base)
            }
            ExprKind::Ternary(condition, if_true, if_false) => {
                if !self.analyze_expr(condition) {
                    return false;
                }
                let before = self.flow_state();
                let true_completes = self.analyze_expr(if_true);
                let true_state = self.flow_state();
                self.set_flow_state(before);
                let false_completes = self.analyze_expr(if_false);
                let false_state = self.flow_state();
                match (true_completes, false_completes) {
                    (true, true) => {
                        self.set_flow_state(merge_flow_states(&true_state, &false_state))
                    }
                    (true, false) => self.set_flow_state(true_state),
                    (false, true) => self.set_flow_state(false_state),
                    (false, false) => {}
                }
                true_completes || false_completes
            }
            ExprKind::Array(expressions) => {
                for expression in *expressions {
                    if !self.analyze_expr(expression) {
                        return false;
                    }
                }
                true
            }
            ExprKind::Tuple(expressions) => {
                for expression in expressions.iter().copied().flatten() {
                    if !self.analyze_expr(expression) {
                        return false;
                    }
                }
                true
            }
            ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => true,
            ExprKind::Ident(_) | ExprKind::Lit(_) | ExprKind::Err(_) => true,
        }
    }

    fn analyze_lhs(&mut self, expression: &'hir hir::Expr<'hir>) -> bool {
        match &expression.peel_parens().kind {
            ExprKind::Index(base, index) => {
                if !self.analyze_lhs(base) {
                    return false;
                }
                if let Some(index) = index { self.analyze_expr(index) } else { true }
            }
            ExprKind::Slice(base, start, end) => {
                if !self.analyze_lhs(base) {
                    return false;
                }
                if let Some(start) = start
                    && !self.analyze_expr(start)
                {
                    return false;
                }
                if let Some(end) = end { self.analyze_expr(end) } else { true }
            }
            ExprKind::Member(base, _) | ExprKind::YulMember(base, _) | ExprKind::Payable(base) => {
                self.analyze_lhs(base)
            }
            ExprKind::Tuple(expressions) => {
                for expression in expressions.iter().copied().flatten() {
                    if !self.analyze_lhs(expression) {
                        return false;
                    }
                }
                true
            }
            ExprKind::Call(..) => self.analyze_expr(expression),
            _ => true,
        }
    }

    fn record_write(&mut self, expression: &hir::Expr<'_>) {
        self.record_roots(self.storage_roots(expression));
    }

    fn record_roots(&mut self, roots: StorageRoots) {
        for variable_id in roots {
            self.writes
                .entry(variable_id)
                .and_modify(|guards| guards.retain(|guard| self.guards.contains(guard)))
                .or_insert_with(|| self.guards.clone());
        }
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
        self.set_slot_alias_roots(variable_id, roots);
    }

    fn set_slot_alias_roots(&mut self, variable_id: VariableId, roots: StorageRoots) {
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
        if !compound
            && let ExprKind::YulMember(base, member) = &lhs.peel_parens().kind
            && member.as_str() == "slot"
            && let Some(local) = lhs_local_var(self.hir, base)
        {
            let roots = slot_roots(
                self.hir,
                rhs,
                &self.aliases.storage,
                &self.aliases.slots,
                &self.call_returns,
            );
            self.set_storage_alias_roots(local, roots);
            return;
        }

        if !compound && let ExprKind::Tuple(expressions) = &lhs.peel_parens().kind {
            let outputs = expressions.len();
            for (index, expression) in expressions.iter().copied().enumerate() {
                let Some(expression) = expression else { continue };
                if let Some(local) = lhs_local_var(self.hir, expression) {
                    let roots = self.storage_roots_for_output(rhs, index, outputs);
                    if self.assembly_depth > 0 {
                        self.set_slot_alias_roots(local, roots.clone());
                    }
                    self.set_storage_alias_roots(local, roots);
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
            if self.assembly_depth > 0 {
                self.set_slot_alias_roots(variable_id, roots.clone());
            }
            self.set_storage_alias_roots(variable_id, roots);
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
        return;
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

fn merge_flow_states(lhs: &FlowState, rhs: &FlowState) -> FlowState {
    FlowState {
        aliases: merge_alias_states(&lhs.aliases, &rhs.aliases),
        guards: lhs.guards.intersection(&rhs.guards).copied().collect(),
    }
}

fn merge_flow_state_into(destination: &mut Option<FlowState>, state: &FlowState) {
    *destination = Some(
        destination
            .as_ref()
            .map_or_else(|| state.clone(), |current| merge_flow_states(current, state)),
    );
}

fn is_loop_termination_if(statement: &hir::Stmt<'_>) -> bool {
    let StmtKind::If(_, then_statement, else_statement) = &statement.kind else { return false };
    is_break_stmt(then_statement)
        || else_statement.as_ref().is_some_and(|statement| is_break_stmt(statement))
}

fn is_break_stmt(statement: &hir::Stmt<'_>) -> bool {
    match &statement.kind {
        StmtKind::Break => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.len() == 1 && is_break_stmt(&block.stmts[0])
        }
        _ => false,
    }
}

fn merge_root_maps(lhs: &RootMap, rhs: &RootMap) -> RootMap {
    let mut merged = lhs.clone();
    for (&variable_id, roots) in rhs {
        merged.entry(variable_id).or_default().extend(roots);
    }
    merged
}

fn is_persistent_storage_write_builtin(callee: &hir::Expr<'_>) -> bool {
    let ExprKind::Ident(resolutions) = &callee.peel_parens().kind else { return false };
    resolutions.iter().any(|resolution| {
        matches!(resolution, Res::Builtin(builtin) if builtin.name().as_str() == "sstore")
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
