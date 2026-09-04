use super::EnumerableLoopRemoval;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{branch_always_exits, resolved_function},
    },
};
use alloy_primitives::U256;
use solar::{
    ast::{LitKind, UnOpKind},
    interface::Symbol,
    sema::{
        Gcx,
        hir::{
            self, BinOpKind, CallArgs, CallArgsKind, Expr, ExprKind, FunctionId, Hir, LoopSource,
            Res, Stmt, StmtKind, VarKind, VariableId, Visit,
        },
        ty::TyKind,
    },
};
use std::{convert::Infallible, ops::ControlFlow};

declare_forge_lint!(
    ENUMERABLE_LOOP_REMOVAL,
    Severity::High,
    "enumerable-loop-removal",
    "`remove` on an EnumerableSet inside a loop that iterates it with `at` corrupts the iteration"
);

// The detector reports only the shape it can judge without a flow analysis: a loop whose own
// index is written exclusively by simple unconditional increments, reads the set with `at` at
// that bare index, and removes from the same set in a straight-line body. Control flow,
// descending traversal, composite indices, and other shapes that need value or path reasoning
// are deliberately unreported, even when they corrupt iteration. Set operands that cannot be
// identified statically are conservatively treated as possible aliases, so false positives are
// still possible on set identity.

impl<'hir> LateLintPass<'hir> for EnumerableLoopRemoval {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if let Some(body) = &func.body {
            LoopFinder { gcx, hir, ctx, bindings: Vec::new() }.walk_body(body.stmts);
        }
    }
}

/// Walks a function body in statement order and, for each loop, flags the EnumerableSet `remove`
/// calls that corrupt that loop's own iteration. The walk keeps, at every point, what each local
/// `storage` reference last named, so each loop is judged against the bindings standing where it
/// runs rather than against every binding of the function.
struct LoopFinder<'ctx, 's, 'c, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    ctx: &'ctx LintContext<'s, 'c>,
    /// What each local `storage` reference names where the walk stands, the latest entry
    /// winning: the path its last straight-line binding resolved to, or `None` once a write
    /// leaves it without one answer (a conditional branch, a loop body, or an unreadable shape).
    bindings: Vec<(VariableId, Option<SetPath>)>,
}

