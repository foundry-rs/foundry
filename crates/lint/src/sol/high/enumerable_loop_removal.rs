use super::EnumerableLoopRemoval;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use alloy_primitives::U256;
use solar::{
    ast::{LitKind, UnOpKind},
    interface::{Span, Symbol},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, CallArgs, CallArgsKind, Expr, ExprKind, FunctionId, Hir, ItemId, LoopSource, Res,
            Stmt, StmtKind, VarKind, VariableId, Visit,
        },
        ty::TyKind,
    },
};
use std::{collections::HashSet, convert::Infallible, ops::ControlFlow};

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
            let mut finder =
                LoopFinder { gcx, hir, ctx, bindings: Vec::new(), emitted: HashSet::new() };
            finder.walk_body(body.stmts);
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
    /// leaves it without one answer, from a conditional branch, a loop body, or a shape the
    /// analysis cannot read.
    bindings: Vec<(VariableId, Option<SetPath>)>,
    // A loop nested in a flagged loop sees the same calls: dedupe emissions by span.
    emitted: HashSet<Span>,
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
            && let Some(last) = block.stmts.last()
            && let StmtKind::Loop(body, LoopSource::For) = &last.kind
        {
            let init = &block.stmts[..block.stmts.len() - 1];
            self.walk_body(init);
            self.enter_loop(init, body.stmts, LoopSource::For);
            return;
        }
        match &stmt.kind {
            // A bare block runs on the straight line: what it binds stays bound past it.
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.walk_body(block.stmts);
            }
            // A bare loop is a `while`, a `do-while`, or a `for` with no init.
            StmtKind::Loop(body, source) => self.enter_loop(&[], body.stmts, *source),
            StmtKind::If(_, then, else_) => {
                // The condition and either branch may write a reference, and which of them ran
                // is not read here: everything the statement writes stops naming one thing, and
                // what a branch binds for its own statements ends with the branch.
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
                // Clauses are branches the same way.
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
        // Analyze the user-written body, peeled out of the synthetic condition guard the lowering
        // wraps every loop in (`if (cond) { body } else break`), so the guard's `break` and the
        // `for`'s next-step are read for what they are, not as user control flow.
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
        match &stmt.kind {
            StmtKind::DeclSingle(variable_id) => {
                if let Some(initializer) = self.hir.variable(*variable_id).initializer {
                    let resolved = set_path(self.hir, initializer, &self.bindings, &mut Vec::new());
                    self.bindings.push((*variable_id, resolved));
                }
            }
            StmtKind::Expr(expr) => {
                if let ExprKind::Assign(target, None, value) = &expr.peel_parens().kind
                    && let ExprKind::Ident(resolutions) = &target.peel_parens().kind
                {
                    let resolved = set_path(self.hir, value, &self.bindings, &mut Vec::new());
                    for res in *resolutions {
                        if let Res::Item(ItemId::Variable(variable_id)) = res {
                            self.bindings.push((*variable_id, resolved.clone()));
                        }
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
        for variable_id in written {
            self.bindings.push((variable_id, None));
        }
    }

    /// Flags the removals in `body` that corrupt this loop's iteration, under the shrunk rule:
    /// an unconditional ascending cadence, a straight-line body, and a `remove` on a set the
    /// loop reads with `at` at that cadence.
    fn analyze_loop(&mut self, body: &'hir [Stmt<'hir>]) {
        // A control-flow construct in the body would make the corruption depend on where
        // control flows, which this detector does not track. Stay silent instead of guessing.
        if !body_is_straight_line(body) {
            return;
        }
        // The loop's own index must step upward on the straight line of the body, never under a
        // branch. Without one, the iteration order is not the ascending walk the swap-and-pop
        // corruption needs.
        let cadence = ascending_cadence(self.hir, body);
        if cadence.is_empty() {
            return;
        }
        // Which sets this loop iterates with `at` at that cadence.
        let mut ats = AtCollector {
            gcx: self.gcx,
            hir: self.hir,
            bindings: &self.bindings,
            cadence: &cadence,
            iterated: Vec::new(),
        };
        for stmt in body {
            let _ = ats.visit_stmt(stmt);
        }
        if ats.iterated.is_empty() {
            return;
        }
        // Every removal in the body, conditional or not: a straight-line body has no exit that
        // could make a conditional removal safe, so the mutation always shifts a slot the
        // ascending walk still reads.
        let mut removes = Vec::new();
        let mut scan = RemoveScanner {
            gcx: self.gcx,
            hir: self.hir,
            bindings: &self.bindings,
            out: &mut removes,
        };
        for stmt in body {
            let _ = scan.visit_stmt(stmt);
        }
        for (removed, span) in removes {
            let corrupts = ats.iterated.iter().any(|iterated| paths_alias(&removed, iterated));
            if corrupts && self.emitted.insert(span) {
                self.ctx.emit(&ENUMERABLE_LOOP_REMOVAL, span);
            }
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
        LoopSource::For | LoopSource::While => {
            if let [only] = body
                && let StmtKind::If(_, then, Some(else_)) = &only.kind
                && matches!(else_.kind, StmtKind::Break)
            {
                return std::slice::from_ref(*then);
            }
            body
        }
        LoopSource::DoWhile => {
            if let Some((last, rest)) = body.split_last()
                && let StmtKind::If(_, then, Some(else_)) = &last.kind
                && matches!(then.kind, StmtKind::Continue)
                && matches!(else_.kind, StmtKind::Break)
            {
                return rest;
            }
            body
        }
    }
}

/// Whether every statement of a loop body runs on one straight line: no branch (`if`/`try`), no
/// jump (`break`/`continue`/`return`/`revert`), and no nested loop. Bare blocks are transparent.
/// Any of these would let control skip a removal, skip the cadence step, or leave the loop before
/// a shifted slot is read, none of which this detector tracks, so their presence makes it silent.
fn body_is_straight_line(stmts: &[Stmt<'_>]) -> bool {
    stmts.iter().all(|stmt| match &stmt.kind {
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            body_is_straight_line(block.stmts)
        }
        StmtKind::If(..)
        | StmtKind::Try(..)
        | StmtKind::Loop(..)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Return(..)
        | StmtKind::Revert(..) => false,
        StmtKind::Expr(expr) if is_builtin_revert(expr) => false,
        _ => true,
    })
}

/// Whether an expression statement is Solidity's builtin `revert()` or `revert(string)` call.
/// Custom-error revert statements have their own `StmtKind::Revert` arm above.
fn is_builtin_revert(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, ..) = &expr.peel_parens().kind else { return false };
    let ExprKind::Ident(resolutions) = &callee.peel_parens().kind else { return false };
    resolutions.iter().any(|res| matches!(res, Res::Builtin(Builtin::Revert | Builtin::RevertMsg)))
}

/// The loop's own indices that step upward unconditionally: a bare identifier advanced by `i++`,
/// `i += <positive literal>`, or `i = i + <positive literal>` as a straight-line statement of the
/// body (a `for`'s desugared post-step lands here, a `while`'s in-body counter too). A step under
/// a branch, a reset (`i = 0`), a no-op (`i += 0`), a decrement, or composite arithmetic
/// (`i = (i + 2) - 1`) does not qualify: the walk is only known to ascend for the simple forms.
fn ascending_cadence<'hir>(hir: &'hir Hir<'hir>, body: &'hir [Stmt<'hir>]) -> Vec<VariableId> {
    let mut cadence = Vec::new();
    let mut other_writes = HashSet::new();
    collect_cadence_writes(hir, body, &mut cadence, &mut other_writes);
    cadence.retain(|variable_id| !other_writes.contains(variable_id));
    cadence
}

/// Walks the straight-line statements of a body, bare blocks included, and records variables
/// written by a supported ascending step separately from every other write. A cadence is valid
/// only when all of its writes are supported ascending steps.
fn collect_cadence_writes<'hir>(
    hir: &'hir Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
    cadence: &mut Vec<VariableId>,
    other_writes: &mut HashSet<VariableId>,
) {
    for stmt in stmts {
        match &stmt.kind {
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                collect_cadence_writes(hir, block.stmts, cadence, other_writes);
            }
            _ => {
                let ascending = match &stmt.kind {
                    StmtKind::Expr(expr) => ascending_step(expr.peel_parens()),
                    _ => None,
                };
                let mut written = Vec::new();
                collect_variables(hir, std::slice::from_ref(stmt), &mut written);
                for variable_id in written {
                    if ascending == Some(variable_id) {
                        if !cadence.contains(&variable_id) {
                            cadence.push(variable_id);
                        }
                    } else {
                        other_writes.insert(variable_id);
                    }
                }
            }
        }
    }
}

