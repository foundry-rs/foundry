use super::EnumerableLoopRemoval;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::primitives::branch_always_exits},
};
use alloy_primitives::U256;
use solar::{
    ast::{LitKind, UnOpKind},
    interface::{Span, Symbol},
    sema::{
        Gcx,
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

impl<'hir> LateLintPass<'hir> for EnumerableLoopRemoval {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        // EnumerableSet removal is swap-and-pop, so removing while iterating the same set at a
        // varying index skips elements or reads out-of-bounds indices. The safe patterns
        // (collect during the loop and remove in a later loop, drain at a literal index, iterate
        // a different set, remove and leave the loop) stay clean.
        if let Some(body) = &func.body {
            // A `storage` reference the function writes to again no longer names what it was
            // bound to, so its path cannot be read.
            let mut reassigned = Vec::new();
            collect_variables(hir, body.stmts, false, true, &mut reassigned);
            let mut finder =
                LoopFinder { gcx, hir, ctx, reassigned: &reassigned, emitted: HashSet::new() };
            // A `Block` has no dedicated visit hook: walk its statements directly.
            for stmt in body.stmts {
                let _ = finder.visit_stmt(stmt);
            }
        }
    }
}

/// Walks a function body and, for each loop, flags the EnumerableSet `remove` calls that corrupt
/// that loop's own iteration.
struct LoopFinder<'ctx, 's, 'c, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    ctx: &'ctx LintContext<'s, 'c>,
    reassigned: &'ctx [VariableId],
    // A loop nested in a flagged loop sees the same calls: dedupe emissions by span.
    emitted: HashSet<Span>,
}

impl<'hir> Visit<'hir> for LoopFinder<'_, '_, '_, 'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        // A `for` desugars to `Block { init; Loop(For) }`; its index lives partly in the init.
        // A nested loop may reassign that index without owning it, so `at` at it still walks
        // this loop; the index is passed down as the one a nested loop cannot claim.
        if let StmtKind::Block(block) = &stmt.kind
            && let Some(last) = block.stmts.last()
            && let StmtKind::Loop(body, LoopSource::For) = &last.kind
        {
            let init = &block.stmts[..block.stmts.len() - 1];
            let index = loop_own_index(self.hir, init, body.stmts);
            self.analyze_loop(body.stmts, &index);
            // Walk the init and the loop body for nested loops, but do not re-analyze this loop
            // as a bare one through the `Loop` arm below.
            for stmt in init {
                let _ = self.visit_stmt(stmt);
            }
            for stmt in body.stmts {
                let _ = self.visit_stmt(stmt);
            }
            return ControlFlow::Continue(());
        }
        // A bare loop is a `while`, a `do-while`, or a `for` with no init: its index is
        // reassigned in the body, read from the condition.
        if let StmtKind::Loop(body, _) = &stmt.kind {
            let index = loop_own_index(self.hir, &[], body.stmts);
            self.analyze_loop(body.stmts, &index);
        }
        self.walk_stmt(stmt)
    }
}

/// The variables a loop advances to pace its own iteration, its cadence. A variable qualifies
/// when the loop reassigns it anywhere in its body, nested loops included, and it also names the
/// loop's own turn: reassigned in the loop's body outside any nested loop (a `for`'s `i++`, a
/// `while`'s counter), or tested in one of the loop's conditions, its guard or an `if (...)
/// break`. A nested loop physically holding the read and the write does not move the cadence: an
/// `at(i)` at the loop's counter still walks the loop. A nested loop's own cursor, reassigned and
/// tested only inside it, names neither the enclosing loop's direct writes nor its conditions, so
/// it stays the nested loop's even when declared outside the enclosing body, a function parameter
/// or a hoisted local included.
fn loop_own_index<'hir>(
    hir: &'hir Hir<'hir>,
    init: &'hir [Stmt<'hir>],
    body: &'hir [Stmt<'hir>],
) -> Vec<VariableId> {
    let mut advanced = Vec::new();
    collect_variables(hir, init, false, true, &mut advanced);
    collect_variables(hir, body, false, true, &mut advanced);
    let mut cadence = Vec::new();
    collect_cadence_hints(hir, init, &mut cadence);
    collect_cadence_hints(hir, body, &mut cadence);
    advanced.retain(|variable| cadence.contains(variable));
    advanced
}