impl<'hir> LoopFinder<'_, '_, '_, 'hir> {
    fn walk_body(&mut self, stmts: &'hir [Stmt<'hir>]) {
        for stmt in stmts {
            self.walk_stmt(stmt);
        }
    }

    fn walk_stmt(&mut self, stmt: &'hir Stmt<'hir>) {
        // A `for` desugars to `Block { init; Loop(For) }`; its index lives partly in the init,
        // which runs once, on the straight line entering the loop.
        if let StmtKind::Block(block) = &stmt.kind
            && let Some((last, init)) = block.stmts.split_last()
            && let StmtKind::Loop(body, source @ (LoopSource::For | LoopSource::ForWithUpdate)) =
                &last.kind
        {
            self.walk_body(init);
            return self.enter_loop(init, body.stmts, *source);
        }
        match &stmt.kind {
            // A bare block runs on the straight line: what it binds stays bound past it.
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => self.walk_body(block.stmts),
            StmtKind::Loop(body, source) => self.enter_loop(&[], body.stmts, *source),
            // Which branch ran is not tracked: everything the statement writes stops naming one
            // thing, and what a branch binds for its own statements ends with the branch.
            StmtKind::If(_, then, else_) => {
                self.poison_writes(std::slice::from_ref(stmt));
                let mark = self.bindings.len();
                self.walk_stmt(then);
                self.bindings.truncate(mark);
                if let Some(else_) = else_ {
                    self.walk_stmt(else_);
                    self.bindings.truncate(mark);
                }
            }
            StmtKind::Try(try_) => {
                self.poison_writes(std::slice::from_ref(stmt));
                let mark = self.bindings.len();
                for clause in try_.clauses {
                    self.walk_body(clause.block.stmts);
                    self.bindings.truncate(mark);
                }
            }
            _ => self.apply_bindings(stmt),
        }
    }

    /// Analyzes one loop, then walks inside it for the nested ones. A write anywhere in the
    /// loop may have run on an earlier turn by the time any statement of it runs again, so
    /// everything the loop writes, init included, stops naming one thing before the loop is
    /// judged, and stays so past it.
    fn enter_loop(
        &mut self,
        init: &'hir [Stmt<'hir>],
        body: &'hir [Stmt<'hir>],
        source: LoopSource,
    ) {
        self.poison_writes(init);
        self.poison_writes(body);
        self.analyze_loop(real_body(source, body));
        let mark = self.bindings.len();
        self.walk_body(body);
        self.bindings.truncate(mark);
    }

    /// Applies one straight-line statement to the bindings. Everything it writes stops naming
    /// one thing first; a declaration or a plain assignment then binds its reference to what
    /// the right-hand side names right here, resolved eagerly because a later write to a
    /// reference the right-hand side reads must not reach back into this binding.
    fn apply_bindings(&mut self, stmt: &'hir Stmt<'hir>) {
        self.poison_writes(std::slice::from_ref(stmt));
        let bindings = &mut self.bindings;
        let mut bind = |var: VariableId, value: &Expr<'_>| {
            let path = set_path(self.hir, value, bindings, &mut Vec::new());
            bindings.push((var, path));
        };
        match &stmt.kind {
            StmtKind::DeclSingle(var) => {
                if let Some(init) = self.hir.variable(*var).initializer {
                    bind(*var, init);
                }
            }
            StmtKind::Expr(expr) => {
                if let ExprKind::Assign(target, None, value) = &expr.peel_parens().kind
                    && let ExprKind::Ident(reses) = &target.peel_parens().kind
                {
                    for var in reses.iter().filter_map(Res::as_variable) {
                        bind(var, value);
                    }
                }
            }
            _ => {}
        }
    }

    /// Marks everything the statements write as no longer naming one thing.
    fn poison_writes(&mut self, stmts: &'hir [Stmt<'hir>]) {
        let mut written = Vec::new();
        collect_variables(self.hir, stmts, &mut written);
        self.bindings.extend(written.into_iter().map(|var| (var, None)));
    }

    /// Flags the removals in `body` that corrupt this loop's iteration: an unconditional
    /// ascending cadence, a straight-line body, and a `remove` on a set the loop reads with `at`
    /// at that cadence.
    fn analyze_loop(&mut self, body: &'hir [Stmt<'hir>]) {
        // Control flow in the body would make the corruption depend on the path taken, which
        // this detector does not track; without an ascending index there is no upward walk for
        // the swap-and-pop corruption to skip.
        if !body_is_straight_line(body) {
            return;
        }
        let cadence = ascending_cadence(self.hir, body);
        if cadence.is_empty() {
            return;
        }
        // Sets iterated with `at` at the cadence, and every reachable removal, including calls
        // nested in short-circuit or ternary expressions whose arm a literal does not rule out.
        let (mut iterated, mut removes) = (Vec::new(), Vec::new());
        let mut calls = ExprWalker {
            hir: self.hir,
            prune_unreachable: true,
            f: |expr: &'hir Expr<'hir>| {
                let Some(call) = enumerable_set_call(self.gcx, self.hir, &self.bindings, expr)
                else {
                    return;
                };
                match call.op {
                    SetOp::At => {
                        if call.index.and_then(Expr::as_variable).is_some_and(|i| cadence.contains(&i))
                        {
                            iterated.push(call.set);
                        }
                    }
                    SetOp::Remove => removes.push((call.set, expr.span)),
                }
            },
        };
        for stmt in body {
            let _ = calls.visit_stmt(stmt);
        }
        for (removed, span) in removes {
            // Two readable paths name the same set exactly when equal; an unreadable one may.
            let corrupts = iterated
                .iter()
                .any(|iterated| removed.as_ref().zip(iterated.as_ref()).is_none_or(|(a, b)| a == b));
            if corrupts {
                self.ctx.emit(&ENUMERABLE_LOOP_REMOVAL, span);
            }
        }
    }
}

/// Calls `f` on every expression under the visited statements. With `prune_unreachable`, the
/// arms of `&&`/`||`/`?:` that a literal boolean condition proves unreachable are skipped.
struct ExprWalker<'hir, F> {
    hir: &'hir Hir<'hir>,
    prune_unreachable: bool,
    f: F,
}

