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
        // EnumerableSet removal is swap-and-pop, so removing while iterating the same set at an
        // index the loop advances skips elements or reads out-of-bounds indices. The safe
        // patterns (collect during the loop and remove in a later loop, drain at an index the
        // loop never moves, iterate a different set, remove and leave the loop) stay clean.
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
            self.enter_loop(init, body.stmts);
            return;
        }
        match &stmt.kind {
            // A bare block runs on the straight line: what it binds stays bound past it.
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.walk_body(block.stmts);
            }
            // A bare loop is a `while`, a `do-while`, or a `for` with no init.
            StmtKind::Loop(body, _) => self.enter_loop(&[], body.stmts),
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
    fn enter_loop(&mut self, init: &'hir [Stmt<'hir>], body: &'hir [Stmt<'hir>]) {
        self.poison_writes(init);
        self.poison_writes(body);
        let cadence = cadence_carriers(self.hir, init, body, loop_own_index(self.hir, init, body));
        self.analyze_loop(body, &cadence);
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

    /// Flags the removals in `body` that corrupt this loop's iteration. `cadence` is the loop's
    /// own moving cadence, see [`loop_own_index`].
    fn analyze_loop(&mut self, body: &'hir [Stmt<'hir>], cadence: &[VariableId]) {
        // Which sets this loop iterates, and which it removes from.
        let mut ats = AtCollector {
            gcx: self.gcx,
            hir: self.hir,
            bindings: &self.bindings,
            cadence,
            iterated: Vec::new(),
        };
        for stmt in body {
            let _ = ats.visit_stmt(stmt);
        }
        // Nested loops included: their removals mutate the set this loop is walking, unless
        // control leaves first.
        let mut removes = Vec::new();
        collect_removes(self.gcx, self.hir, &self.bindings, body, false, false, &mut removes);
        for (removed, span) in removes {
            let corrupts = ats.iterated.iter().any(|iterated| paths_alias(&removed, iterated));
            if corrupts && self.emitted.insert(span) {
                self.ctx.emit(&ENUMERABLE_LOOP_REMOVAL, span);
            }
        }
    }
}

/// How a loop's own statement names a variable as its cadence: a progression stepping it
/// downward, a progression stepping it any other way, or a mention in a termination guard,
/// which says nothing about direction.
#[derive(PartialEq, Eq, Clone, Copy)]
enum CadenceHint {
    Descends,
    Advances,
    Guards,
}

/// The variables that pace a loop's iteration forward, its moving cadence. A variable qualifies
/// when the loop advances it anywhere in its body, nested loops included, and it also names the
/// loop's own turn: progressed in the loop's body outside any nested loop (a `for`'s `i++`, a
/// `while`'s counter), or tested in one of the loop's termination guards. A nested loop's own
/// cursor, progressed and tested only inside it, names neither, so it stays the nested loop's
/// even when declared outside the enclosing body, a function parameter or a hoisted local
/// included.
///
/// A cadence only ever stepped downward is left out: swap-and-pop moves the tail element into
/// the slot being emptied, and a walk going down never returns to a slot at or above the one it
/// just read, so `for (uint256 i = set.length(); i > 0; i--) set.remove(set.at(i - 1))` drains
/// without skipping anything. A cadence stepped upward, or moved in a shape this reading cannot
/// direct, walks into the swapped-in tail and is kept.
fn loop_own_index<'hir>(
    hir: &'hir Hir<'hir>,
    init: &'hir [Stmt<'hir>],
    body: &'hir [Stmt<'hir>],
) -> Vec<VariableId> {
    let mut advanced = Vec::new();
    collect_variables(hir, init, &mut advanced);
    collect_variables(hir, body, &mut advanced);
    let mut hints = Vec::new();
    collect_cadence_hints(hir, init, &mut hints);
    collect_cadence_hints(hir, body, &mut hints);
    let mut moving = Vec::new();
    // Keep every advanced variable a hint names, unless each progression seen steps it
    // downward; a variable named only by a guard has no direction to read and is kept.
    for (variable, _) in &hints {
        if !advanced.contains(variable) || moving.contains(variable) {
            continue;
        }
        let mut progressions = 0usize;
        let mut downward = 0usize;
        // Judged over every hint naming this variable: one upward step anywhere breaks the
        // downward walk.
        for (named, hint) in &hints {
            if named != variable {
                continue;
            }
            match hint {
                CadenceHint::Descends => {
                    progressions += 1;
                    downward += 1;
                }
                CadenceHint::Advances => progressions += 1,
                CadenceHint::Guards => {}
            }
        }
        // No progression read here means the direction is unknown, so the variable is kept.
        if progressions == 0 || downward < progressions {
            moving.push(*variable);
        }
    }
    moving
}