/// The variables that could name a loop's cadence, read from its own body without descending
/// into nested loops: the ones it reassigns directly, and the ones any of its conditions test
/// (its guard, lowered to the first/last `if`, and any `if (...) break`). A nested loop's cursor,
/// advanced and tested inside that loop, appears in neither.
fn collect_cadence_hints<'hir>(
    hir: &'hir Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
    out: &mut Vec<VariableId>,
) {
    struct Hints<'a, 'hir> {
        hir: &'hir Hir<'hir>,
        out: &'a mut Vec<VariableId>,
    }
    impl<'hir> Visit<'hir> for Hints<'_, 'hir> {
        type BreakValue = Infallible;
        fn hir(&self) -> &'hir Hir<'hir> {
            self.hir
        }
        fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
            // A nested loop paces its own cursor; its statements are not this loop's cadence.
            if matches!(stmt.kind, StmtKind::Loop(..)) {
                return ControlFlow::Continue(());
            }
            // Only a termination guard names the loop's cadence: the desugared
            // `if (cond) { body } else break`, or a user `if (...) break/return`, whose branch
            // leaves the loop. A plain `if` testing a variable, `if (j == len) { ... }`, guards
            // nothing about the iteration and does not make its operands the cadence.
            if let StmtKind::If(cond, then, else_) = &stmt.kind
                && (exits_loop(then, false)
                    || else_.is_some_and(|branch| exits_loop(branch, false)))
            {
                push_named_variables(self.hir, cond, self.out);
            }
            self.walk_stmt(stmt)
        }
        fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
            // Only a reassignment that advances the variable from its old value paces the loop:
            // `i++`, `i += n`, `i = i + 1` read the previous value and progress; `i = 0` resets
            // it each turn and does not, so a cursor a nested loop reseeds is not this loop's
            // cadence.
            let advanced = match &expr.kind {
                ExprKind::Unary(op, operand)
                    if matches!(
                        op.kind,
                        UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                    ) =>
                {
                    Some(*operand)
                }
                // A compound assignment (`+=`, `-=`, ...) reads the old value.
                ExprKind::Assign(lhs, Some(_), _) => Some(*lhs),
                // A plain assignment progresses only when it reads the target itself.
                ExprKind::Assign(lhs, None, rhs) if mentions_target(self.hir, lhs, rhs) => {
                    Some(*lhs)
                }
                _ => None,
            };
            if let Some(advanced) = advanced
                && let ExprKind::Ident(resolutions) = &advanced.peel_parens().kind
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
    let mut hints = Hints { hir, out };
    for stmt in stmts {
        let _ = hints.visit_stmt(stmt);
    }
}

/// Whether the right-hand side of an assignment reads a variable its target names, marking the
/// assignment a progression (`i = i + 1`) rather than a reset (`i = 0`).
fn mentions_target<'hir>(
    hir: &'hir Hir<'hir>,
    target: &'hir Expr<'hir>,
    value: &'hir Expr<'hir>,
) -> bool {
    let mut targets = Vec::new();
    if let ExprKind::Ident(resolutions) = &target.peel_parens().kind {
        for res in *resolutions {
            if let Res::Item(ItemId::Variable(variable_id)) = res {
                targets.push(*variable_id);
            }
        }
    }
    if targets.is_empty() {
        return false;
    }
    let mut read = Vec::new();
    push_named_variables(hir, value, &mut read);
    targets.iter().any(|variable| read.contains(variable))
}

/// Every variable an expression names, wherever it sits in the expression tree.
fn push_named_variables<'hir>(
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    out: &mut Vec<VariableId>,
) {
    struct Named<'a, 'hir> {
        hir: &'hir Hir<'hir>,
        out: &'a mut Vec<VariableId>,
    }
    impl<'hir> Visit<'hir> for Named<'_, 'hir> {
        type BreakValue = Infallible;
        fn hir(&self) -> &'hir Hir<'hir> {
            self.hir
        }
        fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
            if let ExprKind::Ident(resolutions) = &expr.kind {
                for res in *resolutions {
                    if let Res::Item(ItemId::Variable(variable_id)) = res {
                        self.out.push(*variable_id);
                    }
                }
            }
            self.walk_expr(expr)
        }
    }
    let mut named = Named { hir, out };
    let _ = named.visit_expr(expr);
}

