//! Finds raw environment reads whose values can cross a Foundry environment mutation.
//!
//! A bounded source-level interpreter follows scalar locals, tuple assignments, internal calls,
//! and modifier placeholders. A roll/warp marks live number/timestamp origins; a subsequent use
//! reports the original read. Reads on both sides of a mutation are also reported because a
//! compiler may reuse the earlier read. Branches retain separate states, and return, revert,
//! break, and continue stop the corresponding path. External calls are not inlined: the callee
//! has a separate EVM frame and its return data is already materialized.
//!
//! This analysis runs before optimization and never changes executable code. Cheatcodes are
//! recognized by their resolved signature and constant receiver address, including local aliases
//! and helper arguments, rather than by a variable or interface name. Getter results carry no
//! raw-read origin. Heap/storage aliases, indirect calls, low-level cheatcode calls, recursion,
//! and paths beyond the explicit analysis limits are not modeled. This is a source warning,
//! not a proof of a particular optimizer's scheduling decisions.

use super::CheatcodeEnvironment;
use crate::{
    linter::{LateLintPass, LintContext},
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
    ast::{BinOpKind, ElementaryType, FunctionKind},
    interface::Span,
    sema::{
        Gcx,
        builtins::Builtin,
        eval::ConstValue,
        hir::{self, Expr, ExprKind, Function, FunctionId, ItemId, Stmt, StmtKind, VariableId},
        ty::TyKind,
    },
};
use std::collections::HashMap;

declare_forge_lint!(
    BLOCK_NUMBER_ACROSS_ROLL,
    Severity::Med,
    "block-number-across-roll",
    "`block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead"
);

declare_forge_lint!(
    BLOCK_TIMESTAMP_ACROSS_WARP,
    Severity::Med,
    "block-timestamp-across-warp",
    "`block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead"
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
}