/// The moving cadence plus every variable the loop binds from it: `uint256 idx = i;` hands the
/// cadence to `idx`, so `at(idx)` walks the loop exactly as `at(i)` does. Copies are read
/// flow-insensitively over the whole body, a copy taken anywhere possibly holding the cadence
/// when the read runs, so an index derived from the cadence reports even when its arithmetic
/// happens to walk somewhere safe.
fn cadence_carriers<'hir>(
    hir: &'hir Hir<'hir>,
    init: &'hir [Stmt<'hir>],
    body: &'hir [Stmt<'hir>],
    mut carriers: Vec<VariableId>,
) -> Vec<VariableId> {
    if carriers.is_empty() {
        return carriers;
    }
    let mut copies = Vec::new();
    collect_copies(hir, init, &mut copies);
    collect_copies(hir, body, &mut copies);
    let mut changed = true;
    // A copy of a copy carries too: iterate to closure, each pass adding at least one carrier.
    while changed {
        changed = false;
        for (target, value) in &copies {
            if !carriers.contains(target) && mentions_any(hir, value, &carriers) {
                carriers.push(*target);
                changed = true;
            }
        }
    }
    carriers
}

/// Every variable a statement list binds paired with what it binds it to: a declaration's
/// initializer, or the right-hand side of an assignment to a bare variable, compound forms
/// included.
fn collect_copies<'hir>(
    hir: &'hir Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
    out: &mut Vec<(VariableId, &'hir Expr<'hir>)>,
) {
    struct Copies<'a, 'hir> {
        hir: &'hir Hir<'hir>,
        out: &'a mut Vec<(VariableId, &'hir Expr<'hir>)>,
    }
    impl<'hir> Visit<'hir> for Copies<'_, 'hir> {
        type BreakValue = Infallible;
        fn hir(&self) -> &'hir Hir<'hir> {
            self.hir
        }
        fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
            if let StmtKind::DeclSingle(variable_id) = &stmt.kind
                && let Some(initializer) = self.hir.variable(*variable_id).initializer
            {
                self.out.push((*variable_id, initializer));
            }
            self.walk_stmt(stmt)
        }
        fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
            if let ExprKind::Assign(target, _, value) = &expr.kind
                && let ExprKind::Ident(resolutions) = &target.peel_parens().kind
            {
                for res in *resolutions {
                    if let Res::Item(ItemId::Variable(variable_id)) = res {
                        self.out.push((*variable_id, value));
                    }
                }
            }
            self.walk_expr(expr)
        }
    }
    let mut copies = Copies { hir, out };
    for stmt in stmts {
        let _ = copies.visit_stmt(stmt);
    }
}

