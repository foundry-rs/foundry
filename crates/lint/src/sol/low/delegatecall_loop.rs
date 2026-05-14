use super::DelegatecallLoop;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{StateMutability, Visibility},
    interface::{Span, kw},
    sema::hir::{self, Expr, ExprKind, FunctionId, ItemId, Res, Stmt, StmtKind, Visit},
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    DELEGATECALL_LOOP,
    Severity::Low,
    "delegatecall-loop",
    "payable functions should not use `delegatecall` inside a loop"
);

impl<'hir> LateLintPass<'hir> for DelegatecallLoop {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if !is_payable_entry_point(func) {
            return;
        }

        let Some(body) = func.body else { return };

        // Start from payable entry points so helper functions are only linted when reachable
        // through a call path that can preserve a non-zero `msg.value`.
        let mut checker = DelegatecallLoopChecker {
            ctx,
            hir,
            loop_depth: 0,
            emitted: HashSet::new(),
            modifier_stack: Vec::new(),
            call_stack: Vec::new(),
        };
        checker.visit_modifier_chain(func.modifiers, 0, body);
    }
}

fn is_payable_entry_point(func: &hir::Function<'_>) -> bool {
    // Match Slither's scope: implemented payable entry points, not internal helpers.
    func.state_mutability == StateMutability::Payable
        && matches!(func.visibility, Visibility::Public | Visibility::External)
}

struct DelegatecallLoopChecker<'a, 's, 'hir> {
    ctx: &'a LintContext<'s, 'a>,
    hir: &'hir hir::Hir<'hir>,
    loop_depth: usize,
    emitted: HashSet<Span>,
    modifier_stack: Vec<FunctionId>,
    call_stack: Vec<FunctionId>,
}

impl<'a, 's, 'hir> DelegatecallLoopChecker<'a, 's, 'hir> {
    fn visit_modifier_chain(
        &mut self,
        modifiers: &'hir [hir::Modifier<'hir>],
        index: usize,
        body: hir::Block<'hir>,
    ) {
        // Solidity modifiers wrap the function body. Walk each modifier in order and replace
        // the `_` placeholder with the remaining modifier chain and final function body.
        let Some(modifier) = modifiers.get(index) else {
            self.visit_block_stmts(body);
            return;
        };

        let _ = self.visit_call_args(&modifier.args);

        let Some(modifier_id) = modifier.id.as_function() else {
            self.visit_modifier_chain(modifiers, index + 1, body);
            return;
        };

        if self.modifier_stack.contains(&modifier_id) {
            self.visit_modifier_chain(modifiers, index + 1, body);
            return;
        }

        let modifier_func = self.hir.function(modifier_id);
        let Some(modifier_body) = modifier_func.body else {
            self.visit_modifier_chain(modifiers, index + 1, body);
            return;
        };

        self.modifier_stack.push(modifier_id);
        self.visit_block_with_placeholder(modifier_body, Some((modifiers, index + 1, body)));
        self.modifier_stack.pop();
    }

    fn visit_block_stmts(&mut self, block: hir::Block<'hir>) {
        for stmt in block.stmts {
            let _ = self.visit_stmt(stmt);
        }
    }

    fn visit_block_with_placeholder(
        &mut self,
        block: hir::Block<'hir>,
        placeholder: Option<(&'hir [hir::Modifier<'hir>], usize, hir::Block<'hir>)>,
    ) {
        for stmt in block.stmts {
            self.visit_stmt_with_placeholder(stmt, placeholder);
        }
    }

    fn visit_stmt_with_placeholder(
        &mut self,
        stmt: &'hir Stmt<'hir>,
        placeholder: Option<(&'hir [hir::Modifier<'hir>], usize, hir::Block<'hir>)>,
    ) {
        if matches!(stmt.kind, StmtKind::Placeholder) {
            if let Some((modifiers, index, body)) = placeholder {
                self.visit_modifier_chain(modifiers, index, body);
            }
        } else {
            let _ = self.visit_stmt(stmt);
        }
    }
}

impl<'hir> Visit<'hir> for DelegatecallLoopChecker<'_, '_, 'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match stmt.kind {
            // HIR lowers `for` update expressions into the loop body, so this covers loop
            // bodies and `for (...; ...; next)` delegatecalls with the same scope tracking.
            StmtKind::Loop(block, _) => self.visit_loop_block(block),
            _ => self.walk_stmt(stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if self.loop_depth > 0 && is_delegatecall(expr) && self.emitted.insert(expr.span) {
            self.ctx.emit(&DELEGATECALL_LOOP, expr.span);
        }

        let result = self.walk_expr(expr);
        if result.is_break() {
            return result;
        }

        // Internal helper calls inherit the current loop context. This catches cases where the
        // loop body calls a helper that performs the delegatecall.
        if let ExprKind::Call(callee, _, _) = &expr.kind {
            for func_id in resolved_function_ids(callee) {
                self.visit_internal_call(func_id);
            }
        }

        ControlFlow::Continue(())
    }
}

impl<'hir> DelegatecallLoopChecker<'_, '_, 'hir> {
    fn visit_loop_block(&mut self, block: hir::Block<'hir>) -> ControlFlow<()> {
        // Keep loop state as traversal context rather than as a property of a single function,
        // so delegatecalls inside internal helpers reached from a loop are also reported.
        self.loop_depth += 1;
        self.visit_block_stmts(block);
        self.loop_depth -= 1;
        ControlFlow::Continue(())
    }

    fn visit_internal_call(&mut self, func_id: FunctionId) {
        // Avoid infinite recursion on cyclic internal call graphs while still following each
        // acyclic helper path from the payable entry point.
        if self.call_stack.contains(&func_id) {
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        self.call_stack.push(func_id);
        self.visit_modifier_chain(func.modifiers, 0, body);
        self.call_stack.pop();
    }
}

fn is_delegatecall(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(call_expr, _, _) = &expr.kind else {
        return false;
    };

    matches!(&call_expr.kind, ExprKind::Member(_, member) if member.name == kw::Delegatecall)
}

fn resolved_function_ids<'hir>(
    callee: &'hir hir::Expr<'hir>,
) -> impl Iterator<Item = FunctionId> + 'hir {
    // Only direct internal calls resolve to function IDs here. Member calls and external calls
    // are intentionally ignored because they do not execute in the current function's HIR body.
    let reses = match &callee.peel_parens().kind {
        ExprKind::Ident(reses) => *reses,
        _ => &[],
    };
    reses.iter().filter_map(|res| match res {
        Res::Item(ItemId::Function(func_id)) => Some(*func_id),
        _ => None,
    })
}
