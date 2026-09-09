//! Finds raw environment reads whose values can cross a Foundry environment mutation.
//!
//! A bounded source-level interpreter follows scalar locals, tuple assignments, internal calls,
//! and modifier placeholders. An environment setter marks matching live origins; a subsequent use
//! reports the original read. Reads on both sides of a mutation are also reported because a
//! compiler may reuse the earlier read. Branches retain separate states, and return, revert,
//! break, and continue stop the corresponding path. External calls are not inlined: the callee
//! has a separate EVM frame and its return data is already materialized.
//! Differing helper return values and conditional-expression values are discarded instead of
//! combining components from mutually exclusive outcomes.
//!
//! This analysis runs before optimization and never changes executable code. Cheatcodes are
//! recognized by their resolved signature and constant receiver address, including local aliases
//! and helper arguments, rather than by a variable or interface name. Getter results carry no
//! raw-read origin. Heap/storage aliases, indirect calls, low-level cheatcode calls, recursion,
//! and paths beyond the explicit analysis limits are not modeled. This is a source warning,
//! not a proof of a particular optimizer's scheduling decisions.

use super::CheatcodeEnvironment;
use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{
        Severity, SolLint,
        analysis::{
            arg_for_param, dispatched_function, for_each_child, is_exit_call, is_inc_dec,
            loop_update,
        },
    },
};
use alloy_primitives::{U256, keccak256, uint};
use solar::{
    ast::{BinOpKind, ElementaryType, FunctionKind, UnOpKind},
    interface::{Span, diagnostics::DiagId, source_map::FileName},
    sema::{
        Gcx,
        builtins::Builtin,
        eval::ConstValue,
        hir::{self, Expr, ExprKind, Function, FunctionId, Stmt, StmtKind, VariableId},
        ty::TyKind,
    },
};
use std::collections::HashMap;

declare_forge_lint!(
    ENVIRONMENT_READ_ACROSS_MUTATION,
    Severity::Med,
    "environment-read-across-mutation",
    "environment read may be reused across a Foundry environment mutation"
);

const CHEATCODE_ADDRESS: U256 = uint!(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D_U256);
const MAX_STEPS: usize = 16_384;
const MAX_PATHS: usize = 32;
const MAX_CALL_DEPTH: usize = 8;
const MAX_LOOP_ITERATIONS: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Environment {
    Number,
    Timestamp,
    ChainId,
    Coinbase,
    Difficulty,
    Prevrandao,
    BaseFee,
    BlobBaseFee,
    GasLimit,
    SlotNumber,
    GasPrice,
    BlockHash,
    BlobHash,
}

