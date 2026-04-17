use solar::{
    interface::data_structures::Never,
    sema::hir::{self, Visit},
};
use std::ops::ControlFlow;

use super::LintContext;

/// Trait for lints that operate on the HIR (High-level Intermediate Representation).
/// Its methods mirror `hir::visit::Visit`, with the addition of `LintContext`.
///
/// The original `check_nested_*` hooks took borrowed IDs, but current `solar::hir::Visit`
/// dispatches nested IDs by value. Those legacy hooks are kept as deprecated compatibility shims;
/// `LateLintVisitor` dispatches both the borrowed-ID hooks and the corresponding `*_id` hooks.
pub trait LateLintPass<'hir>: Send + Sync {
    fn check_nested_source(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::SourceId,
    ) {
    }
    #[deprecated(
        note = "use check_nested_item_id instead; current solar::hir::Visit passes ItemId by value"
    )]
    fn check_nested_item(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::ItemId,
    ) {
    }
    fn check_nested_item_id(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::ItemId,
    ) {
    }
    #[deprecated(
        note = "use check_nested_contract_id instead; current solar::hir::Visit passes ContractId by value"
    )]
    fn check_nested_contract(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::ContractId,
    ) {
    }
    fn check_nested_contract_id(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::ContractId,
    ) {
    }
    #[deprecated(
        note = "use check_nested_function_id instead; current solar::hir::Visit passes FunctionId by value"
    )]
    fn check_nested_function(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::FunctionId,
    ) {
    }
    fn check_nested_function_id(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::FunctionId,
    ) {
    }
    #[deprecated(
        note = "use check_nested_var_id instead; current solar::hir::Visit passes VariableId by value"
    )]
    fn check_nested_var(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::VariableId,
    ) {
    }
    fn check_nested_var_id(
        &mut self,
        _ctx: &LintContext,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::VariableId,
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
    #[allow(deprecated)]
    fn visit_nested_item_ref(&mut self, id: &'hir hir::ItemId) -> ControlFlow<Never> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_item(self.ctx, self.hir, id);
            pass.check_nested_item_id(self.ctx, self.hir, *id);
        }

        match id {
            hir::ItemId::Contract(id) => self.visit_nested_contract_ref(id),
            hir::ItemId::Function(id) => self.visit_nested_function_ref(id),
            hir::ItemId::Variable(id) => self.visit_nested_var_ref(id),
            _ => self.visit_item(self.hir.item(*id)),
        }
    }

    #[allow(deprecated)]
    fn visit_nested_contract_ref(&mut self, id: &'hir hir::ContractId) -> ControlFlow<Never> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_contract(self.ctx, self.hir, id);
            pass.check_nested_contract_id(self.ctx, self.hir, *id);
        }

        self.visit_contract(self.hir.contract(*id))
    }

    #[allow(deprecated)]
    fn visit_nested_function_ref(&mut self, id: &'hir hir::FunctionId) -> ControlFlow<Never> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_function(self.ctx, self.hir, id);
            pass.check_nested_function_id(self.ctx, self.hir, *id);
        }

        self.visit_function(self.hir.function(*id))
    }

    #[allow(deprecated)]
    fn visit_nested_var_ref(&mut self, id: &'hir hir::VariableId) -> ControlFlow<Never> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_var(self.ctx, self.hir, id);
            pass.check_nested_var_id(self.ctx, self.hir, *id);
        }

        self.visit_var(self.hir.variable(*id))
    }

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
        self.hir.source(id).items.iter().try_for_each(|id| self.visit_nested_item_ref(id))
    }

    fn visit_nested_item(&mut self, id: hir::ItemId) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_item_id(self.ctx, self.hir, id);
        }
        self.walk_nested_item(id)
    }

    fn visit_nested_contract(&mut self, id: hir::ContractId) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_contract_id(self.ctx, self.hir, id);
        }
        self.walk_nested_contract(id)
    }

    fn visit_nested_function(&mut self, id: hir::FunctionId) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_function_id(self.ctx, self.hir, id);
        }
        self.walk_nested_function(id)
    }

    fn visit_nested_var(&mut self, id: hir::VariableId) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_nested_var_id(self.ctx, self.hir, id);
        }
        self.walk_nested_var(id)
    }

    fn visit_contract(
        &mut self,
        contract: &'hir hir::Contract<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_contract(self.ctx, self.hir, contract);
        }
        for base in contract.bases_args {
            self.visit_modifier(base)?;
        }
        contract.items.iter().try_for_each(|id| self.visit_nested_item_ref(id))
    }

    fn visit_function(&mut self, func: &'hir hir::Function<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_function(self.ctx, self.hir, func);
        }
        for param in func.parameters {
            self.visit_nested_var_ref(param)?;
        }
        for modifier in func.modifiers {
            self.visit_modifier(modifier)?;
        }
        for ret in func.returns {
            self.visit_nested_var_ref(ret)?;
        }
        if let Some(body) = func.body.as_ref() {
            for stmt in body.iter() {
                self.visit_stmt(stmt)?;
            }
        }
        ControlFlow::Continue(())
    }

    fn visit_modifier(
        &mut self,
        modifier: &'hir hir::Modifier<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_modifier(self.ctx, self.hir, modifier);
        }
        self.walk_modifier(modifier)
    }

    fn visit_struct(&mut self, strukt: &'hir hir::Struct<'hir>) -> ControlFlow<Self::BreakValue> {
        for field in strukt.fields {
            self.visit_nested_var_ref(field)?;
        }
        ControlFlow::Continue(())
    }

    fn visit_error(&mut self, error: &'hir hir::Error<'hir>) -> ControlFlow<Self::BreakValue> {
        for param in error.parameters {
            self.visit_nested_var_ref(param)?;
        }
        ControlFlow::Continue(())
    }

    fn visit_event(&mut self, event: &'hir hir::Event<'hir>) -> ControlFlow<Self::BreakValue> {
        for param in event.parameters {
            self.visit_nested_var_ref(param)?;
        }
        ControlFlow::Continue(())
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

    fn visit_call_args(
        &mut self,
        args: &'hir hir::CallArgs<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_call_args(self.ctx, self.hir, args);
        }
        self.walk_call_args(args)
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_stmt(self.ctx, self.hir, stmt);
        }
        match &stmt.kind {
            hir::StmtKind::DeclSingle(var) => self.visit_nested_var_ref(var)?,
            hir::StmtKind::DeclMulti(vars, expr) => {
                for var in vars.iter().flatten() {
                    self.visit_nested_var_ref(var)?;
                }
                self.visit_expr(expr)?;
            }
            hir::StmtKind::Block(block)
            | hir::StmtKind::UncheckedBlock(block)
            | hir::StmtKind::Loop(block, _) => {
                for stmt in block.stmts {
                    self.visit_stmt(stmt)?;
                }
            }
            hir::StmtKind::Emit(expr) | hir::StmtKind::Revert(expr) | hir::StmtKind::Expr(expr) => {
                self.visit_expr(expr)?
            }
            hir::StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.visit_expr(expr)?;
                }
            }
            hir::StmtKind::Break
            | hir::StmtKind::Continue
            | hir::StmtKind::Placeholder
            | hir::StmtKind::Err(_) => {}
            hir::StmtKind::If(cond, true_, false_) => {
                self.visit_expr(cond)?;
                self.visit_stmt(true_)?;
                if let Some(false_) = false_ {
                    self.visit_stmt(false_)?;
                }
            }
            hir::StmtKind::Try(try_) => {
                self.visit_expr(&try_.expr)?;
                for clause in try_.clauses {
                    for var in clause.args {
                        self.visit_nested_var_ref(var)?;
                    }
                    for stmt in clause.block.iter() {
                        self.visit_stmt(stmt)?;
                    }
                }
            }
        }
        ControlFlow::Continue(())
    }

    fn visit_ty(&mut self, ty: &'hir hir::Type<'hir>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_ty(self.ctx, self.hir, ty);
        }
        match &ty.kind {
            hir::TypeKind::Elementary(_) | hir::TypeKind::Custom(_) | hir::TypeKind::Err(_) => {}
            hir::TypeKind::Array(arr) => {
                self.visit_ty(&arr.element)?;
                if let Some(len) = arr.size {
                    self.visit_expr(len)?;
                }
            }
            hir::TypeKind::Function(func) => {
                for param in func.parameters {
                    self.visit_nested_var_ref(param)?;
                }
                for ret in func.returns {
                    self.visit_nested_var_ref(ret)?;
                }
            }
            hir::TypeKind::Mapping(map) => {
                self.visit_ty(&map.key)?;
                self.visit_ty(&map.value)?;
            }
        }
        ControlFlow::Continue(())
    }
}