impl<'hir, F: FnMut(&'hir Expr<'hir>)> Visit<'hir> for ExprWalker<'hir, F> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Infallible> {
        (self.f)(expr);
        if !self.prune_unreachable {
            return self.walk_expr(expr);
        }
        match &expr.kind {
            ExprKind::Binary(left, op, right)
                if matches!(op.kind, BinOpKind::And | BinOpKind::Or) =>
            {
                self.visit_expr(left)?;
                let short_circuits = matches!(
                    (op.kind, literal_bool(left)),
                    (BinOpKind::And, Some(false)) | (BinOpKind::Or, Some(true))
                );
                if !short_circuits {
                    self.visit_expr(right)?;
                }
                ControlFlow::Continue(())
            }
            ExprKind::Ternary(condition, true_expr, false_expr) => {
                self.visit_expr(condition)?;
                match literal_bool(condition) {
                    Some(true) => self.visit_expr(true_expr),
                    Some(false) => self.visit_expr(false_expr),
                    None => {
                        self.visit_expr(true_expr)?;
                        self.visit_expr(false_expr)
                    }
                }
            }
            _ => self.walk_expr(expr),
        }
    }
}

/// The user-written body of a loop, peeled out of the synthetic condition guard the AST lowering
/// wraps it in. A `for`/`while` lowers to a single `if (cond) { body } else break`, its `body`
/// holding the user statements (and, for a `for`, the next-step); a `do-while` appends an
/// `if (cond) continue else break` after the user statements. Peeling these makes the guard's
/// `break`/`continue` and the next-step read for what they are, not as user control flow. A body
/// that does not match the exact synthetic shape is returned unchanged.
const fn real_body<'hir>(source: LoopSource, body: &'hir [Stmt<'hir>]) -> &'hir [Stmt<'hir>] {
    match source {
        LoopSource::For | LoopSource::ForWithUpdate | LoopSource::While => {
            if let [only] = body
                && let StmtKind::If(_, then, Some(else_)) = &only.kind
                && matches!(else_.kind, StmtKind::Break)
            {
                return std::slice::from_ref(*then);
            }
        }
        LoopSource::DoWhile => {
            if let Some((last, rest)) = body.split_last()
                && let StmtKind::If(_, then, Some(else_)) = &last.kind
                && matches!(then.kind, StmtKind::Continue)
                && matches!(else_.kind, StmtKind::Break)
            {
                return rest;
            }
        }
    }
    body
}

/// Whether every statement of a loop body runs on one straight line: no branch (`if`/`try`), no
/// jump (`break`/`continue`), no terminal statement, no inline assembly, and no nested loop. Bare
/// blocks are transparent. Any of these would let control skip a removal, skip the cadence step,
/// or leave the loop before a shifted slot is read, none of which this detector tracks.
fn body_is_straight_line(stmts: &[Stmt<'_>]) -> bool {
    stmts.iter().all(|stmt| {
        !branch_always_exits(stmt)
            && match &stmt.kind {
                StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                    body_is_straight_line(block.stmts)
                }
                StmtKind::If(..)
                | StmtKind::Try(..)
                | StmtKind::Loop(..)
                | StmtKind::AssemblyBlock(..)
                | StmtKind::Break
                | StmtKind::Continue => false,
                _ => true,
            }
    })
}

/// The loop's own indices that step upward unconditionally: bare identifiers whose every write
/// on the straight line of the body (bare blocks included) is a supported ascending step. A
/// reset (`i = 0`), a no-op (`i += 0`), a decrement, or composite arithmetic disqualifies the
/// variable: the walk is only known to ascend for the simple forms.
fn ascending_cadence<'hir>(hir: &'hir Hir<'hir>, body: &'hir [Stmt<'hir>]) -> Vec<VariableId> {
    let (mut cadence, mut other_writes) = (Vec::new(), Vec::new());
    collect_cadence_writes(hir, body, &mut cadence, &mut other_writes);
    cadence.retain(|var| !other_writes.contains(var));
    cadence
}

fn collect_cadence_writes<'hir>(
    hir: &'hir Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
    cadence: &mut Vec<VariableId>,
    other_writes: &mut Vec<VariableId>,
) {
    for stmt in stmts {
        let mut written = match &stmt.kind {
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                collect_cadence_writes(hir, block.stmts, cadence, other_writes);
                continue;
            }
            StmtKind::DeclSingle(var) => vec![*var],
            StmtKind::DeclMulti(vars, _) => vars.iter().flatten().copied().collect(),
            _ => Vec::new(),
        };
        collect_variables(hir, std::slice::from_ref(stmt), &mut written);
        let ascending = match &stmt.kind {
            StmtKind::Expr(expr) => ascending_step(expr.peel_parens()),
            _ => None,
        };
        for var in written {
            if ascending != Some(var) {
                other_writes.push(var);
            } else if !cadence.contains(&var) {
                cadence.push(var);
            }
        }
    }
}

