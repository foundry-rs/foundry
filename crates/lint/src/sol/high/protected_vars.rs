//! Slither-compatible protected-variable control-flow analysis.
//!
//! Storage references are tracked as may-alias sets across internal calls and control-flow joins.
//! Calls are memoized by their storage, slot, and guard context so recursive propagation
//! terminates.

use super::ProtectedVars;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            branch_always_exits, builtins, function_ids, is_builtin, is_loop_termination_if,
            lhs_local_var, loop_update, runtime_entry_points, unique,
        },
    },
};
use solar::{
    ast::{BinOpKind, ContractKind, DataLocation, ElementaryType, FunctionKind},
    interface::sym,
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, CallArgs, ContractId, ExprId, ExprKind, FunctionId, ItemId, LoopSource,
            NatSpecKind, Res, StmtKind, VariableId,
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

impl<'gcx> LateLintPass<'gcx> for ProtectedVars {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract_id: ContractId,
    ) {
        let contract = gcx.hir.contract(contract_id);
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract)
            || contract.linearization_failed()
            || !is_most_derived_contract(&gcx.hir, contract_id)
        {
            return;
        }
        let bases = contract.linearized_bases;

        let protected = protected_variables(gcx, bases);
        if protected.is_empty() {
            return;
        }
        let targets = protection_targets(gcx, bases);

        // The effective runtime dispatch surface: most-derived overrides plus the inherited
        // fallback/receive functions.
        let entries = runtime_entry_points(gcx, contract_id);

        for entry_id in entries {
            let entry = gcx.hir.function(entry_id);
            let span = entry.name.map_or(entry.keyword_span(), |name| name.span);
            let context = if entry.contract == Some(contract_id) {
                String::new()
            } else {
                format!(" in most-derived contract `{}`", contract.name)
            };
            let mut writes: Vec<_> = analyze_entry(gcx, bases, entry_id).into_iter().collect();
            writes.sort_unstable_by_key(|(var_id, _)| *var_id);
            for (var_id, guards) in writes {
                let Some(requirements) = protected.get(&var_id) else { continue };
                let variable = gcx
                    .hir
                    .variable(var_id)
                    .name
                    .map_or("<unnamed>".to_string(), |n| n.to_string());
                for requirement in requirements {
                    let msg = match requirement {
                        Some(signature) => {
                            if targets.get(signature).is_some_and(|target| guards.contains(target))
                            {
                                continue;
                            }
                            format!(
                                "protected variable `{variable}` is written without `{signature}`{context}"
                            )
                        }
                        None => format!(
                            "protected variable `{variable}` has a malformed write-protection annotation{context}"
                        ),
                    };
                    ctx.emit_with_msg(&PROTECTED_VARS, span, msg);
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
            && hir.contract(candidate_id).linearized_bases[1..].contains(&contract_id)
    })
}

/// Protected state variables with their `@custom:security write-protection="<sig>"` requirements;
/// `None` marks a malformed annotation.
fn protected_variables(
    gcx: Gcx<'_>,
    bases: &[ContractId],
) -> HashMap<VariableId, Vec<Option<String>>> {
    let mut protected = HashMap::new();
    for var_id in bases.iter().flat_map(|&cid| gcx.hir.contract(cid).variables()) {
        let var = gcx.hir.variable(var_id);
        if !var.kind.is_state() {
            continue;
        }
        let mut requirements = Vec::new();
        for item in gcx.natspec_doc_comments(var.doc) {
            let NatSpecKind::Custom { name } = item.kind else { continue };
            let content = item.content();
            let Some(index) = write_protection_token(content) else { continue };
            if name.as_str() != "security" {
                continue;
            }
            let requirement = content[index + "write-protection".len()..]
                .strip_prefix("=\"")
                .and_then(|value| value.split_once('"'))
                .map(|(signature, _)| signature)
                .filter(|signature| !signature.is_empty())
                .map(str::to_owned);
            if !requirements.contains(&requirement) {
                requirements.push(requirement);
            }
        }
        if !requirements.is_empty() {
            protected.insert(var_id, requirements);
        }
    }
    protected
}

/// Byte offset of a standalone `write-protection` token in `content`.
fn write_protection_token(content: &str) -> Option<usize> {
    let is_token_char = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-');
    content.match_indices("write-protection").find_map(|(index, token)| {
        let before = content[..index].chars().next_back();
        let after = content[index + token.len()..].chars().next();
        (!before.is_some_and(is_token_char) && !after.is_some_and(is_token_char)).then_some(index)
    })
}

/// Guard functions and modifiers by Slither signature. Functions take precedence over modifiers;
/// within a kind, linearization order keeps the most-derived declaration and drops shadowed ones.
fn protection_targets(gcx: Gcx<'_>, bases: &[ContractId]) -> HashMap<String, FunctionId> {
    let mut targets = HashMap::new();
    for kind in [FunctionKind::Function, FunctionKind::Modifier] {
        for fid in bases.iter().flat_map(|&cid| gcx.hir.contract(cid).functions()) {
            let function = gcx.hir.function(fid);
            if function.kind == kind && function.name.is_some() {
                targets.entry(callable_signature(gcx, fid)).or_insert(fid);
            }
        }
    }
    targets
}

fn callable_signature(gcx: Gcx<'_>, function_id: FunctionId) -> String {
    let function = gcx.hir.function(function_id);
    let params = function.parameters.iter().map(|&parameter| {
        let ty = gcx.type_of_item(parameter.into());
        if function.kind == FunctionKind::Modifier {
            source_type_signature(gcx, ty)
        } else {
            slither_function_parameter(gcx, ty, &mut HashSet::new())
        }
    });
    format!("{}({})", function.name.unwrap().as_str(), params.collect::<Vec<_>>().join(","))
}

/// Formats the source-level types used by Slither modifier signatures.
fn source_type_signature<'gcx>(gcx: Gcx<'gcx>, ty: Ty<'gcx>) -> String {
    let mut signature = ty.display(gcx).to_string();
    for prefix in ["contract ", "struct ", "enum "] {
        signature = signature.replace(prefix, "");
    }
    for suffix in
        [" storage", " memory", " calldata", " external", " internal", " pure", " view", " payable"]
    {
        signature = signature.replace(suffix, "");
    }
    signature.replace("function ", "function").replace("returns ", "returns")
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
                .collect::<Vec<_>>();
            format!("({})", fields.join(","))
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

#[derive(Clone, Default, PartialEq, Eq)]
struct AliasState {
    /// Storage pointer locals to the state variables they may alias.
    storage: RootMap,
    /// Yul locals holding a `.slot` to the state variables they may denote.
    slots: RootMap,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct FlowState {
    aliases: AliasState,
    /// Guards that have run on every path reaching this point.
    guards: HashSet<FunctionId>,
}

impl FlowState {
    fn merge(&self, other: &Self) -> Self {
        fn merge_roots(lhs: &RootMap, rhs: &RootMap) -> RootMap {
            let mut merged = lhs.clone();
            for (&var_id, roots) in rhs {
                merged.entry(var_id).or_default().extend(roots);
            }
            merged
        }
        Self {
            aliases: AliasState {
                storage: merge_roots(&self.aliases.storage, &other.aliases.storage),
                slots: merge_roots(&self.aliases.slots, &other.aliases.slots),
            },
            guards: self.guards.intersection(&other.guards).copied().collect(),
        }
    }
}

/// Joins `state` into the accumulated state of the paths that reach a point.
fn join(destination: &mut Option<FlowState>, state: FlowState) {
    *destination = Some(match destination.take() {
        Some(current) => current.merge(&state),
        None => state,
    });
}

/// A finite call-graph key that distinguishes storage aliases without depending on values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CallContext {
    function_id: FunctionId,
    storage: Vec<(VariableId, Vec<VariableId>)>,
    slots: Vec<(VariableId, Vec<VariableId>)>,
    guards: Vec<FunctionId>,
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

#[derive(Clone, PartialEq, Eq)]
struct CallSummary {
    /// Storage roots that may be returned in each return slot.
    returns: Vec<StorageRoots>,
    /// Guards that hold after the call.
    guards: HashSet<FunctionId>,
    /// Whether the call can complete normally.
    completes: bool,
}

/// States collected at `break`/`continue` statements of the innermost loop.
#[derive(Default)]
struct LoopFlow {
    breaks: Option<FlowState>,
    continues: Option<FlowState>,
}

/// What `_` resumes: the rest of the modifier chain and the function body.
#[derive(Clone, Copy)]
struct ModifierContinuation<'gcx> {
    modifiers: &'gcx [hir::Modifier<'gcx>],
    next: usize,
    body: hir::Block<'gcx>,
}