/// The bare identifier an expression steps upward, if it is one of the simple ascending forms.
fn ascending_step<'hir>(expr: &'hir Expr<'hir>) -> Option<VariableId> {
    match &expr.kind {
        // `i++` / `++i`.
        ExprKind::Unary(op, operand) if matches!(op.kind, UnOpKind::PreInc | UnOpKind::PostInc) => {
            bare_identifier(operand)
        }
        // `i += <positive literal>`.
        ExprKind::Assign(lhs, Some(op), rhs)
            if op.kind == hir::BinOpKind::Add && is_positive_literal(rhs) =>
        {
            bare_identifier(lhs)
        }
        // `i = i + <positive literal>`.
        ExprKind::Assign(lhs, None, rhs) => {
            let target = bare_identifier(lhs)?;
            let ExprKind::Binary(left, op, right) = &rhs.peel_parens().kind else { return None };
            (op.kind == hir::BinOpKind::Add
                && bare_identifier(left) == Some(target)
                && is_positive_literal(right))
            .then_some(target)
        }
        _ => None,
    }
}

/// The variable a bare identifier expression resolves to, or `None` for anything else.
fn bare_identifier(expr: &Expr<'_>) -> Option<VariableId> {
    let ExprKind::Ident(resolutions) = &expr.peel_parens().kind else { return None };
    resolutions.iter().find_map(|res| match res {
        Res::Item(ItemId::Variable(variable_id)) => Some(*variable_id),
        _ => None,
    })
}