impl Environment {
    /// Block/configuration fields and blockhash history replaced by fork operations.
    const BLOCK: &'static [Self] = &[
        Self::Number,
        Self::Timestamp,
        Self::ChainId,
        Self::Coinbase,
        Self::Difficulty,
        Self::Prevrandao,
        Self::BaseFee,
        Self::BlobBaseFee,
        Self::GasLimit,
        Self::SlotNumber,
        Self::BlockHash,
    ];

    /// Snapshots also restore cheatcode overrides of transaction properties.
    const ALL: &'static [Self] = &[
        Self::Number,
        Self::Timestamp,
        Self::ChainId,
        Self::Coinbase,
        Self::Difficulty,
        Self::Prevrandao,
        Self::BaseFee,
        Self::BlobBaseFee,
        Self::GasLimit,
        Self::SlotNumber,
        Self::BlockHash,
        Self::GasPrice,
        Self::BlobHash,
    ];

    const fn from_builtin(builtin: Builtin) -> Option<Self> {
        Some(match builtin {
            Builtin::BlockNumber => Self::Number,
            Builtin::BlockTimestamp => Self::Timestamp,
            Builtin::BlockChainid => Self::ChainId,
            Builtin::BlockCoinbase => Self::Coinbase,
            Builtin::BlockDifficulty => Self::Difficulty,
            Builtin::BlockPrevrandao => Self::Prevrandao,
            Builtin::BlockBasefee => Self::BaseFee,
            Builtin::BlockBlobbasefee => Self::BlobBaseFee,
            Builtin::BlockGaslimit => Self::GasLimit,
            Builtin::BlockSlotnum => Self::SlotNumber,
            Builtin::TxGasPrice => Self::GasPrice,
            Builtin::Blockhash => Self::BlockHash,
            Builtin::Blobhash => Self::BlobHash,
            _ => return None,
        })
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Number => "block.number",
            Self::Timestamp => "block.timestamp",
            Self::ChainId => "block.chainid",
            Self::Coinbase => "block.coinbase",
            Self::Difficulty => "block.difficulty",
            Self::Prevrandao => "block.prevrandao",
            Self::BaseFee => "block.basefee",
            Self::BlobBaseFee => "block.blobbasefee",
            Self::GasLimit => "block.gaslimit",
            Self::SlotNumber => "block.slotnum",
            Self::GasPrice => "tx.gasprice",
            Self::BlockHash => "blockhash(...)",
            Self::BlobHash => "blobhash(...)",
        }
    }

    const fn getter(self) -> Option<&'static str> {
        Some(match self {
            Self::Number => "vm.getBlockNumber()",
            Self::Timestamp => "vm.getBlockTimestamp()",
            Self::ChainId => "vm.getChainId()",
            Self::BlobBaseFee => "vm.getBlobBaseFee()",
            Self::BlobHash => "vm.getBlobhashes()",
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Read {
    environment: Environment,
    span: Span,
    changed: Option<Mutation>,
    origin: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Mutation {
    span: Span,
    function: FunctionId,
}

/// Exact unsigned and boolean locals used to prune exhausted loops and constant branches.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scalar {
    Uint(U256),
    Bool(bool),
}

impl Scalar {
    const fn as_bool(self) -> Option<bool> {
        if let Self::Bool(value) = self { Some(value) } else { None }
    }

    fn binary(self, op: BinOpKind, rhs: Self) -> Option<Self> {
        Some(match (self, rhs) {
            (Self::Uint(lhs), Self::Uint(rhs)) => match op {
                BinOpKind::Lt => Self::Bool(lhs < rhs),
                BinOpKind::Le => Self::Bool(lhs <= rhs),
                BinOpKind::Gt => Self::Bool(lhs > rhs),
                BinOpKind::Ge => Self::Bool(lhs >= rhs),
                BinOpKind::Eq => Self::Bool(lhs == rhs),
                BinOpKind::Ne => Self::Bool(lhs != rhs),
                _ => return None,
            },
            (Self::Bool(lhs), Self::Bool(rhs)) => Self::Bool(match op {
                BinOpKind::And => lhs && rhs,
                BinOpKind::Or => lhs || rhs,
                BinOpKind::Eq => lhs == rhs,
                BinOpKind::Ne => lhs != rhs,
                _ => return None,
            }),
            _ => return None,
        })
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Value {
    reads: Vec<Read>,
    cheatcode: bool,
    tuple: Vec<Self>,
    scalar: Option<Scalar>,
}

impl Value {
    fn merge(&mut self, other: &Self) {
        if self.scalar != other.scalar {
            self.scalar = None;
        }
        for read in &other.reads {
            if !self.reads.contains(read) {
                self.reads.push(*read);
            }
        }
        self.cheatcode |= other.cheatcode;
        self.tuple.resize_with(self.tuple.len().max(other.tuple.len()), Self::default);
        for (a, b) in self.tuple.iter_mut().zip(&other.tuple) {
            a.merge(b);
        }
    }

    fn change(&mut self, environment: Environment, mutation: Mutation) {
        for read in &mut self.reads {
            if read.environment == environment {
                // Keep the first matching call on this path, not an unrelated or later setter.
                read.changed.get_or_insert(mutation);
            }
        }
        for part in &mut self.tuple {
            part.change(environment, mutation);
        }
    }

    /// Refresh origins held outside locals while sibling expressions execute.
    fn refresh(&mut self, state: &State) {
        for read in &mut self.reads {
            read.changed = read.changed.or_else(|| state.changed.get(&read.origin).copied());
        }
        for part in &mut self.tuple {
            part.refresh(state);
        }
    }

    fn part(&self, index: usize) -> Self {
        self.tuple.get(index).cloned().unwrap_or_default()
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Flow {
    #[default]
    Next,
    Return,
    Break,
    Continue,
    Halt,
}

#[derive(Clone, Default)]
struct State {
    locals: HashMap<VariableId, Value>,
    seen: Value,
    changed: HashMap<usize, Mutation>,
    return_parameters: Vec<VariableId>,
    flow: Flow,
    unchecked: bool,
}

impl State {
    fn change(&mut self, environment: Environment, mutation: Mutation) {
        for read in self.seen.reads.iter().filter(|read| read.environment == environment) {
            self.changed.entry(read.origin).or_insert(mutation);
        }
        self.seen.change(environment, mutation);
        for value in self.locals.values_mut() {
            value.change(environment, mutation);
        }
    }

    fn merge(&mut self, other: &Self) {
        for (&origin, &mutation) in &other.changed {
            self.changed.entry(origin).or_insert(mutation);
        }
        self.seen.merge(&other.seen);
        for (var, value) in &other.locals {
            self.locals.entry(*var).or_default().merge(value);
        }
    }
}

// Project sources expose the span-owner policies needed to emit a multi-span diagnostic.
// The pinned Solar late-pass context only supports single-span messages and suggestions.
impl<'ast> ProjectLintPass<'ast> for CheatcodeEnvironment {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]) {
        if !ctx.is_lint_enabled(ENVIRONMENT_READ_ACROSS_MUTATION.id) {
            return;
        }
        let gcx = ctx.gcx();
        let input_sources = gcx
            .hir
            .sources_enumerated()
            .filter_map(|(id, source)| {
                let FileName::Real(path) = &source.file.name else { return None };
                Some((id, sources.iter().find(|source| &source.path == path)?))
            })
            .collect::<HashMap<_, _>>();
        for func in gcx.hir.functions() {
            if func.body.is_none()
                || func.kind == FunctionKind::Modifier
                || !input_sources.contains_key(&func.source)
            {
                continue;
            }
            let mut checker = Checker {
                sources,
                gcx,
                contract: func.contract,
                stack: vec![func.span],
                remaining: MAX_STEPS,
            };
            let initial = State { return_parameters: func.returns.to_vec(), ..State::default() };
            for state in checker.layer(func, 0, initial) {
                if matches!(state.flow, Flow::Next | Flow::Return) {
                    checker.use_value(&checker.return_values(func, &state));
                }
            }
        }
    }
}

struct Checker<'a, 'ast, 'gcx> {
    sources: &'a [ProjectSource<'ast>],
    gcx: Gcx<'gcx>,
    contract: Option<hir::ContractId>,
    stack: Vec<Span>,
    remaining: usize,
}

impl<'gcx> Checker<'_, '_, 'gcx> {
    const fn step(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }

    fn use_value(&self, value: &Value) {
        for read in &value.reads {
            if read.changed.is_some() {
                self.emit(*read);
            }
        }
        for part in &value.tuple {
            self.use_value(part);
        }
    }

    fn emit(&self, read: Read) {
        let Some(mutation) = read.changed else { return };
        let lint = &ENVIRONMENT_READ_ACROSS_MUTATION;
        // The primary read can belong to an inherited helper in another source. Use its
        // policy, not the mutation's, and never emit diagnostics for dependency-only files.
        let Some(source) = self.sources.iter().find(|source| source.file.contains(read.span.lo()))
        else {
            return;
        };
        if !source.policy.is_lint_enabled(lint.id)
            || source.policy.is_lint_suppressed(lint.id, read.span)
        {
            return;
        }
        let name = read.environment.name();
        let setter = self.gcx.hir.function(mutation.function).name.unwrap();
        let advice = read.environment.getter().map_or_else(
            || "capture it through an external helper call instead".to_string(),
            |getter| format!("capture it with `{getter}` instead"),
        );
        self.gcx
            .sess
            .dcx
            .diag::<()>(lint.level(), format!("`{name}` may be reused across `vm.{setter}`"))
            .code(DiagId::new_str(lint.id))
            .span(read.span)
            .span_label(mutation.span, format!("`vm.{setter}` changes this environment here"))
            .help(advice)
            .help(lint.help)
            .emit();
    }

    fn read(&mut self, environment: Environment, span: Span, state: &mut State) -> Value {
        if !self.step() || state.flow == Flow::Halt {
            return Value::default();
        }
        for read in &state.seen.reads {
            if read.environment == environment && read.changed.is_some() {
                self.emit(*read);
            }
        }
        let value = Value {
            reads: vec![Read {
                environment,
                span,
                changed: None,
                // The decreasing step budget gives repeated evaluations distinct identities.
                origin: self.remaining,
            }],
            ..Value::default()
        };
        state.seen.merge(&value);
        value
    }

    /// Runs the next modifier, substituting the remaining function at each placeholder.
    fn layer(&mut self, func: &'gcx Function<'gcx>, index: usize, mut state: State) -> Vec<State> {
        if !self.step() || state.flow == Flow::Halt {
            return Vec::new();
        }
        if let Some(modifier) = func.modifiers.get(index) {
            let Some(id) = self.contract.map_or_else(
                || modifier.id.as_function(),
                |contract| self.gcx.resolve_modifier_target(contract, modifier),
            ) else {
                return Vec::new();
            };
            let definition = self.gcx.hir.function(id);
            let Some(body) = definition.body else { return Vec::new() };
            let values = definition
                .parameters
                .iter()
                .map(|&param| {
                    let value = arg_for_param(&self.gcx.hir, definition, param, &modifier.args)
                        .map(|arg| self.expr(arg, &mut state))
                        .unwrap_or_default();
                    (param, value)
                })
                .collect::<Vec<_>>();
            state.locals.extend(values);
            self.block(body.stmts, vec![state], Some((func, index + 1)))
        } else if let Some(body) = func.body {
            self.block(body.stmts, vec![state], None)
        } else {
            Vec::new()
        }
    }

    fn return_values(&self, func: &Function<'_>, state: &State) -> Value {
        let values = func
            .returns
            .iter()
            .map(|var| state.locals.get(var).cloned().unwrap_or_default())
            .collect::<Vec<_>>();
        if values.len() == 1 {
            values.into_iter().next().unwrap_or_default()
        } else {
            Value { tuple: values, ..Value::default() }
        }
    }

    fn block(
        &mut self,
        stmts: &'gcx [Stmt<'gcx>],
        mut states: Vec<State>,
        continuation: Option<(&'gcx Function<'gcx>, usize)>,
    ) -> Vec<State> {
        for stmt in stmts {
            let mut next = Vec::new();
            for state in states {
                if state.flow == Flow::Next {
                    next.extend(self.stmt(stmt, state, continuation));
                } else {
                    next.push(state);
                }
                if next.len() >= MAX_PATHS {
                    break;
                }
            }
            next.truncate(MAX_PATHS);
            states = next;
        }
        states
    }

    fn stmt(
        &mut self,
        stmt: &'gcx Stmt<'gcx>,
        mut state: State,
        continuation: Option<(&'gcx Function<'gcx>, usize)>,
    ) -> Vec<State> {
        if !self.step() || state.flow == Flow::Halt {
            return Vec::new();
        }
        match &stmt.kind {
            StmtKind::Block(block) => {
                return self.block(block.stmts, vec![state], continuation);
            }
            StmtKind::UncheckedBlock(block) => {
                let previous = std::mem::replace(&mut state.unchecked, true);
                let mut states = self.block(block.stmts, vec![state], continuation);
                for state in &mut states {
                    state.unchecked = previous;
                }
                return states;
            }
            StmtKind::DeclSingle(var) => {
                let value = self
                    .gcx
                    .hir
                    .variable(*var)
                    .initializer
                    .map(|expr| self.expr(expr, &mut state))
                    .unwrap_or_else(|| self.default_value(*var));
                state.locals.insert(*var, value);
            }
            StmtKind::DeclMulti(vars, expr) => {
                let value = self.expr(expr, &mut state);
                for (index, var) in vars.iter().enumerate() {
                    if let Some(var) = var {
                        state.locals.insert(*var, value.part(index));
                    }
                }
            }
            StmtKind::If(cond, then, otherwise) => {
                let known = self.expr(cond, &mut state).scalar.and_then(Scalar::as_bool);
                let mut states = Vec::new();
                if known != Some(false) {
                    states.extend(self.stmt(then, state.clone(), continuation));
                }
                if known != Some(true) {
                    states.extend(otherwise.map_or_else(
                        || vec![state.clone()],
                        |stmt| self.stmt(stmt, state.clone(), continuation),
                    ));
                }
                return states;
            }
            StmtKind::Loop(body, source) => {
                // HIR includes the loop condition and its break edge in the body.
                let mut active = vec![state];
                let mut exits = Vec::new();
                for _ in 0..MAX_LOOP_ITERATIONS {
                    let iteration = self.block(body.stmts, active, continuation);
                    active = Vec::new();
                    for mut state in iteration {
                        match state.flow {
                            Flow::Break => {
                                state.flow = Flow::Next;
                                exits.push(state);
                            }
                            Flow::Next | Flow::Continue => {
                                state.flow = Flow::Next;
                                if let Some(update) = loop_update(*source) {
                                    active.extend(self.stmt(update, state, continuation));
                                } else {
                                    active.push(state);
                                }
                            }
                            _ => exits.push(state),
                        }
                    }
                    active.truncate(MAX_PATHS);
                    exits.truncate(MAX_PATHS);
                }
                // Check the final for/while condition without executing another body. A loop
                // exhausted exactly at the iteration budget still reaches following statements.
                if !matches!(source, hir::LoopSource::DoWhile)
                    && let [stmt] = body.stmts
                    && let StmtKind::If(cond, _, Some(otherwise)) = &stmt.kind
                    && matches!(otherwise.kind, StmtKind::Break)
                {
                    for mut state in active {
                        let known = self.expr(cond, &mut state).scalar.and_then(Scalar::as_bool);
                        if known != Some(true) && state.flow == Flow::Next {
                            exits.push(state);
                        }
                    }
                    exits.truncate(MAX_PATHS);
                }
                // Only observed exit edges reach following statements. Do not manufacture an
                // exit from an infinite loop or treat an unvisited iteration as execution.
                return exits;
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    let value = self.expr(expr, &mut state);
                    if state.flow == Flow::Halt {
                        return vec![state];
                    }
                    for (index, var) in state.return_parameters.iter().enumerate() {
                        let part = if state.return_parameters.len() == 1 {
                            value.clone()
                        } else {
                            value.part(index)
                        };
                        state.locals.insert(*var, part);
                    }
                }
                state.flow = Flow::Return;
            }
            StmtKind::Break => state.flow = Flow::Break,
            StmtKind::Continue => state.flow = Flow::Continue,
            StmtKind::Revert(expr) => {
                self.expr(expr, &mut state);
                state.flow = Flow::Halt;
            }
            StmtKind::Expr(expr) | StmtKind::Emit(expr) => {
                self.expr(expr, &mut state);
                if is_exit_call(expr) {
                    state.flow = Flow::Halt;
                }
            }
            StmtKind::Try(try_stmt) => {
                // Argument evaluation happens in the caller even when the external call fails.
                // Effects of a reverted cheatcode call itself must not reach a catch clause.
                let mut failure = state.clone();
                for_each_child(&try_stmt.expr, &mut |child| {
                    self.expr(child, &mut failure);
                });
                self.expr(&try_stmt.expr, &mut state);
                return try_stmt
                    .clauses
                    .iter()
                    .enumerate()
                    .flat_map(|(index, clause)| {
                        let input = if index == 0 { &state } else { &failure };
                        self.block(clause.block.stmts, vec![input.clone()], continuation)
                    })
                    .take(MAX_PATHS)
                    .collect();
            }
            StmtKind::Placeholder => {
                if let Some((func, index)) = continuation {
                    let mut states = self.layer(func, index, state);
                    for state in &mut states {
                        if state.flow == Flow::Return {
                            state.flow = Flow::Next;
                        }
                    }
                    return states;
                }
            }
            // Assembly can both overwrite locals and terminate the frame. Stop this path rather
            // than carrying facts across unmodeled instructions.
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => {
                return Vec::new();
            }
        }
        vec![state]
    }

    fn bind(&self, lhs: &Expr<'_>, value: Value, state: &mut State) {
        match &lhs.peel_parens().kind {
            ExprKind::Tuple(parts) => {
                for (index, part) in parts.iter().enumerate() {
                    if let Some(part) = part {
                        self.bind(part, value.part(index), state);
                    }
                }
            }
            _ => {
                if let Some(var) = lhs.as_variable()
                    && !self.gcx.hir.variable(var).is_state_variable()
                {
                    state.locals.insert(var, value);
                }
            }
        }
    }

    /// Evaluate destination components without reading an overwritten local.
    fn destination(&mut self, expr: &'gcx Expr<'gcx>, state: &mut State) {
        let expr = expr.peel_parens();
        if let ExprKind::Tuple(parts) = &expr.kind {
            for part in parts.iter().flatten() {
                self.destination(part, state);
            }
        } else if expr.as_variable().is_none() {
            for_each_child(expr, &mut |child| {
                self.expr(child, state);
            });
        }
    }

    fn expr(&mut self, expr: &'gcx Expr<'gcx>, state: &mut State) -> Value {
        let mut value = self.expr_inner(expr, state);
        value.refresh(state);
        self.use_value(&value);
        value.scalar = value.scalar.or_else(|| {
            self.gcx.try_eval_const_value(expr).ok().and_then(|value| {
                value.as_bool().map(Scalar::Bool).or_else(|| value.as_u256().map(Scalar::Uint))
            })
        });
        // Only exact unsigned and boolean values participate in branch pruning.
        value.scalar =
            value.scalar.filter(|scalar| match (scalar, self.gcx.type_of_expr(expr.id)) {
                (Scalar::Uint(value), Some(ty)) => match ty.kind {
                    TyKind::Elementary(ElementaryType::UInt(size)) => {
                        value.bit_len() <= size.bits() as usize
                    }
                    TyKind::IntLiteral(false, ..) => true,
                    _ => false,
                },
                (Scalar::Bool(_), Some(ty)) => {
                    matches!(ty.kind, TyKind::Elementary(ElementaryType::Bool))
                }
                _ => false,
            });
        value
    }

    fn expr_inner(&mut self, expr: &'gcx Expr<'gcx>, state: &mut State) -> Value {
        if !self.step() || state.flow == Flow::Halt {
            return Value::default();
        }
        let expr = expr.peel_parens();
        if let Some(environment) =
            self.gcx.resolved_builtin(expr).and_then(Environment::from_builtin)
            && !matches!(environment, Environment::BlockHash | Environment::BlobHash)
        {
            return self.read(environment, expr.span, state);
        }
        if let Some(var) = expr.as_variable()
            && let Some(value) = state.locals.get(&var)
        {
            self.use_value(value);
            return value.clone();
        }
        if self.constant_word(expr, 0) == Some(CHEATCODE_ADDRESS) {
            return Value { cheatcode: true, ..Value::default() };
        }
        match &expr.kind {
            ExprKind::Assign(lhs, op, rhs) => {
                let mut value = self.expr(rhs, state);
                if let Some(op) = op {
                    let left = self.expr(lhs, state);
                    let scalar = self.binary_scalar(lhs, op.kind, left.scalar, value.scalar, state);
                    value.merge(&left);
                    value.scalar = scalar;
                    value.cheatcode = false;
                }
                if op.is_none() {
                    self.destination(lhs, state);
                }
                value.refresh(state);
                self.bind(lhs, value.clone(), state);
                value
            }
            ExprKind::Delete(lhs) => {
                self.destination(lhs, state);
                let value =
                    lhs.as_variable().map(|var| self.default_value(var)).unwrap_or_default();
                self.bind(lhs, value, state);
                Value::default()
            }
            ExprKind::Unary(op, inner) if is_inc_dec(op.kind) => {
                let before = self.expr(inner, state);
                let mut value = before.clone();
                let binary = if matches!(op.kind, UnOpKind::PreInc | UnOpKind::PostInc) {
                    BinOpKind::Add
                } else {
                    BinOpKind::Sub
                };
                value.scalar = self.binary_scalar(
                    inner,
                    binary,
                    value.scalar,
                    Some(Scalar::Uint(U256::from(1))),
                    state,
                );
                value.cheatcode = false;
                self.bind(inner, value.clone(), state);
                if op.kind.is_prefix() { value } else { before }
            }
            ExprKind::Unary(op, inner) => {
                let mut value = self.expr(inner, state);
                value.scalar = match (op.kind, value.scalar) {
                    (UnOpKind::Not, Some(Scalar::Bool(value))) => Some(Scalar::Bool(!value)),
                    _ => None,
                };
                value.cheatcode = false;
                value
            }
            ExprKind::Payable(inner) => self.expr(inner, state),
            ExprKind::Tuple(parts) => Value {
                tuple: parts
                    .iter()
                    .map(|part| part.map(|part| self.expr(part, state)).unwrap_or_default())
                    .collect(),
                ..Value::default()
            },
            ExprKind::Ternary(cond, yes, no) => {
                match self.expr(cond, state).scalar.and_then(Scalar::as_bool) {
                    Some(true) => self.expr(yes, state),
                    Some(false) => self.expr(no, state),
                    None => {
                        let mut alternate = state.clone();
                        let mut value = self.expr(yes, state);
                        let other = self.expr(no, &mut alternate);
                        if state.flow == Flow::Halt {
                            *state = alternate;
                            value = other;
                        } else if alternate.flow != Flow::Halt {
                            if value != other {
                                value = Value::default();
                            }
                            state.merge(&alternate);
                        }
                        value
                    }
                }
            }
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let mut value = self.expr(lhs, state);
                let known = value.scalar.and_then(Scalar::as_bool);
                let skip = op.kind == BinOpKind::Or;
                if known == Some(skip) {
                    return value;
                }
                let before = state.clone();
                let right = self.expr(rhs, state);
                let scalar = self.binary_scalar(expr, op.kind, value.scalar, right.scalar, state);
                value.merge(&right);
                value.scalar = scalar;
                if known.is_none() {
                    if state.flow == Flow::Halt {
                        *state = before;
                    } else {
                        state.merge(&before);
                    }
                }
                value.cheatcode = false;
                value
            }
            ExprKind::Binary(lhs, op, rhs) => {
                let mut value = self.expr(lhs, state);
                let right = self.expr(rhs, state);
                let scalar = self.binary_scalar(expr, op.kind, value.scalar, right.scalar, state);
                value.merge(&right);
                value.scalar = scalar;
                value.cheatcode = false;
                value
            }
            ExprKind::Call(callee, args, opts) => {
                let mut receiver = if let ExprKind::Member(receiver, _) = &callee.peel_parens().kind
                {
                    self.expr(receiver, state)
                } else {
                    Value::default()
                };
                for option in opts.iter().flat_map(|opts| opts.args) {
                    self.expr(&option.value, state);
                }
                let mut arguments =
                    args.exprs().map(|arg| (arg.id, self.expr(arg, state))).collect::<Vec<_>>();
                receiver.refresh(state);
                self.use_value(&receiver);
                for (_, value) in &mut arguments {
                    value.refresh(state);
                    self.use_value(value);
                }
                if let Some(environment) =
                    self.gcx.resolved_builtin(callee).and_then(Environment::from_builtin)
                    && matches!(environment, Environment::BlockHash | Environment::BlobHash)
                {
                    let mut value = self.read(environment, expr.span, state);
                    // A hash result also depends on its raw environment-derived index.
                    for (_, argument) in &arguments {
                        value.merge(argument);
                    }
                    value.cheatcode = false;
                    return value;
                }
                if receiver.cheatcode
                    && let Some((function, environments)) = self.mutation(callee)
                {
                    for &environment in environments {
                        state.change(environment, Mutation { span: expr.span, function });
                    }
                    return Value::default();
                }
                if matches!(
                    self.gcx.type_of_expr(callee.id).map(|ty| ty.kind),
                    Some(TyKind::Type(_))
                ) {
                    let mut value =
                        arguments.into_iter().next().map(|(_, value)| value).unwrap_or_default();
                    // Only address-preserving casts keep a proven cheatcode receiver.
                    value.cheatcode &= self.cast_bits(callee).is_some_and(|bits| bits >= 160);
                    return value;
                }
                let target = if let Some(contract) = self.contract {
                    dispatched_function(self.gcx, contract, callee)
                } else if matches!(callee.kind, ExprKind::Ident(_)) {
                    self.gcx.resolved_function(callee)
                } else {
                    None
                };
                if let Some(target) = target
                    && matches!(self.gcx.type_of_expr(callee.id).map(|ty| ty.kind), Some(TyKind::Fn(f)) if f.is_internal())
                {
                    let func = self.gcx.hir.function(target);
                    let bindings = func
                        .parameters
                        .iter()
                        .enumerate()
                        .map(|(index, &param)| {
                            let value = self
                                .gcx
                                .call_arg(expr, index)
                                .and_then(|arg| arguments.iter().find(|(id, _)| *id == arg.id))
                                .map(|(_, value)| value.clone())
                                .unwrap_or_default();
                            (param, value)
                        })
                        .collect();
                    return self.call(target, bindings, state);
                }
                // External return data and getter results are materialized at the call boundary.
                Value::default()
            }
            _ => {
                let mut value = Value::default();
                for_each_child(expr, &mut |child| value.merge(&self.expr(child, state)));
                value.cheatcode = false;
                value
            }
        }
    }

    fn default_value(&self, var: VariableId) -> Value {
        let scalar = match self.gcx.type_of_item(var.into()).kind {
            TyKind::Elementary(ElementaryType::UInt(_)) => Some(Scalar::Uint(U256::ZERO)),
            TyKind::Elementary(ElementaryType::Bool) => Some(Scalar::Bool(false)),
            _ => None,
        };
        Value { scalar, ..Value::default() }
    }

    fn binary_scalar(
        &self,
        expr: &Expr<'_>,
        op: BinOpKind,
        lhs: Option<Scalar>,
        rhs: Option<Scalar>,
        state: &mut State,
    ) -> Option<Scalar> {
        let (lhs, rhs) = (lhs?, rhs?);
        if matches!(op, BinOpKind::Add | BinOpKind::Sub)
            && let (Scalar::Uint(lhs), Scalar::Uint(rhs)) = (lhs, rhs)
            && let TyKind::Elementary(ElementaryType::UInt(size)) =
                self.gcx.type_of_expr(expr.id)?.kind
        {
            let (value, overflow) = if op == BinOpKind::Add {
                lhs.overflowing_add(rhs)
            } else {
                lhs.overflowing_sub(rhs)
            };
            if !state.unchecked && (overflow || value.bit_len() > size.bits() as usize) {
                state.flow = Flow::Halt;
                return None;
            }
            return Some(Scalar::Uint(value & (U256::MAX >> (256 - size.bits() as usize))));
        }
        lhs.binary(op, rhs)
    }

    fn mutation(&self, callee: &Expr<'_>) -> Option<(FunctionId, &'static [Environment])> {
        let function = self.gcx.resolved_function(callee)?;
        // Match the ABI signature, including overloads, rather than just the method name.
        let environments: &'static [Environment] = match self.gcx.item_signature(function.into()) {
            "roll(uint256)" => &[Environment::Number, Environment::BlockHash],
            "warp(uint256)" => &[Environment::Timestamp],
            "chainId(uint256)" => &[Environment::ChainId],
            "coinbase(address)" => &[Environment::Coinbase],
            "difficulty(uint256)" | "prevrandao(bytes32)" | "prevrandao(uint256)" => {
                &[Environment::Difficulty, Environment::Prevrandao]
            }
            "fee(uint256)" => &[Environment::BaseFee],
            "blobBaseFee(uint256)" => &[Environment::BlobBaseFee],
            "txGasPrice(uint256)" => &[Environment::GasPrice],
            "setBlockhash(uint256,bytes32)" => &[Environment::BlockHash],
            "blobhashes(bytes32[])" => &[Environment::BlobHash],
            "selectFork(uint256)"
            | "createSelectFork(string)"
            | "createSelectFork(string,uint256)"
            | "createSelectFork(string,bytes32)" => Environment::ALL,
            "rollFork(uint256)"
            | "rollFork(bytes32)"
            | "rollFork(uint256,uint256)"
            | "rollFork(uint256,bytes32)" => Environment::BLOCK,
            "revertTo(uint256)"
            | "revertToState(uint256)"
            | "revertToAndDelete(uint256)"
            | "revertToStateAndDelete(uint256)" => Environment::ALL,
            _ => return None,
        };
        Some((function, environments))
    }

    /// Evaluates only the constant-address forms used by cheatcode declarations. The pinned
    /// compiler's integer evaluator does not yet support general casts or keccak calls.
    fn constant_word(&self, expr: &Expr<'_>, depth: usize) -> Option<U256> {
        if depth >= 16 {
            return None;
        }
        let expr = expr.peel_parens();
        if let Ok(value) = self.gcx.try_eval_const(expr) {
            return value.as_u256();
        }
        if let Some(id) = self.gcx.resolved_variable(expr) {
            let var = self.gcx.hir.variable(id);
            return var
                .is_constant()
                .then_some(var.initializer)
                .flatten()
                .and_then(|init| self.constant_word(init, depth + 1));
        }
        match &expr.kind {
            ExprKind::Call(callee, args, None) if args.exprs().count() == 1 => {
                let arg = args.exprs().next()?;
                if self.gcx.resolved_builtin(callee) == Some(Builtin::Keccak256) {
                    let ConstValue::String(bytes) = self.gcx.try_eval_const_value(arg).ok()? else {
                        return None;
                    };
                    return Some(U256::from_be_bytes(keccak256(bytes.as_byte_str()).0));
                }
                let bits = self.cast_bits(callee)?;
                let value = self.constant_word(arg, depth + 1)?;
                Some(value & (U256::MAX >> (256 - bits)))
            }
            ExprKind::Payable(inner) => self.constant_word(inner, depth + 1),
            _ => None,
        }
    }

    fn cast_bits(&self, callee: &Expr<'_>) -> Option<usize> {
        let TyKind::Type(ty) = self.gcx.type_of_expr(callee.id)?.kind else { return None };
        match ty.kind {
            TyKind::Contract(_) | TyKind::Elementary(ElementaryType::Address(_)) => Some(160),
            TyKind::Elementary(ElementaryType::UInt(size) | ElementaryType::Int(size)) => {
                Some(size.bits() as usize)
            }
            // A bytesN cast can change alignment. Only bytes32 preserves the complete word.
            TyKind::Elementary(ElementaryType::FixedBytes(size)) if size.bits() == 256 => Some(256),
            _ => None,
        }
    }

    fn call(
        &mut self,
        id: FunctionId,
        bindings: Vec<(VariableId, Value)>,
        state: &mut State,
    ) -> Value {
        let func = self.gcx.hir.function(id);
        if func.body.is_none()
            || self.stack.len() >= MAX_CALL_DEPTH
            || self.stack.contains(&func.span)
        {
            return Value::default();
        }
        self.stack.push(func.span);
        let mut input = state.clone();
        // An unchecked block does not change arithmetic in a called function.
        input.unchecked = false;
        let caller_returns = std::mem::replace(&mut input.return_parameters, func.returns.to_vec());
        input.locals.extend(bindings);
        for var in func.returns {
            input.locals.insert(*var, self.default_value(*var));
        }
        let mut output = self
            .layer(func, 0, input)
            .into_iter()
            .filter(|s| matches!(s.flow, Flow::Next | Flow::Return));
        let mut value = Value::default();
        if let Some(mut first) = output.next() {
            value = self.return_values(func, &first);
            self.use_value(&value);
            let mut ambiguous = false;
            for other in output {
                let returned = self.return_values(func, &other);
                self.use_value(&returned);
                ambiguous |= value != returned;
                first.merge(&other);
            }
            // Component-wise joins can invent tuples that no execution returns, e.g.
            // `(block.number, nonVm)` and `(0, vm)`. Keep only an unambiguous return value.
            if ambiguous {
                value = Value::default();
            }
            first.flow = Flow::Next;
            first.unchecked = state.unchecked;
            first.return_parameters = caller_returns;
            *state = first;
        } else {
            state.flow = Flow::Halt;
        }
        self.stack.pop();
        value
    }
}