/// Runs the entry to a fixpoint over the memoized call summaries and returns, per written state
/// variable, the guards that held on every path to some write.
fn analyze_entry<'gcx>(
    gcx: Gcx<'gcx>,
    bases: &'gcx [ContractId],
    entry_id: FunctionId,
) -> HashMap<VariableId, HashSet<FunctionId>> {
    let mut call_summaries = HashMap::new();
    let mut previous_writes = HashMap::new();
    loop {
        let mut analyzer = EntryAnalyzer {
            gcx,
            bases,
            writes: HashMap::new(),
            aliases: AliasState::default(),
            guards: HashSet::new(),
            call_returns: HashMap::new(),
            call_summaries: call_summaries.clone(),
            seen_calls: HashSet::new(),
            evaluated_calls: HashSet::new(),
            stack: Vec::new(),
            return_stack: Vec::new(),
            return_flow: Vec::new(),
            loop_flow: Vec::new(),
            modifier_continuations: Vec::new(),
            assembly_depth: 0,
        };
        analyzer.analyze_function(entry_id);
        if analyzer.writes == previous_writes && analyzer.call_summaries == call_summaries {
            return analyzer.writes;
        }
        previous_writes = analyzer.writes;
        call_summaries = analyzer.call_summaries;
    }
}

struct EntryAnalyzer<'gcx> {
    gcx: Gcx<'gcx>,
    bases: &'gcx [ContractId],
    /// Written state variables to the guards that held at every write.
    writes: HashMap<VariableId, HashSet<FunctionId>>,
    aliases: AliasState,
    guards: HashSet<FunctionId>,
    /// Storage roots returned by each analyzed call expression.
    call_returns: HashMap<ExprId, Vec<StorageRoots>>,
    call_summaries: HashMap<CallContext, CallSummary>,
    /// Contexts currently being analyzed (recursion detection).
    seen_calls: HashSet<CallContext>,
    /// Contexts already analyzed in this pass.
    evaluated_calls: HashSet<CallContext>,
    stack: Vec<FunctionId>,
    /// Per active function, the storage roots flowing into each return slot.
    return_stack: Vec<Vec<StorageRoots>>,
    /// Per active function, the joined state at its `return` statements.
    return_flow: Vec<Option<FlowState>>,
    loop_flow: Vec<LoopFlow>,
    modifier_continuations: Vec<ModifierContinuation<'gcx>>,
    assembly_depth: usize,
}