impl<'hir> LoopFinder<'_, '_, '_, 'hir> {
    /// Flags the removals in `body` that corrupt this loop's iteration. `index` is the loop's own
    /// index, which a nested loop may write without taking it over.
    fn analyze_loop(&mut self, body: &'hir [Stmt<'hir>], index: &[VariableId]) {
        // Which sets this loop iterates, and which it removes from.
        let mut ats = AtCollector {
            gcx: self.gcx,
            hir: self.hir,
            reassigned: self.reassigned,
            outer_index: index,
            iterated: Vec::new(),
            owned_by_inner: Vec::new(),
        };
        for stmt in body {
            let _ = ats.visit_stmt(stmt);
        }
        // Nested loops included: their removals mutate the set this loop is walking, unless
        // control leaves first.
        let mut removes = Vec::new();
        collect_removes(self.gcx, self.hir, self.reassigned, body, false, false, &mut removes);
        for (removed, span) in removes {
            let corrupts = ats.iterated.iter().any(|iterated| paths_alias(&removed, iterated));
            if corrupts && self.emitted.insert(span) {
                self.ctx.emit(&ENUMERABLE_LOOP_REMOVAL, span);
            }
        }
    }
}

/// Collects the sets a loop iterates with `at` at an index that advances with it. An `at` read
/// inside a nested loop belongs to that loop when its index is the nested loop's own, and to
/// this loop when it is this loop's index: `holders.at(i)` walks the `i` loop wherever it sits.
/// `owned_by_inner` holds the variables the nested loops entered so far declare or write; a read
/// at one of them is the nested loop's, unless it is `outer_index`, this loop's own index, which
/// a nested loop may reassign without taking over.
struct AtCollector<'a, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    reassigned: &'a [VariableId],
    outer_index: &'a [VariableId],
    iterated: Vec<Option<SetPath>>,
    owned_by_inner: Vec<VariableId>,
}

impl<'hir> Visit<'hir> for AtCollector<'_, 'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        if let StmtKind::Loop(block, _) = &stmt.kind {
            // A nested loop's own reads are indexed by what it declares or advances. A `while`
            // advances an index declared outside it, so its writes count too, not only its
            // declarations; the outer loop's index is withheld from this below.
            let depth = self.owned_by_inner.len();
            collect_variables(self.hir, block.stmts, true, true, &mut self.owned_by_inner);
            let result = self.walk_stmt(stmt);
            self.owned_by_inner.truncate(depth);
            return result;
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let Some(call) = enumerable_set_call(self.gcx, self.hir, self.reassigned, expr)
            && call.name == SetOp::At
            && nth_argument(self.hir, call.function_id, call.args, call.index_arg, INDEX_PARAMETER)
                .is_none_or(|index| self.index_advances_this_loop(index))
        {
            self.iterated.push(call.set);
        }
        self.walk_expr(expr)
    }
}

impl<'hir> AtCollector<'_, 'hir> {
    /// Whether an `at` read at `index` advances this loop's own iteration. A fixed index does
    /// not: the swap-and-pop refills that position, so a drain like `remove(at(0))` stays clean.
    /// An index a nested loop owns is that loop's, not this one's, unless it is this loop's own
    /// index that the nested loop merely reassigns. An index that cannot be read may be anything,
    /// so it is assumed to advance.
    fn index_advances_this_loop(&self, index: &'hir Expr<'hir>) -> bool {
        if is_fixed_index(self.hir, index) {
            return false;
        }
        !mentions_any(self.hir, index, &self.owned_by_inner)
            || mentions_any(self.hir, index, self.outer_index)
    }
}