/// Whether an expression is a positive integer literal: `1`, `2`, never `0`.
fn is_positive_literal(expr: &Expr<'_>) -> bool {
    let ExprKind::Lit(lit) = &expr.peel_parens().kind else { return false };
    matches!(&lit.kind, LitKind::Number(value) if !value.is_zero())
}

/// Collects the sets a loop iterates with `at` at its ascending cadence. The index must be the
/// bare cadence identifier; copies and composite expressions are deliberately left out.
struct AtCollector<'a, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    bindings: &'a Bindings,
    cadence: &'a [VariableId],
    iterated: Vec<Option<SetPath>>,
}

impl<'hir> Visit<'hir> for AtCollector<'_, 'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let Some(call) = enumerable_set_call(self.gcx, self.hir, self.bindings, expr)
            && call.name == SetOp::At
            && nth_argument(self.hir, call.function_id, call.args, call.index_arg, INDEX_PARAMETER)
                .and_then(bare_identifier)
                .is_some_and(|index| self.cadence.contains(&index))
        {
            self.iterated.push(call.set);
        }
        self.walk_expr(expr)
    }
}

/// Scans a straight-line body for EnumerableSet `remove` calls, with the span to report.
struct RemoveScanner<'a, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    bindings: &'a Bindings,
    out: &'a mut Vec<(Option<SetPath>, Span)>,
}

impl<'hir> Visit<'hir> for RemoveScanner<'_, 'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let Some(call) = enumerable_set_call(self.gcx, self.hir, self.bindings, expr)
            && call.name == SetOp::Remove
        {
            self.out.push((call.set, expr.span));
        }
        self.walk_expr(expr)
    }
}

/// The variables a statement list writes to, through the loops under it as well: assignments,
/// compound assignments, increments and decrements, wherever they sit.
fn collect_variables<'hir>(
    hir: &'hir Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
    out: &mut Vec<VariableId>,
) {
    struct Collector<'a, 'hir> {
        hir: &'hir Hir<'hir>,
        out: &'a mut Vec<VariableId>,
    }
    impl<'hir> Visit<'hir> for Collector<'_, 'hir> {
        type BreakValue = Infallible;
        fn hir(&self) -> &'hir Hir<'hir> {
            self.hir
        }
        fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
            let written = match &expr.kind {
                ExprKind::Assign(lhs, ..) => Some(*lhs),
                ExprKind::Unary(op, operand)
                    if matches!(
                        op.kind,
                        UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                    ) =>
                {
                    Some(*operand)
                }
                _ => None,
            };
            if let Some(written) = written
                && let ExprKind::Ident(resolutions) = &written.peel_parens().kind
            {
                for res in *resolutions {
                    if let Res::Item(ItemId::Variable(variable_id)) = res {
                        self.out.push(*variable_id);
                    }
                }
            }
            self.walk_expr(expr)
        }
    }
    let mut collector = Collector { hir, out };
    for stmt in stmts {
        let _ = collector.visit_stmt(stmt);
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum SetOp {
    At,
    Remove,
}

/// `at`'s index is its second parameter, wherever the call form puts the argument.
const INDEX_PARAMETER: usize = 1;

/// A resolved EnumerableSet call: which operation, on which set, and where its index sits. The
/// method form binds the set to the receiver, so its arguments start one position later than the
/// parameters they fill.
struct SetCall<'hir> {
    name: SetOp,
    function_id: FunctionId,
    args: &'hir CallArgs<'hir>,
    set: Option<SetPath>,
    index_arg: usize,
}