impl<'gcx> EntryAnalyzer<'gcx> {
    fn analyze_function(&mut self, function_id: FunctionId) -> CallSummary {
        let function = self.gcx.hir.function(function_id);
        let empty_returns = || function.returns.iter().map(|_| StorageRoots::new()).collect();
        let Some(body) = function.body else {
            return CallSummary {
                returns: empty_returns(),
                guards: self.guards.clone(),
                completes: true,
            };
        };
        self.stack.push(function_id);
        self.return_stack.push(empty_returns());
        self.return_flow.push(None);
        let completes = self.analyze_modifier_chain(function.modifiers, 0, body);
        if completes && !body.stmts.iter().any(branch_always_exits) {
            self.capture_named_returns();
        }
        let returned = self.return_flow.pop().expect("return flow frame").is_some();
        let returns = self.return_stack.pop().expect("return frame");
        self.stack.pop();
        CallSummary { returns, guards: self.guards.clone(), completes: completes || returned }
    }

    /// Analyzes the modifier at `index` (or the body once the chain is exhausted). Returns whether
    /// control can complete normally.
    fn analyze_modifier_chain(
        &mut self,
        modifiers: &'gcx [hir::Modifier<'gcx>],
        index: usize,
        body: hir::Block<'gcx>,
    ) -> bool {
        let Some(modifier) = modifiers.get(index) else {
            let previous_returns = self.return_flow.last_mut().and_then(Option::take);
            let falls_through = self.analyze_block(body);
            let mut completions = self.return_flow.last_mut().and_then(Option::take);

            // Returns from the function body resume in each enclosing modifier postlude. They
            // therefore become ordinary placeholder completions here and must not also escape
            // the whole modifier chain through `return_flow`: a reverting postlude can still
            // prevent the call from completing. Keep only returns captured in modifier prefixes
            // outside the body continuation.
            *self.return_flow.last_mut().expect("return flow frame") = previous_returns;

            if falls_through {
                join(&mut completions, self.flow_state());
            }
            let Some(state) = completions else { return false };
            self.set_flow_state(state);
            return true;
        };
        if !modifier.args.exprs().all(|argument| self.analyze_expr(argument)) {
            return false;
        }

        let Some(declared_id) = modifier.id.as_function() else { return false };
        let modifier_id = self.dispatch_function(declared_id);
        self.guards.insert(modifier_id);
        let arguments = self.ordered_call_arguments(declared_id, modifier.args, None);
        let bound = self.argument_aliases(modifier_id, &arguments);
        for parameter in self.gcx.hir.function(modifier_id).parameters {
            self.aliases.storage.remove(parameter);
        }
        self.aliases.storage.extend(bound.storage);

        let Some(modifier_body) = self.gcx.hir.function(modifier_id).body else { return false };
        self.modifier_continuations.push(ModifierContinuation { modifiers, next: index + 1, body });
        let completes = self.analyze_block(modifier_body);
        self.modifier_continuations.pop();
        completes
    }

    fn analyze_call(
        &mut self,
        function_id: FunctionId,
        arguments: &[&'gcx hir::Expr<'gcx>],
    ) -> CallSummary {
        let bound = self.argument_aliases(function_id, arguments);
        let saved_aliases = std::mem::replace(&mut self.aliases, bound);

        let function = self.gcx.hir.function(function_id);
        let context = CallContext::new(function_id, function, &self.aliases, &self.guards);
        let cached = if self.seen_calls.contains(&context) {
            // Recursive call: assume the summary so far, or a non-completing call.
            Some(self.call_summaries.get(&context).cloned().unwrap_or_else(|| CallSummary {
                returns: Vec::new(),
                guards: self.guards.clone(),
                completes: false,
            }))
        } else if self.evaluated_calls.contains(&context) {
            self.call_summaries.get(&context).cloned()
        } else {
            None
        };
        let summary = match cached {
            Some(summary) => {
                self.guards = summary.guards.clone();
                summary
            }
            None => {
                self.seen_calls.insert(context.clone());
                self.evaluated_calls.insert(context.clone());
                let summary = self.analyze_function(function_id);
                self.seen_calls.remove(&context);
                self.call_summaries.insert(context, summary.clone());
                summary
            }
        };
        self.aliases = saved_aliases;
        summary
    }

    /// Aliases of `function_id`'s parameters when bound to `arguments` under the current state.
    fn argument_aliases(
        &self,
        function_id: FunctionId,
        arguments: &[&'gcx hir::Expr<'gcx>],
    ) -> AliasState {
        let function = self.gcx.hir.function(function_id);
        let mut bound = AliasState::default();
        for (&parameter, &argument) in function.parameters.iter().zip(arguments) {
            if self.gcx.hir.variable(parameter).data_location == Some(DataLocation::Storage) {
                let roots = self.storage_roots(argument);
                if !roots.is_empty() {
                    bound.storage.insert(parameter, roots);
                }
            }
            if function.is_yul {
                let roots = self.slot_roots(argument);
                if !roots.is_empty() {
                    bound.slots.insert(parameter, roots);
                }
            }
        }
        bound
    }

    fn analyze_block(&mut self, block: hir::Block<'gcx>) -> bool {
        block.stmts.iter().all(|statement| self.analyze_stmt(statement))
    }

    /// Analyzes each alternative from the current state and joins the states of those that
    /// complete (plus `merged`, the state of any implicit fall-through path). Returns whether any
    /// path completes.
    fn analyze_alternatives<T: Copy>(
        &mut self,
        alternatives: impl IntoIterator<Item = T>,
        mut merged: Option<FlowState>,
        analyze: impl Fn(&mut Self, T) -> bool,
    ) -> bool {
        let before = self.flow_state();
        for alternative in alternatives {
            self.set_flow_state(before.clone());
            if analyze(self, alternative) {
                join(&mut merged, self.flow_state());
            }
        }
        let Some(merged) = merged else { return false };
        self.set_flow_state(merged);
        true
    }

    /// Analyzes a statement, returning whether control can continue past it.
    fn analyze_stmt(&mut self, statement: &'gcx hir::Stmt<'gcx>) -> bool {
        match statement.kind {
            StmtKind::DeclSingle(variable_id) => {
                let Some(initializer) = self.gcx.hir.variable(variable_id).initializer else {
                    return true;
                };
                if !self.analyze_expr(initializer) {
                    return false;
                }
                self.alias_local_from_expr(variable_id, initializer);
                true
            }
            StmtKind::DeclMulti(variables, expression) => {
                if !self.analyze_expr(expression) {
                    return false;
                }
                for (index, variable_id) in variables.iter().enumerate() {
                    if let Some(variable_id) = variable_id {
                        let roots =
                            self.storage_roots_for_output(expression, index, variables.len());
                        self.alias_local(*variable_id, roots);
                    }
                }
                true
            }
            StmtKind::Emit(expression) | StmtKind::Expr(expression) => {
                self.analyze_expr(expression) && !branch_always_exits(statement)
            }
            StmtKind::Revert(expression) => {
                self.analyze_expr(expression);
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
            StmtKind::Loop(block, source) => self.analyze_loop(block, source),
            StmtKind::If(condition, then_statement, else_statement) => {
                self.analyze_expr(condition)
                    && self.analyze_alternatives(
                        [Some(then_statement), else_statement],
                        None,
                        |this, arm| arm.is_none_or(|arm| this.analyze_stmt(arm)),
                    )
            }
            StmtKind::Try(try_statement) => {
                self.analyze_expr(&try_statement.expr)
                    && self.analyze_alternatives(try_statement.clauses, None, |this, clause| {
                        this.analyze_block(clause.block)
                    })
            }
            StmtKind::Switch(switch) => {
                if !self.analyze_expr(switch.selector) {
                    return false;
                }
                // A value matching no `case` falls through unless a `default` (stored last, with
                // no constant) is present.
                let has_default = switch.cases.last().is_some_and(|case| case.constant.is_none());
                let fallthrough = (!has_default).then(|| self.flow_state());
                self.analyze_alternatives(switch.cases, fallthrough, |this, case| {
                    this.analyze_block(case.body)
                })
            }
            StmtKind::Break | StmtKind::Continue => {
                let state = self.flow_state();
                if let Some(flow) = self.loop_flow.last_mut() {
                    let destination = if matches!(statement.kind, StmtKind::Break) {
                        &mut flow.breaks
                    } else {
                        &mut flow.continues
                    };
                    join(destination, state);
                }
                false
            }
            StmtKind::Placeholder => match self.modifier_continuations.last().copied() {
                Some(cont) => self.analyze_modifier_chain(cont.modifiers, cont.next, cont.body),
                None => true,
            },
            StmtKind::Err(_) => true,
        }
    }

    /// Iterates the loop body from the joined loop-head state until the alias/guard state stops
    /// changing. Returns whether the loop can be left normally.
    fn analyze_loop(&mut self, block: hir::Block<'gcx>, source: LoopSource<'gcx>) -> bool {
        let mut head = self.flow_state();
        let mut exits = None;
        loop {
            self.set_flow_state(head.clone());
            let (mut breaks, continues, completes) = self.analyze_loop_stmts(|this| {
                this.analyze_block(block)
                    && loop_update(source).is_none_or(|update| this.analyze_stmt(update))
            });
            let mut backedges = completes.then(|| self.flow_state());
            // `continue` in a do-while still evaluates the lowered `if (!cond) break;`.
            let epilogue = matches!(source, LoopSource::DoWhile)
                .then(|| block.stmts.last())
                .flatten()
                .filter(|epilogue| is_loop_termination_if(epilogue));
            match (continues, epilogue) {
                (Some(state), Some(epilogue)) => {
                    self.set_flow_state(state);
                    let (epilogue_breaks, epilogue_continues, completes) =
                        self.analyze_loop_stmts(|this| this.analyze_stmt(epilogue));
                    if completes {
                        join(&mut backedges, self.flow_state());
                    }
                    breaks = breaks.into_iter().chain(epilogue_breaks).reduce(|a, b| a.merge(&b));
                    backedges =
                        backedges.into_iter().chain(epilogue_continues).reduce(|a, b| a.merge(&b));
                }
                (Some(state), None) => join(&mut backedges, state),
                (None, _) => {}
            }
            if let Some(breaks) = breaks {
                join(&mut exits, breaks);
            }
            let Some(backedges) = backedges else { break };
            let next = head.merge(&backedges);
            if next == head {
                break;
            }
            head = next;
        }
        let Some(exits) = exits else { return false };
        self.set_flow_state(exits);
        true
    }

    /// Runs `analyze` inside a fresh loop frame, returning the `break` state, the `continue` state
    /// and whether the body completed.
    fn analyze_loop_stmts(
        &mut self,
        analyze: impl FnOnce(&mut Self) -> bool,
    ) -> (Option<FlowState>, Option<FlowState>, bool) {
        self.loop_flow.push(LoopFlow::default());
        let completes = analyze(self);
        let flow = self.loop_flow.pop().expect("loop flow frame");
        (flow.breaks, flow.continues, completes)
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
            join(exits, state);
        }
    }

    /// Analyzes an expression, returning whether its evaluation can complete.
    fn analyze_expr(&mut self, expression: &'gcx hir::Expr<'gcx>) -> bool {
        match &expression.peel_parens().kind {
            ExprKind::Assign(lhs, operator, rhs) => {
                if !(self.analyze_expr(rhs) && self.analyze_expr(lhs)) {
                    return false;
                }
                self.apply_assignment(lhs, rhs, operator.is_some());
                true
            }
            ExprKind::Delete(inner) => {
                if !self.analyze_expr(inner) {
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
                if !(self.analyze_expr(callee)
                    && options
                        .iter()
                        .flat_map(|options| options.args)
                        .all(|option| self.analyze_expr(&option.value))
                    && args.exprs().all(|argument| self.analyze_expr(argument)))
                {
                    return false;
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

                if builtins(callee).any(|builtin| builtin == Builtin::YulSstore)
                    && let Some(slot) = args.exprs().next()
                {
                    let roots = self.slot_roots(slot);
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
                self.analyze_expr(lhs)
                    && if matches!(operator.kind, BinOpKind::And | BinOpKind::Or) {
                        // The right operand may be skipped.
                        self.analyze_alternatives([Some(*rhs), None], None, |this, rhs| {
                            rhs.is_none_or(|rhs| this.analyze_expr(rhs))
                        })
                    } else {
                        self.analyze_expr(rhs)
                    }
            }
            ExprKind::Index(base, index) => {
                self.analyze_expr(base) && index.is_none_or(|index| self.analyze_expr(index))
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_expr(base)
                    && start.is_none_or(|start| self.analyze_expr(start))
                    && end.is_none_or(|end| self.analyze_expr(end))
            }
            ExprKind::Member(base, _) | ExprKind::YulMember(base, _) | ExprKind::Payable(base) => {
                self.analyze_expr(base)
            }
            ExprKind::Ternary(condition, if_true, if_false) => {
                self.analyze_expr(condition)
                    && self.analyze_alternatives([*if_true, *if_false], None, |this, arm| {
                        this.analyze_expr(arm)
                    })
            }
            ExprKind::Array(expressions) => {
                expressions.iter().all(|expression| self.analyze_expr(expression))
            }
            ExprKind::Tuple(expressions) => {
                expressions.iter().flatten().all(|expression| self.analyze_expr(expression))
            }
            ExprKind::New(_)
            | ExprKind::TypeCall(_)
            | ExprKind::Type(_)
            | ExprKind::Ident(_)
            | ExprKind::Lit(_)
            | ExprKind::Err(_) => true,
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

    fn set_storage_alias(&mut self, variable_id: VariableId, roots: StorageRoots) {
        let variable = self.gcx.hir.variable(variable_id);
        if !variable.kind.is_state()
            && variable.data_location == Some(DataLocation::Storage)
            && !roots.is_empty()
        {
            self.aliases.storage.insert(variable_id, roots);
        } else {
            self.aliases.storage.remove(&variable_id);
        }
    }

    fn set_slot_alias(&mut self, variable_id: VariableId, roots: StorageRoots) {
        if roots.is_empty() {
            self.aliases.slots.remove(&variable_id);
        } else {
            self.aliases.slots.insert(variable_id, roots);
        }
    }

    /// Storage pointer locals alias the roots they are bound to; inside assembly, Yul locals also
    /// track the slots they hold.
    fn alias_local(&mut self, variable_id: VariableId, roots: StorageRoots) {
        if self.assembly_depth > 0 {
            self.set_slot_alias(variable_id, roots.clone());
        }
        self.set_storage_alias(variable_id, roots);
    }

    fn alias_local_from_expr(&mut self, variable_id: VariableId, expression: &hir::Expr<'_>) {
        if self.assembly_depth > 0 {
            let roots = self.slot_roots(expression);
            self.set_slot_alias(variable_id, roots);
        }
        let roots = self.storage_roots(expression);
        self.set_storage_alias(variable_id, roots);
    }

    fn apply_assignment(
        &mut self,
        lhs: &'gcx hir::Expr<'gcx>,
        rhs: &'gcx hir::Expr<'gcx>,
        compound: bool,
    ) {
        let lhs = lhs.peel_parens();
        if compound {
            return self.record_write(lhs);
        }
        match &lhs.kind {
            // `pointer.slot := x` retargets a storage pointer.
            ExprKind::YulMember(base, member)
                if member.as_str() == "slot"
                    && let Some(local) = lhs_local_var(&self.gcx.hir, base) =>
            {
                let roots = self.slot_roots(rhs);
                self.set_storage_alias(local, roots);
            }
            ExprKind::Tuple(expressions) => {
                for (index, expression) in expressions.iter().enumerate() {
                    let Some(expression) = expression else { continue };
                    if let Some(local) = lhs_local_var(&self.gcx.hir, expression) {
                        let roots = self.storage_roots_for_output(rhs, index, expressions.len());
                        self.alias_local(local, roots);
                    } else {
                        self.record_write(expression);
                    }
                }
            }
            _ => match lhs_local_var(&self.gcx.hir, lhs) {
                Some(local) => self.alias_local_from_expr(local, rhs),
                None => self.record_write(lhs),
            },
        }
    }

    fn set_return_aliases(&mut self, expression: &'gcx hir::Expr<'gcx>) {
        let Some(&function_id) = self.stack.last() else { return };
        let outputs = self.gcx.hir.function(function_id).returns.len();
        let roots: Vec<_> = (0..outputs)
            .map(|index| self.storage_roots_for_output(expression, index, outputs))
            .collect();
        self.extend_returns(roots);
    }

    fn capture_named_returns(&mut self) {
        let Some(&function_id) = self.stack.last() else { return };
        let function = self.gcx.hir.function(function_id);
        let aliases = if function.is_yul { &self.aliases.slots } else { &self.aliases.storage };
        let roots: Vec<_> = function
            .returns
            .iter()
            .map(|return_id| aliases.get(return_id).cloned().unwrap_or_default())
            .collect();
        self.extend_returns(roots);
    }

    fn extend_returns(&mut self, roots: Vec<StorageRoots>) {
        if let Some(frame) = self.return_stack.last_mut() {
            for (returned, roots) in frame.iter_mut().zip(roots) {
                returned.extend(roots);
            }
        }
    }

    /// Storage roots flowing into output `index` of `outputs` from `expression`.
    fn storage_roots_for_output(
        &self,
        expression: &hir::Expr<'_>,
        index: usize,
        outputs: usize,
    ) -> StorageRoots {
        match &expression.peel_parens().kind {
            ExprKind::Tuple(expressions) if outputs > 1 => expressions
                .get(index)
                .copied()
                .flatten()
                .map_or_else(StorageRoots::new, |expression| self.storage_roots(expression)),
            ExprKind::Call(..) => self
                .call_returns
                .get(&expression.id)
                .and_then(|returns| returns.get(index))
                .cloned()
                .unwrap_or_default(),
            _ if outputs == 1 && index == 0 => self.storage_roots(expression),
            _ => StorageRoots::new(),
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

    /// Call arguments in declaration order of `declared_id`'s parameters, the attached receiver
    /// first.
    fn ordered_call_arguments(
        &self,
        declared_id: FunctionId,
        arguments: CallArgs<'gcx>,
        receiver: Option<&'gcx hir::Expr<'gcx>>,
    ) -> Vec<&'gcx hir::Expr<'gcx>> {
        let parameters = self.gcx.hir.function(declared_id).parameters;
        let parameters = &parameters[usize::from(receiver.is_some())..];
        let names: Vec<_> = parameters
            .iter()
            .map(|&parameter| self.gcx.hir.variable(parameter).name.map(|name| name.name))
            .collect();
        let arguments = (0..parameters.len())
            .filter_map(|index| arguments.argument_for_parameter(index, Some(&names)));
        receiver.into_iter().chain(arguments).collect()
    }

    /// `(declared, dispatched, attached receiver)` for a call that executes contract code in this
    /// storage context: bare identifiers, `using for` attached calls, library calls and
    /// `super.`/`Base.`/`Lib.` qualified calls.
    fn resolved_internal_call(
        &self,
        callee: &'gcx hir::Expr<'gcx>,
    ) -> Option<(FunctionId, FunctionId, Option<&'gcx hir::Expr<'gcx>>)> {
        let (function_id, attached) = match self.gcx.resolved_callee(callee.id) {
            Some(resolved) => (resolved.res.as_function()?, resolved.attached),
            None => (unique(function_ids(callee))?, false),
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
        self.gcx
            .hir
            .function(function_id)
            .contract
            .is_some_and(|contract_id| self.gcx.hir.contract(contract_id).kind.is_library())
    }

    /// The most-derived override of a virtual function or modifier in the analyzed hierarchy.
    fn dispatch_function(&self, function_id: FunctionId) -> FunctionId {
        let function = self.gcx.hir.function(function_id);
        if !function.virtual_ {
            return function_id;
        }
        let signature = callable_signature(self.gcx, function_id);
        self.bases
            .iter()
            .flat_map(|&contract_id| self.gcx.hir.contract(contract_id).functions())
            .find(|&candidate_id| {
                self.gcx.hir.function(candidate_id).kind == function.kind
                    && callable_signature(self.gcx, candidate_id) == signature
            })
            .unwrap_or(function_id)
    }

    /// State variables an lvalue may write: state roots, aliased storage pointers and
    /// storage-returning calls.
    fn storage_roots(&self, expression: &hir::Expr<'_>) -> StorageRoots {
        let mut roots = StorageRoots::new();
        self.collect_storage_roots(expression, &mut roots);
        roots
    }

    fn collect_storage_roots(&self, expression: &hir::Expr<'_>, roots: &mut StorageRoots) {
        let expression = expression.peel_parens();
        match &expression.kind {
            ExprKind::Ident(resolutions) => {
                for variable_id in resolutions.iter().filter_map(Res::as_variable) {
                    if self.gcx.hir.variable(variable_id).kind.is_state() {
                        roots.insert(variable_id);
                    } else if let Some(aliases) = self.aliases.storage.get(&variable_id) {
                        roots.extend(aliases);
                    }
                }
            }
            ExprKind::Index(base, _)
            | ExprKind::Slice(base, ..)
            | ExprKind::Member(base, _)
            | ExprKind::YulMember(base, _)
            | ExprKind::Payable(base)
            | ExprKind::Unary(_, base)
            | ExprKind::Delete(base) => self.collect_storage_roots(base, roots),
            ExprKind::Tuple(expressions) => {
                for expression in expressions.iter().flatten() {
                    self.collect_storage_roots(expression, roots);
                }
            }
            ExprKind::Ternary(_, if_true, if_false) => {
                self.collect_storage_roots(if_true, roots);
                self.collect_storage_roots(if_false, roots);
            }
            ExprKind::Call(..) => {
                roots.extend(self.call_returns.get(&expression.id).into_iter().flatten().flatten());
            }
            _ => {}
        }
    }

    /// State variables whose slot a Yul expression may evaluate to.
    fn slot_roots(&self, expression: &hir::Expr<'_>) -> StorageRoots {
        let mut roots = StorageRoots::new();
        self.collect_slot_roots(expression, &mut roots);
        roots
    }

    fn collect_slot_roots(&self, expression: &hir::Expr<'_>, roots: &mut StorageRoots) {
        let expression = expression.peel_parens();
        match &expression.kind {
            ExprKind::Ident(resolutions) => {
                for variable_id in resolutions.iter().filter_map(Res::as_variable) {
                    if let Some(aliases) = self.aliases.slots.get(&variable_id) {
                        roots.extend(aliases);
                    }
                }
            }
            ExprKind::YulMember(base, member) if member.as_str() == "slot" => {
                self.collect_storage_roots(base, roots);
            }
            ExprKind::Call(..) if self.call_returns.contains_key(&expression.id) => {
                roots.extend(self.call_returns[&expression.id].iter().flatten());
            }
            // Any other expression may propagate a slot computed from its operands.
            ExprKind::Call(callee, args, options) => {
                self.collect_slot_roots(callee, roots);
                for option in options.iter().flat_map(|options| options.args) {
                    self.collect_slot_roots(&option.value, roots);
                }
                for argument in args.exprs() {
                    self.collect_slot_roots(argument, roots);
                }
            }
            ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
                self.collect_slot_roots(lhs, roots);
                self.collect_slot_roots(rhs, roots);
            }
            ExprKind::Index(base, index) => {
                for expression in [Some(*base), *index].into_iter().flatten() {
                    self.collect_slot_roots(expression, roots);
                }
            }
            ExprKind::Slice(base, start, end) => {
                for expression in [Some(*base), *start, *end].into_iter().flatten() {
                    self.collect_slot_roots(expression, roots);
                }
            }
            ExprKind::Ternary(condition, if_true, if_false) => {
                for expression in [condition, if_true, if_false] {
                    self.collect_slot_roots(expression, roots);
                }
            }
            ExprKind::Member(base, _)
            | ExprKind::YulMember(base, _)
            | ExprKind::Payable(base)
            | ExprKind::Unary(_, base)
            | ExprKind::Delete(base) => self.collect_slot_roots(base, roots),
            ExprKind::Array(expressions) => {
                for expression in *expressions {
                    self.collect_slot_roots(expression, roots);
                }
            }
            ExprKind::Tuple(expressions) => {
                for expression in expressions.iter().flatten() {
                    self.collect_slot_roots(expression, roots);
                }
            }
            _ => {}
        }
    }
}

/// `super.f`, `Base.f` or `Lib.f`: a statically dispatched internal call.
fn is_static_internal_base(base: &hir::Expr<'_>) -> bool {
    is_builtin(base, sym::super_)
        || matches!(&base.peel_parens().kind, ExprKind::Ident(resolutions)
        if resolutions.iter().any(|resolution| {
            matches!(resolution, Res::Item(ItemId::Contract(_)) | Res::Namespace(_))
        }))
}

fn is_dynamic_array_or_bytes(gcx: Gcx<'_>, expression: &hir::Expr<'_>) -> bool {
    gcx.type_of_expr(expression.peel_parens().id).is_some_and(|ty| {
        matches!(
            ty.peel_refs().kind,
            TyKind::DynArray(_) | TyKind::Elementary(ElementaryType::Bytes)
        )
    })
}
