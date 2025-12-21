use solar::{interface::data_structures::Never, sema::hir};
use std::ops::ControlFlow;

use super::LintContext;

/// Trait for lints that operate on the HIR (High-level Intermediate Representation).
/// Its methods mirror `hir::visit::Visit`, with the addition of `LintCotext`.
pub trait LateLintPass<'hir>: Send + Sync {
    fn check_nested_source(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::SourceId,
    ) {
    }
    fn check_nested_item(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::ItemId,
    ) {
    }
    fn check_nested_contract(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::ContractId,
    ) {
    }
    fn check_nested_function(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::FunctionId,
    ) {
    }
    fn check_nested_var(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::VariableId,
    ) {
    }
    fn check_item(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _item: hir::Item<'hir, 'hir>,
    ) {
    }
    fn check_contract(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _contract: &'hir hir::Contract<'hir>,
    ) {
    }
    fn check_function(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _func: &'hir hir::Function<'hir>,
    ) {
    }
    fn check_modifier(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _mod: &'hir hir::Modifier<'hir>,
    ) {
    }
    fn check_var(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _var: &'hir hir::Variable<'hir>,
    ) {
    }
    fn check_expr(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _expr: &'hir hir::Expr<'hir>,
    ) {
    }
    fn check_call_args(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _args: &'hir hir::CallArgs<'hir>,
    ) {
    }
    fn check_stmt(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _stmt: &'hir hir::Stmt<'hir>,
    ) {
    }
    fn check_ty(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _ty: &'hir hir::Type<'hir>,
    ) {
    }
}

/// Visitor struct for `LateLintPass`es
pub struct LateLintVisitor<'a, 's, 'hir> {
    ctx: &'a LintContext<'s, 'a>,
    passes: &'a mut [Box<dyn LateLintPass<'hir> + 's>],
    hir: &'hir hir::Hir<'hir>,
}

impl<'a, 's, 'hir> LateLintVisitor<'a, 's, 'hir>
where
    's: 'hir,
{
    pub fn new(
        ctx: &'a LintContext<'s, 'a>,
        passes: &'a mut [Box<dyn LateLintPass<'hir> + 's>],
        hir: &'hir hir::Hir<'hir>,
    ) -> Self {
        Self { ctx, passes, hir }
    }
}

impl<'s, 'hir> hir::Visit<'hir> for LateLintVisitor<'_, 's, 'hir>
where
    's: 'hir,
{
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_nested_source(&mut self, id: hir::SourceId) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_source(self.ctx, self.hir, id);
        }
        self.walk_nested_source(id)
    }

    fn visit_contract(
        &mut self,
        contract: &'hir hir::Contract<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_contract(self.ctx, self.hir, contract);
        }
        self.walk_contract(contract)
    }

    fn visit_function(&mut self, func: &'hir hir::Function<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_function(self.ctx, self.hir, func);
        }
        self.walk_function(func)
    }

    fn visit_item(&mut self, item: hir::Item<'hir, 'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_item(self.ctx, self.hir, item);
        }
        self.walk_item(item)
    }

    fn visit_var(&mut self, var: &'hir hir::Variable<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_var(self.ctx, self.hir, var);
        }
        self.walk_var(var)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_expr(self.ctx, self.hir, expr);
        }
        self.walk_expr(expr)
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_stmt(self.ctx, self.hir, stmt);
        }
        self.walk_stmt(stmt)
    }

    fn visit_ty(&mut self, ty: &'hir hir::Type<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_ty(self.ctx, self.hir, ty);
        }
        self.walk_ty(ty)
    }
}