/// The EnumerableSet `at` or `remove` a call dispatches to. Resolving through the type checker
/// covers the `using for` method form, the library-qualified form and import aliases, and keeps
/// same-name functions from other libraries out.
fn enumerable_set_call<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    bindings: &Bindings,
    expr: &'hir Expr<'hir>,
) -> Option<SetCall<'hir>> {
    let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
    let ty = gcx.type_of_expr(callee.peel_parens().id)?;
    let TyKind::Fn(function_ty) = ty.kind else { return None };
    let function_id = function_ty.function_id?;
    let function = hir.function(function_id);
    let contract = hir.contract(function.contract?);
    if !contract.kind.is_library() || contract.name.as_str() != "EnumerableSet" {
        return None;
    }
    let name = match function.name?.as_str() {
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
    let set = set_expr.and_then(|expr| set_path(hir, expr, bindings, &mut Vec::new()));
    Some(SetCall { name, function_id, args, set, index_arg })
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
        ExprKind::Ident(resolutions) => {
            let variable_id = resolutions.iter().find_map(|res| match res {
                Res::Item(ItemId::Variable(variable_id)) => Some(*variable_id),
                _ => None,
            })?;
            if seen.contains(&variable_id) {
                return None;
            }
            seen.push(variable_id);
            let variable = hir.variable(variable_id);
            // A local `storage` reference is another name for the set its last binding gave it.
            // The walk records what stands where the loop is judged, so a rebinding past the
            // loop does not reach back into it; a reference declared inside the analyzed loop
            // has no entry and is bound by its initializer anew each turn, read under the same
            // bindings.
            if matches!(variable.kind, VarKind::Statement) {
                if let Some((_, binding)) =
                    bindings.iter().rev().find(|(bound, _)| *bound == variable_id)
                {
                    return binding.clone();
                }
                if let Some(initializer) = variable.initializer {
                    return set_path(hir, initializer, bindings, seen);
                }
                // Bound neither by the walk nor by an initializer, a tuple-destructured
                // component: nothing says which set it names, so it may name any of them.
                return None;
            }
            Some(SetPath { base: variable_id, steps: Vec::new() })
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

/// Whether two set operands can name the same set. Two paths that can be read name the same set
/// exactly when they are equal; a path that cannot be read may be either.
fn paths_alias(removed: &Option<SetPath>, iterated: &Option<SetPath>) -> bool {
    match (removed, iterated) {
        (Some(removed), Some(iterated)) => removed == iterated,
        _ => true,
    }
}

/// The argument at position `arg` of a positional call, or the one a named call binds to the
/// callee's parameter at position `parameter`. Named arguments come in source order, which is
/// neither the parameter order nor, in the method form, the argument order.
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
            named
                .iter()
                .find(|argument| argument.name.as_str() == name.as_str())
                .map(|argument| &argument.value)
        }
    }
}

/// Whether `receiver` is a value of one of the set struct types declared in a library (or
/// contract) named `EnumerableSet` (`AddressSet` / `UintSet` / `Bytes32Set`), which tells the
/// bound method form apart from the library-qualified form.
fn is_enumerable_set_value<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    receiver: &Expr<'_>,
) -> bool {
    let Some(ty) = gcx.type_of_expr(receiver.peel_parens().id) else { return false };
    let TyKind::Struct(id) = ty.peel_refs().kind else { return false };
    let Some(contract_id) = hir.strukt(id).contract else { return false };
    hir.contract(contract_id).name.as_str() == "EnumerableSet"
}
