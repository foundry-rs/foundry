//! Loop-context walker shared by the `*-loop` lints.
//!
//! Visits every statement and expression that executes inside a loop of a function, following
//! its modifier chain through `_` and inlining the internal helpers it calls (including `super`
//! dispatch, resolved against the contract the entry point belongs to).

use crate::sol::analysis::{is_builtin, is_contract_cast, loop_stmts};
use solar::{
    ast::{StateMutability, Visibility},
    interface::{Symbol, sym},
    sema::{
        Gcx,
        hir::{
            Block, ContractId, Expr, ExprKind, Function, FunctionId, FunctionKind, Hir, Modifier,
            Stmt, StmtKind, Visit,
        },
    },
};
use std::{convert::Infallible, ops::ControlFlow};

/// A statement or expression reached inside a loop.
pub(super) enum LoopItem<'gcx> {
    Stmt(&'gcx Stmt<'gcx>),
    Expr(&'gcx Expr<'gcx>),
}

/// Calls `f` for every expression executing inside a loop of a payable entry point, including
/// loops in the internal helpers it calls (whether the call itself sits in a loop or not).
pub(super) fn for_each_payable_loop_expr<'gcx>(
    gcx: Gcx<'gcx>,
    func: &'gcx Function<'gcx>,
    mut f: impl FnMut(&'gcx Expr<'gcx>),
) {
    if !matches!(func.kind, FunctionKind::Constructor | FunctionKind::Modifier)
        && func.state_mutability == StateMutability::Payable
        && matches!(func.visibility, Visibility::Public | Visibility::External)
    {
        for_each_loop_item(gcx, func, true, |item| {
            if let LoopItem::Expr(expr) = item {
                f(expr);
            }
        });
    }
}

/// Calls `f` for every statement and expression executing inside a loop of `func`. Internal
/// helpers called from a loop are inlined; with `follow_calls_outside_loop`, so are helpers
/// called outside one, so that their own loops are reported too.
pub(super) fn for_each_loop_item<'gcx>(
    gcx: Gcx<'gcx>,
    func: &'gcx Function<'gcx>,
    follow_calls_outside_loop: bool,
    f: impl FnMut(LoopItem<'gcx>),
) {
    let Some(body) = func.body else { return };
    let mut walker = LoopWalker {
        gcx,
        f,
        loop_depth: 0,
        placeholder: None,
        stack: Vec::new(),
        dispatch: func.contract,
        current: func.contract,
        follow_calls_outside_loop,
    };
    walker.visit_modifiers(func.modifiers, 0, body, func.contract);
}

/// The rest of a modifier chain and the function body it wraps, executed at `_`.
type Continuation<'gcx> = (&'gcx [Modifier<'gcx>], usize, Block<'gcx>, Option<ContractId>);

struct LoopWalker<'gcx, F> {
    gcx: Gcx<'gcx>,
    f: F,
    loop_depth: usize,
    placeholder: Option<Continuation<'gcx>>,
    /// Modifiers and helpers currently being inlined, to cut recursion.
    stack: Vec<FunctionId>,
    /// Contract whose linearization resolves `super`.
    dispatch: Option<ContractId>,
    /// Contract defining the code being walked.
    current: Option<ContractId>,
    follow_calls_outside_loop: bool,
}

impl<'gcx, F: FnMut(LoopItem<'gcx>)> LoopWalker<'gcx, F> {
    fn visit_modifiers(
        &mut self,
        modifiers: &'gcx [Modifier<'gcx>],
        index: usize,
        body: Block<'gcx>,
        contract: Option<ContractId>,
    ) {
        let Some(modifier) = modifiers.get(index) else {
            return self.visit_scoped(body, None, contract);
        };
        let _ = self.visit_call_args(&modifier.args);
        if let Some(id) = modifier.id.as_function()
            && let Some(modifier_body) = self.gcx.hir.function(id).body
            && !self.stack.contains(&id)
        {
            self.stack.push(id);
            let continuation = Some((modifiers, index + 1, body, contract));
            self.visit_scoped(modifier_body, continuation, self.gcx.hir.function(id).contract);
            self.stack.pop();
        } else {
            self.visit_modifiers(modifiers, index + 1, body, contract);
        }
    }

    fn visit_scoped(
        &mut self,
        block: Block<'gcx>,
        placeholder: Option<Continuation<'gcx>>,
        contract: Option<ContractId>,
    ) {
        let saved = (self.placeholder, self.current);
        (self.placeholder, self.current) = (placeholder, contract);
        for stmt in block.stmts {
            let _ = self.visit_stmt(stmt);
        }
        (self.placeholder, self.current) = saved;
    }

    fn visit_call(&mut self, id: FunctionId) {
        let func = self.gcx.hir.function(id);
        if let Some(body) = func.body
            && !self.stack.contains(&id)
        {
            self.stack.push(id);
            self.visit_modifiers(func.modifiers, 0, body, func.contract);
            self.stack.pop();
        }
    }

    /// The internal function a call dispatches to, if it can be inlined: a helper called
    /// directly, through a library/base qualifier or a `using for` binding, or via `super`.
    /// Calls on a contract-typed value (`this` included) are external and are not followed.
    fn callee(&self, callee: &'gcx Expr<'gcx>) -> Option<FunctionId> {
        let callee = callee.peel_parens();
        let func_id = self.gcx.resolved_expr(callee)?.as_function()?;
        let ExprKind::Member(base, member) = &callee.kind else { return Some(func_id) };
        if is_builtin(base, sym::super_) {
            return self.super_target(func_id, member.name);
        }
        let attached = self.gcx.resolved_callee(callee.id).is_some_and(|c| c.attached);
        (attached || is_contract_cast(base)).then_some(func_id)
    }

    /// `super.<name>(..)` resolved against the dispatching contract: the first base after the
    /// current one (in its linearization) defining `name` with the resolved signature.
    fn super_target(&self, resolved: FunctionId, name: Symbol) -> Option<FunctionId> {
        let bases = self.gcx.hir.contract(self.dispatch?).linearized_bases;
        let start = bases.iter().position(|&c| Some(c) == self.current)? + 1;
        let params = self.gcx.item_parameter_types(resolved);
        bases[start..].iter().flat_map(|&c| self.gcx.hir.contract(c).functions()).find(|&id| {
            let func = self.gcx.hir.function(id);
            func.name.is_some_and(|n| n.name == name) && self.gcx.item_parameter_types(id) == params
        })
    }
}

impl<'gcx, F: FnMut(LoopItem<'gcx>)> Visit<'gcx> for LoopWalker<'gcx, F> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Infallible> {
        if self.loop_depth > 0 {
            (self.f)(LoopItem::Stmt(stmt));
        }
        match stmt.kind {
            StmtKind::Loop(block, source) => {
                self.loop_depth += 1;
                for stmt in loop_stmts(block, source) {
                    self.visit_stmt(stmt)?;
                }
                self.loop_depth -= 1;
            }
            StmtKind::Placeholder => {
                if let Some((modifiers, index, body, contract)) = self.placeholder {
                    self.visit_modifiers(modifiers, index, body, contract);
                }
            }
            _ => self.walk_stmt(stmt)?,
        }
        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Infallible> {
        self.walk_expr(expr)?;
        let in_loop = self.loop_depth > 0;
        if in_loop {
            (self.f)(LoopItem::Expr(expr));
        }
        if (in_loop || self.follow_calls_outside_loop)
            && let ExprKind::Call(callee, ..) = &expr.kind
            && let Some(id) = self.callee(callee)
        {
            self.visit_call(id);
        }
        ControlFlow::Continue(())
    }
}