impl Environment {
    fn lint(self) -> &'static SolLint {
        match self {
            Self::Number => &BLOCK_NUMBER_ACROSS_ROLL,
            Self::Timestamp => &BLOCK_TIMESTAMP_ACROSS_WARP,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Read {
    environment: Environment,
    span: Span,
    changed: bool,
}

#[derive(Clone, Default)]
struct Value {
    reads: Vec<Read>,
    cheatcode: bool,
    tuple: Vec<Self>,
}

impl Value {
    fn merge(&mut self, other: &Self) {
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

    fn change(&mut self, environment: Environment) {
        for read in &mut self.reads {
            read.changed |= read.environment == environment;
        }
        for part in &mut self.tuple {
            part.change(environment);
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
    return_parameters: Vec<VariableId>,
    flow: Flow,
}

impl State {
    fn change(&mut self, environment: Environment) {
        self.seen.change(environment);
        for value in self.locals.values_mut() {
            value.change(environment);
        }
    }

    fn merge(&mut self, other: &Self) {
        self.seen.merge(&other.seen);
        for (var, value) in &other.locals {
            self.locals.entry(*var).or_default().merge(value);
        }
    }
}

impl<'gcx> LateLintPass<'gcx> for CheatcodeEnvironment {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        if func.body.is_none() || func.kind == FunctionKind::Modifier {
            return;
        }
        let mut checker = Checker {
            ctx,
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

struct Checker<'a, 's, 'p, 'gcx> {
    ctx: &'a LintContext<'s, 'p>,
    gcx: Gcx<'gcx>,
    contract: Option<hir::ContractId>,
    stack: Vec<Span>,
    remaining: usize,
}

impl<'gcx> Checker<'_, '_, '_, 'gcx> {
    const fn step(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }

    fn use_value(&self, value: &Value) {
        for read in &value.reads {
            if read.changed {
                self.ctx.emit(read.environment.lint(), read.span);
            }
        }
        for part in &value.tuple {
            self.use_value(part);
        }
    }

    /// Runs the next modifier, substituting the remaining function at each placeholder.
    fn layer(&mut self, func: &'gcx Function<'gcx>, index: usize, mut state: State) -> Vec<State> {
        if !self.step() || state.flow == Flow::Halt {
            return Vec::new();
        }
        if let Some(modifier) = func.modifiers.get(index) {
            let ItemId::Function(mut id) = modifier.id else { return Vec::new() };
            // A qualified `Base.modifier` names that implementation exactly.
            if modifier.span.lo() == modifier.name_span.lo()
                && let Some(contract) = self.contract
            {
                id = self.gcx.resolve_virtual_function(contract, id);
            }
            let definition = self.gcx.hir.function(id);
            let Some(body) = definition.body else { return Vec::new() };
            let values: Vec<_> = definition
                .parameters
                .iter()
                .map(|&param| {
                    let value = arg_for_param(&self.gcx.hir, definition, param, &modifier.args)
                        .map(|arg| self.expr(arg, &mut state))
                        .unwrap_or_default();
                    (param, value)
                })
                .collect();
            state.locals.extend(values);
            self.block(body.stmts, vec![state], Some((func, index + 1)))
        } else if let Some(body) = func.body {
            self.block(body.stmts, vec![state], None)
        } else {
            Vec::new()
        }
    }

    fn return_values(&self, func: &Function<'_>, state: &State) -> Value {
        let values: Vec<_> = func
            .returns
            .iter()
            .map(|var| state.locals.get(var).cloned().unwrap_or_default())
            .collect();
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
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                return self.block(block.stmts, vec![state], continuation);
            }
            StmtKind::DeclSingle(var) => {
                let value = self
                    .gcx
                    .hir
                    .variable(*var)
                    .initializer
                    .map(|expr| self.expr(expr, &mut state))
                    .unwrap_or_default();
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
                self.expr(cond, &mut state);
                let known = self.gcx.try_eval_const_value(cond).ok().and_then(|v| v.as_bool());
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

    fn expr(&mut self, expr: &'gcx Expr<'gcx>, state: &mut State) -> Value {
        if !self.step() || state.flow == Flow::Halt {
            return Value::default();
        }
        let expr = expr.peel_parens();
        let environment = match self.gcx.resolved_builtin(expr) {
            Some(Builtin::BlockNumber) => Some(Environment::Number),
            Some(Builtin::BlockTimestamp) => Some(Environment::Timestamp),
            _ => None,
        };
        if let Some(environment) = environment {
            for read in &state.seen.reads {
                if read.environment == environment && read.changed {
                    self.ctx.emit(environment.lint(), read.span);
                }
            }
            let value = Value {
                reads: vec![Read { environment, span: expr.span, changed: false }],
                ..Value::default()
            };
            state.seen.merge(&value);
            return value;
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
                if op.is_some() {
                    value.merge(&self.expr(lhs, state));
                    value.cheatcode = false;
                }
                self.bind(lhs, value.clone(), state);
                value
            }
            ExprKind::Delete(lhs) => {
                self.bind(lhs, Value::default(), state);
                Value::default()
            }
            ExprKind::Unary(op, inner) if is_inc_dec(op.kind) => {
                let mut value = self.expr(inner, state);
                value.cheatcode = false;
                self.bind(inner, value.clone(), state);
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
                self.expr(cond, state);
                match self.gcx.try_eval_const_value(cond).ok().and_then(|v| v.as_bool()) {
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
                            value.merge(&other);
                            state.merge(&alternate);
                        }
                        value
                    }
                }
            }
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let mut value = self.expr(lhs, state);
                let known = self.gcx.try_eval_const_value(lhs).ok().and_then(|v| v.as_bool());
                let skip = op.kind == BinOpKind::Or;
                if known == Some(skip) {
                    return value;
                }
                let before = state.clone();
                value.merge(&self.expr(rhs, state));
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
            ExprKind::Call(callee, args, opts) => {
                let receiver = if let ExprKind::Member(receiver, _) = &callee.peel_parens().kind {
                    self.expr(receiver, state)
                } else {
                    Value::default()
                };
                for option in opts.iter().flat_map(|opts| opts.args) {
                    self.expr(&option.value, state);
                }
                let arguments: Vec<_> =
                    args.exprs().map(|arg| (arg.id, self.expr(arg, state))).collect();
                if receiver.cheatcode
                    && let Some(environment) = self.mutation(callee)
                {
                    state.change(environment);
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
                if let Some(target) = target {
                    let func = self.gcx.hir.function(target);
                    let bindings = func
                        .parameters
                        .iter()
                        .map(|&param| {
                            let value = arg_for_param(&self.gcx.hir, func, param, args)
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

    fn mutation(&self, callee: &Expr<'_>) -> Option<Environment> {
        let func = self.gcx.hir.function(self.gcx.resolved_function(callee)?);
        let [param] = func.parameters else { return None };
        if !matches!(self.gcx.type_of_item((*param).into()).kind, TyKind::Elementary(ElementaryType::UInt(size)) if size.bits() == 256)
        {
            return None;
        }
        match func.name?.name.as_str() {
            "roll" => Some(Environment::Number),
            "warp" => Some(Environment::Timestamp),
            _ => None,
        }
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
        let caller_returns = std::mem::replace(&mut input.return_parameters, func.returns.to_vec());
        input.locals.extend(bindings);
        for var in func.returns {
            input.locals.insert(*var, Value::default());
        }
        let mut output = self
            .layer(func, 0, input)
            .into_iter()
            .filter(|s| matches!(s.flow, Flow::Next | Flow::Return));
        let mut value = Value::default();
        if let Some(mut first) = output.next() {
            value.merge(&self.return_values(func, &first));
            for other in output {
                value.merge(&self.return_values(func, &other));
                first.merge(&other);
            }
            first.flow = Flow::Next;
            first.return_parameters = caller_returns;
            *state = first;
        } else {
            state.flow = Flow::Halt;
        }
        self.stack.pop();
        self.use_value(&value);
        value
    }
}