/// The variables a statement list touches, through the loops under it as well. `declarations`
/// collects the ones it declares, `assignments` the ones it writes to; a caller asks for the
/// kind it needs, since a loop owns the locals it declares but shares a variable it only
/// reassigns with whoever declared it.
fn collect_variables<'hir>(
    hir: &'hir Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
    declarations: bool,
    assignments: bool,
    out: &mut Vec<VariableId>,
) {
    struct Collector<'a, 'hir> {
        hir: &'hir Hir<'hir>,
        declarations: bool,
        assignments: bool,
        out: &'a mut Vec<VariableId>,
    }
    impl<'hir> Visit<'hir> for Collector<'_, 'hir> {
        type BreakValue = Infallible;
        fn hir(&self) -> &'hir Hir<'hir> {
            self.hir
        }
        fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
            if self.declarations {
                match &stmt.kind {
                    StmtKind::DeclSingle(variable_id) => self.out.push(*variable_id),
                    StmtKind::DeclMulti(variables, _) => {
                        self.out.extend(variables.iter().flatten().copied());
                    }
                    _ => {}
                }
            }
            self.walk_stmt(stmt)
        }
        fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
            if !self.assignments {
                return self.walk_expr(expr);
            }
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
    let mut collector = Collector { hir, declarations, assignments, out };
    for stmt in stmts {
        let _ = collector.visit_stmt(stmt);
    }
}

/// Whether `expr` names any of `variables`.
fn mentions_any<'hir>(
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    variables: &[VariableId],
) -> bool {
    struct Finder<'a, 'hir> {
        hir: &'hir Hir<'hir>,
        variables: &'a [VariableId],
        found: bool,
    }
    impl<'hir> Visit<'hir> for Finder<'_, 'hir> {
        type BreakValue = Infallible;
        fn hir(&self) -> &'hir Hir<'hir> {
            self.hir
        }
        fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
            if let ExprKind::Ident(resolutions) = &expr.kind
                && resolutions.iter().any(|res| {
                    matches!(res, Res::Item(ItemId::Variable(id)) if self.variables.contains(id))
                })
            {
                self.found = true;
            }
            self.walk_expr(expr)
        }
    }
    let mut finder = Finder { hir, variables, found: false };
    let _ = finder.visit_expr(expr);
    finder.found
}

/// Collects the sets a statement list removes from, with the span to report. `tail_exits` says
/// whether control always leaves the loop once the list is done, in which case a removal in it is
/// the last thing the loop does and corrupts nothing.
fn collect_removes<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    reassigned: &[VariableId],
    stmts: &'hir [Stmt<'hir>],
    tail_exits: bool,
    nested: bool,
    out: &mut Vec<(Option<SetPath>, Span)>,
) {
    // Whether control leaves the loop after the statement at each position, either through a
    // later statement of this list or because the list itself is followed by an exit.
    let mut after = vec![tail_exits; stmts.len()];
    for index in (0..stmts.len()).rev() {
        after[index] = match stmts.get(index + 1) {
            Some(next) => after[index + 1] || exits_loop(next, nested),
            None => tail_exits,
        };
    }

    for (index, stmt) in stmts.iter().enumerate() {
        let leaves = after[index] || exits_loop(stmt, nested);
        match &stmt.kind {
            hir::StmtKind::Block(block) | hir::StmtKind::UncheckedBlock(block) => {
                collect_removes(gcx, hir, reassigned, block.stmts, leaves, nested, out);
            }
            hir::StmtKind::If(cond, then, else_) => {
                collect_removes_in_expr(gcx, hir, reassigned, cond, leaves, out);
                collect_removes(
                    gcx,
                    hir,
                    reassigned,
                    std::slice::from_ref(*then),
                    leaves,
                    nested,
                    out,
                );
                if let Some(else_) = else_ {
                    collect_removes(
                        gcx,
                        hir,
                        reassigned,
                        std::slice::from_ref(*else_),
                        leaves,
                        nested,
                        out,
                    );
                }
            }
            // Only leaving the function leaves the analyzed loop from inside a nested one.
            hir::StmtKind::Loop(block, _) => {
                collect_removes(gcx, hir, reassigned, block.stmts, false, true, out)
            }
            _ => {
                let mut scan = RemoveScanner { gcx, hir, reassigned, leaves, out };
                let _ = scan.visit_stmt(stmt);
            }
        }
    }
}

fn collect_removes_in_expr<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    reassigned: &[VariableId],
    expr: &'hir Expr<'hir>,
    leaves: bool,
    out: &mut Vec<(Option<SetPath>, Span)>,
) {
    let mut scan = RemoveScanner { gcx, hir, reassigned, leaves, out };
    let _ = scan.visit_expr(expr);
}