/// The variables that could name a loop's cadence, read from its own body without descending
/// into nested loops: the ones it progresses directly, each with its step's direction, and the
/// ones any of its conditions test (its guard, lowered to the first/last `if`, and any
/// `if (...) break`). A nested loop's cursor, advanced and tested inside that loop, appears in
/// neither.
fn collect_cadence_hints<'hir>(
    hir: &'hir Hir<'hir>,
    stmts: &'hir [Stmt<'hir>],
    out: &mut Vec<(VariableId, CadenceHint)>,
) {
    struct Hints<'a, 'hir> {
        hir: &'hir Hir<'hir>,
        out: &'a mut Vec<(VariableId, CadenceHint)>,
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
                let mut named = Vec::new();
                push_named_variables(self.hir, cond, &mut named);
                for variable_id in named {
                    self.out.push((variable_id, CadenceHint::Guards));
                }
            }
            self.walk_stmt(stmt)
        }
        fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
            // Only a reassignment that advances the variable from its old value paces the loop:
            // `i++`, `i += n`, `i = i + 1` read the previous value and progress; `i = 0` resets
            // it each turn and does not, so a cursor a nested loop reseeds is not this loop's
            // cadence. Each progression carries whether it steps downward: a decrement, a `-=`,
            // or an assignment subtracting from the variable itself.
            let progressed = match &expr.kind {
                ExprKind::Unary(op, operand)
                    if matches!(
                        op.kind,
                        UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                    ) =>
                {
                    Some((*operand, matches!(op.kind, UnOpKind::PreDec | UnOpKind::PostDec)))
                }
                // A compound assignment (`+=`, `-=`, ...) reads the old value.
                ExprKind::Assign(lhs, Some(op), _) => Some((*lhs, op.kind == hir::BinOpKind::Sub)),
                // A plain assignment progresses only when it reads the target itself.
                ExprKind::Assign(lhs, None, rhs) if mentions_target(self.hir, lhs, rhs) => {
                    Some((*lhs, subtracts_from_target(self.hir, lhs, rhs)))
                }
                _ => None,
            };
            if let Some((progressed, descends)) = progressed
                && let ExprKind::Ident(resolutions) = &progressed.peel_parens().kind
            {
                let hint = if descends { CadenceHint::Descends } else { CadenceHint::Advances };
                for res in *resolutions {
                    if let Res::Item(ItemId::Variable(variable_id)) = res {
                        self.out.push((*variable_id, hint));
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

/// Whether a plain assignment steps its target downward: the right-hand side subtracts from the
/// target itself, `i = i - 1`. Anything else that reads the target, `i = i + 1` or `i = j - i`,
/// is not a downward step.
fn subtracts_from_target<'hir>(
    hir: &'hir Hir<'hir>,
    target: &'hir Expr<'hir>,
    value: &'hir Expr<'hir>,
) -> bool {
    let ExprKind::Binary(minuend, op, _) = &value.peel_parens().kind else { return false };
    op.kind == hir::BinOpKind::Sub && mentions_target(hir, target, minuend)
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

/// Collects the sets a loop iterates with `at` at an index its own moving cadence paces. An
/// `at` whose index never names that cadence reads a slot the loop does not advance over,
/// wherever the read sits: a stationary cursor drains the position swap-and-pop keeps
/// refilling, a nested loop's cursor walks that loop, and a literal or a `constant` never moves
/// at all. An index that cannot be read may be anything, so it is assumed to walk the loop.
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
                .is_none_or(|index| mentions_any(self.hir, index, self.cadence))
        {
            self.iterated.push(call.set);
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
    bindings: &Bindings,
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
                collect_removes(gcx, hir, bindings, block.stmts, leaves, nested, out);
            }
            hir::StmtKind::If(cond, then, else_) => {
                // `remove` answers true exactly when it took the value out, so a removal that
                // is itself the whole condition corrupts nothing when the mutating answer
                // always leaves the loop: `if (set.remove(value)) break` shrinks the set only
                // on the exiting path, and the continuing path left it untouched. Any other
                // operand riding along (`remove(v) && flag`) can steer execution past the exit
                // after the set shrank, so only the bare call is read this way.
                let removal_implies_exit = exits_loop(then, nested)
                    && enumerable_set_call(gcx, hir, bindings, cond.peel_parens())
                        .is_some_and(|call| call.name == SetOp::Remove);
                let mut conditional = Vec::new();
                collect_removes_in_expr(gcx, hir, bindings, cond, leaves, &mut conditional);
                if removal_implies_exit {
                    let exempted = cond.peel_parens().span;
                    conditional.retain(|(_, span)| *span != exempted);
                }
                out.append(&mut conditional);
                collect_removes(
                    gcx,
                    hir,
                    bindings,
                    std::slice::from_ref(*then),
                    leaves,
                    nested,
                    out,
                );
                if let Some(else_) = else_ {
                    collect_removes(
                        gcx,
                        hir,
                        bindings,
                        std::slice::from_ref(*else_),
                        leaves,
                        nested,
                        out,
                    );
                }
            }
            // Each clause runs on its own path: a removal in one is followed only by that
            // clause's trailing statements, so a success clause may remove and return while a
            // `catch` that falls through holds no removal at all. The tried call itself runs
            // before any clause is dispatched.
            hir::StmtKind::Try(try_) => {
                collect_removes_in_expr(gcx, hir, bindings, &try_.expr, leaves, out);
                for clause in try_.clauses {
                    collect_removes(gcx, hir, bindings, clause.block.stmts, leaves, nested, out);
                }
            }
            // Only leaving the function leaves the analyzed loop from inside a nested one.
            hir::StmtKind::Loop(block, _) => {
                collect_removes(gcx, hir, bindings, block.stmts, false, true, out)
            }
            _ => {
                let mut scan = RemoveScanner { gcx, hir, bindings, leaves, out };
                let _ = scan.visit_stmt(stmt);
            }
        }
    }
}

fn collect_removes_in_expr<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    bindings: &Bindings,
    expr: &'hir Expr<'hir>,
    leaves: bool,
    out: &mut Vec<(Option<SetPath>, Span)>,
) {
    let mut scan = RemoveScanner { gcx, hir, bindings, leaves, out };
    let _ = scan.visit_expr(expr);
}

/// Records the `remove` calls of a statement that holds no further control flow of its own.
struct RemoveScanner<'a, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    bindings: &'a Bindings,
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
            && let Some(call) = enumerable_set_call(self.gcx, self.hir, self.bindings, expr)
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