/// The bare identifier an expression steps upward, if it is one of the simple ascending forms:
/// `i++`/`++i`, `i += <positive literal>`, `i = i + <positive literal>` or `i = <positive literal> + i`.
fn ascending_step<'hir>(expr: &'hir Expr<'hir>) -> Option<VariableId> {
    match &expr.kind {
        ExprKind::Unary(op, operand) if matches!(op.kind, UnOpKind::PreInc | UnOpKind::PostInc) => {
            operand.as_variable()
        }
        ExprKind::Assign(lhs, Some(op), rhs)
            if op.kind == BinOpKind::Add && is_positive_literal(rhs) =>
        {
            lhs.as_variable()
        }
        ExprKind::Assign(lhs, None, rhs) => {
            let target = lhs.as_variable()?;
            let ExprKind::Binary(left, op, right) = &rhs.peel_parens().kind else { return None };
            (op.kind == BinOpKind::Add
                && ((left.as_variable() == Some(target) && is_positive_literal(right))
                    || (is_positive_literal(left) && right.as_variable() == Some(target))))
            .then_some(target)
        }
        _ => None,
    }
}

/// A non-zero integer literal.
fn is_positive_literal(expr: &Expr<'_>) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Lit(lit)
        if matches!(&lit.kind, LitKind::Number(value) if !value.is_zero()))
}

fn literal_bool(expr: &Expr<'_>) -> Option<bool> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => match lit.kind {
            LitKind::Bool(value) => Some(value),
            _ => None,
        },
        _ => None,
    }
}

/// The variables a statement list writes through expressions, nested loops included:
/// assignments (tuple targets included), increments, decrements, and deletes.
fn collect_variables<'hir>(
    hir: &'hir Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
    out: &mut Vec<VariableId>,
) {
    let mut writes = ExprWalker {
        hir,
        prune_unreachable: false,
        f: |expr: &Expr<'_>| {
            let target = match &expr.kind {
                ExprKind::Assign(target, ..) | ExprKind::Delete(target) => target,
                ExprKind::Unary(op, target)
                    if matches!(
                        op.kind,
                        UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                    ) =>
                {
                    target
                }
                _ => return,
            };
            collect_lvalue_variables(target, out);
        },
    };
    for stmt in stmts {
        let _ = writes.visit_stmt(stmt);
    }
}

/// The variables written through an assignment target. Tuple targets can contain several
/// identifiers; member and indexed targets do not write the base variable itself.
fn collect_lvalue_variables(expr: &Expr<'_>, out: &mut Vec<VariableId>) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => out.extend(reses.iter().filter_map(Res::as_variable)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().for_each(|expr| collect_lvalue_variables(expr, out));
        }
        _ => {}
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum SetOp {
    At,
    Remove,
}

/// A resolved EnumerableSet call.
struct SetCall<'hir> {
    op: SetOp,
    set: Option<SetPath>,
    /// The `index` argument of `at`.
    index: Option<&'hir Expr<'hir>>,
}

/// The EnumerableSet `at` or `remove` a call dispatches to. Resolving through the type checker
/// covers the `using for` method form, the library-qualified form and import aliases. The library
/// is identified only by its kind and exact `EnumerableSet` name, not its source or behavior.
fn enumerable_set_call<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    bindings: &Bindings,
    expr: &'hir Expr<'hir>,
) -> Option<SetCall<'hir>> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let function_id = resolved_function(gcx, callee)?;
    let function = hir.function(function_id);
    let contract = hir.contract(function.contract?);
    if !contract.kind.is_library() || contract.name.as_str() != "EnumerableSet" {
        return None;
    }
    let op = match function.name?.as_str() {
        "at" => SetOp::At,
        "remove" => SetOp::Remove,
        _ => return None,
    };
    // The set operand is the bound receiver in the method form and the first argument in the
    // library-qualified form; the index of `at` sits right after it.
    let (set_expr, index_arg) = match &callee.peel_parens().kind {
        ExprKind::Member(receiver, _) if is_enumerable_set_value(gcx, hir, receiver) => {
            (Some(&**receiver), 0)
        }
        _ => (nth_argument(hir, function_id, args, 0, 0), 1),
    };
    Some(SetCall {
        op,
        set: set_expr.and_then(|expr| set_path(hir, expr, bindings, &mut Vec::new())),
        index: nth_argument(hir, function_id, args, index_arg, 1),
    })
}