/// Records the `remove` calls of a statement that holds no further control flow of its own.
struct RemoveScanner<'a, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    reassigned: &'a [VariableId],
    leaves: bool,
    out: &'a mut Vec<(Option<SetPath>, Span)>,
}

impl<'hir> Visit<'hir> for RemoveScanner<'_, 'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if !self.leaves
            && let Some(call) = enumerable_set_call(self.gcx, self.hir, self.reassigned, expr)
            && call.name == SetOp::Remove
        {
            self.out.push((call.set, expr.span));
        }
        self.walk_expr(expr)
    }
}

/// Whether executing `stmt` always leaves the loop being analyzed. `nested` says the statement
/// sits under a loop of its own, where a `break` only ends that one and the analyzed loop comes
/// round again; leaving the function still leaves both.
fn exits_loop(stmt: &Stmt<'_>, nested: bool) -> bool {
    match &stmt.kind {
        StmtKind::Break => !nested,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.iter().any(|stmt| exits_loop(stmt, nested))
        }
        StmtKind::If(_, then, Some(else_)) => exits_loop(then, nested) && exits_loop(else_, nested),
        // Every path out of a `try` leaves the loop only if the success clause and each `catch`
        // do: the statement itself falls through otherwise.
        StmtKind::Try(try_) => try_
            .clauses
            .iter()
            .all(|clause| clause.block.stmts.iter().any(|stmt| exits_loop(stmt, nested))),
        // `return`, `revert`, `revert(...)`, `require(false)`: they leave the function, hence
        // every loop. A nested loop and a `continue` leave nothing.
        _ => branch_always_exits(stmt),
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
    reassigned: &[VariableId],
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
    let set = set_expr.and_then(|expr| set_path(hir, expr, reassigned, &mut Vec::new()));
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

/// The path a set expression names, or `None` when it cannot be read: an index that varies, a
/// call result, anything the analysis would have to evaluate.
fn set_path(
    hir: &Hir<'_>,
    expr: &Expr<'_>,
    reassigned: &[VariableId],
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
            // A local `storage` reference is another name for the set it was bound to, unless the
            // function binds it again, in which case what it names cannot be read here.
            match (variable.kind, variable.initializer) {
                (VarKind::Statement, _) if reassigned.contains(&variable_id) => None,
                (VarKind::Statement, Some(initializer)) => {
                    set_path(hir, initializer, reassigned, seen)
                }
                _ => Some(SetPath { base: variable_id, steps: Vec::new() }),
            }
        }
        ExprKind::Member(base, field) => {
            let mut path = set_path(hir, base, reassigned, seen)?;
            path.steps.push(Step::Field(field.name));
            Some(path)
        }
        ExprKind::Index(base, Some(index)) => {
            let ExprKind::Lit(lit) = &index.peel_parens().kind else { return None };
            let LitKind::Number(key) = &lit.kind else { return None };
            let mut path = set_path(hir, base, reassigned, seen)?;
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

/// Whether the index holds a value fixed at compile time, so the read does not advance the
/// iteration: a `remove(at(0))` drains position zero repeatedly, which swap-and-pop keeps
/// refilling. A number literal, a value-preserving cast of one (`uint256(0)`), or a `constant`
/// whose initializer is itself fixed all qualify. The check stays conservative: only a
/// provably fixed index is exempted, so nothing that may vary is mistaken for a drain. Constant
/// arithmetic (`0 + 0`) is not folded and stays treated as varying, on the safe side.
fn is_fixed_index(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => matches!(lit.kind, LitKind::Number(_)),
        // A conversion `T(x)` keeps `x`'s value when it is a single-argument cast.
        ExprKind::Call(callee, args, _)
            if matches!(callee.peel_parens().kind, ExprKind::Type(..)) =>
        {
            let mut operands = args.exprs();
            match (operands.next(), operands.next()) {
                (Some(inner), None) => is_fixed_index(hir, inner),
                _ => false,
            }
        }
        // A `constant` is worth the fixed value it was initialized with.
        ExprKind::Ident(resolutions) => resolutions.iter().any(|res| match res {
            Res::Item(ItemId::Variable(variable_id)) => {
                let variable = hir.variable(*variable_id);
                variable.is_constant()
                    && variable
                        .initializer
                        .is_some_and(|initializer| is_fixed_index(hir, initializer))
            }
            _ => false,
        }),
        _ => false,
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