/// One step of the storage path naming a set: a struct field or a literal mapping key.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Step {
    Field(Symbol),
    Key(U256),
}

/// The storage location a set expression names: a base variable and the steps taken from it.
/// `holders`, `pair.a` and `sets[1]` each name one, and two of them are the same set exactly
/// when they are the same path.
#[derive(PartialEq, Eq, Clone)]
struct SetPath {
    base: VariableId,
    steps: Vec<Step>,
}

/// What each local `storage` reference names at the point being analyzed, the latest entry
/// winning; `None` marks a reference no straight-line reading gives one answer for.
type Bindings = [(VariableId, Option<SetPath>)];

/// The path a set expression names, or `None` when it cannot be read: an index that varies, a
/// call result, a reference without one straight-line binding, anything the analysis would have
/// to evaluate.
fn set_path(
    hir: &Hir<'_>,
    expr: &Expr<'_>,
    bindings: &Bindings,
    seen: &mut Vec<VariableId>,
) -> Option<SetPath> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(_) => {
            let var = expr.as_variable()?;
            if seen.contains(&var) {
                return None;
            }
            seen.push(var);
            let variable = hir.variable(var);
            if !matches!(variable.kind, VarKind::Statement) {
                return Some(SetPath { base: var, steps: Vec::new() });
            }
            // A local `storage` reference is another name for the set its last binding gave it.
            // A reference declared inside the analyzed loop has no entry and is bound by its
            // initializer anew each turn; a tuple-destructured one has neither and may name any
            // set.
            match bindings.iter().rev().find(|(bound, _)| *bound == var) {
                Some((_, binding)) => binding.clone(),
                None => set_path(hir, variable.initializer?, bindings, seen),
            }
        }
        ExprKind::Member(base, field) => {
            let mut path = set_path(hir, base, bindings, seen)?;
            path.steps.push(Step::Field(field.name));
            Some(path)
        }
        ExprKind::Index(base, Some(index)) => {
            let ExprKind::Lit(lit) = &index.peel_parens().kind else { return None };
            let LitKind::Number(key) = &lit.kind else { return None };
            let mut path = set_path(hir, base, bindings, seen)?;
            path.steps.push(Step::Key(*key));
            Some(path)
        }
        _ => None,
    }
}

/// The argument at position `arg` of a positional call, or the one a named call binds to the
/// callee's parameter at position `parameter`. In the method form the bound receiver fills the
/// first parameter, so positional arguments sit one position before the parameters they fill.
fn nth_argument<'hir>(
    hir: &'hir Hir<'hir>,
    function_id: FunctionId,
    args: &'hir CallArgs<'hir>,
    arg: usize,
    parameter: usize,
) -> Option<&'hir Expr<'hir>> {
    match &args.kind {
        CallArgsKind::Unnamed(exprs) => exprs.get(arg),
        CallArgsKind::Named(named) => {
            let parameter = *hir.function(function_id).parameters.get(parameter)?;
            let name = hir.variable(parameter).name?;
            named.iter().find(|argument| argument.name.name == name.name).map(|arg| &arg.value)
        }
    }
}

/// Whether `receiver` is a value of one of the set struct types declared in a library (or
/// contract) named `EnumerableSet`, which tells the bound method form apart from the
/// library-qualified form.
fn is_enumerable_set_value<'hir>(gcx: Gcx<'hir>, hir: &'hir Hir<'hir>, receiver: &Expr<'_>) -> bool {
    let Some(ty) = gcx.type_of_expr(receiver.peel_parens().id) else { return false };
    let TyKind::Struct(id) = ty.peel_refs().kind else { return false };
    hir.strukt(id).contract.is_some_and(|c| hir.contract(c).name.as_str() == "EnumerableSet")
}
