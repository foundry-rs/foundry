use solar_ast::{self as ast, visit::Visit};
use solar_interface::data_structures::Never;
use std::ops::ControlFlow;

use super::LintContext;

/// Trait for lints that operate directly on the AST.
/// Its methods mirror `ast::visit::Visit`, with the addition of `LintCotext`.
pub trait EarlyLintPass<'ast>: Send + Sync {
    fn check_expr(&mut self, _ctx: &LintContext<'_>, _expr: &'ast ast::Expr<'ast>) {}
    fn check_item_struct(&mut self, _ctx: &LintContext<'_>, _struct: &'ast ast::ItemStruct<'ast>) {}
    fn check_item_function(
        &mut self,
        _ctx: &LintContext<'_>,
        _func: &'ast ast::ItemFunction<'ast>,
    ) {
    }
    fn check_variable_definition(
        &mut self,
        _ctx: &LintContext<'_>,
        _var: &'ast ast::VariableDefinition<'ast>,
    ) {
    }
    fn check_import_directive(
        &mut self,
        _ctx: &LintContext<'_>,
        _import: &'ast ast::ImportDirective<'ast>,
    ) {
    }
    fn check_using_directive(
        &mut self,
        _ctx: &LintContext<'_>,
        _using: &'ast ast::UsingDirective<'ast>,
    ) {
    }
    fn check_item_contract(
        &mut self,
        _ctx: &LintContext<'_>,
        _contract: &'ast ast::ItemContract<'ast>,
    ) {
    }
    fn check_doc_comment(&mut self, _ctx: &LintContext<'_>, _cmnt: &'ast ast::DocComment) {}
    // TODO: Add methods for each required AST node type

    /// Should be called after the source unit has been visited. Enables lints that require
    /// knowledge of the entire AST to perform their analysis.
    ///
    /// # Performance
    ///
    /// Since a full-AST analysis can be computationally expensive, implementations
    /// should guard their logic by first checking if the relevant lint is enabled
    /// using [`LintContext::is_lint_enabled`]. This avoids performing costly work
    /// if the user has disabled the lint.
    ///
    /// ### Example
    /// ```rust,ignore
    /// fn check_full_source_unit(&mut self, ctx: &LintContext<'ast>, ast: &'ast ast::SourceUnit<'ast>) {
    ///     // Check if the lint is enabled before performing expensive work.
    ///     if ctx.is_lint_enabled(MY_EXPENSIVE_LINT.id) {
    ///         // ... perform computation and emit diagnostics ...
    ///     }
    /// }
    /// ```
    fn check_full_source_unit(
        &mut self,
        _ctx: &LintContext<'ast>,
        _ast: &'ast ast::SourceUnit<'ast>,
    ) {
    }
}

/// Visitor struct for `EarlyLintPass`es
pub struct EarlyLintVisitor<'a, 's, 'ast> {
    pub ctx: &'a LintContext<'s>,
    pub passes: &'a mut [Box<dyn EarlyLintPass<'ast> + 's>],
}

impl<'a, 's, 'ast> EarlyLintVisitor<'a, 's, 'ast>
where
    's: 'ast,
{
    pub fn new(
        ctx: &'a LintContext<'s>,
        passes: &'a mut [Box<dyn EarlyLintPass<'ast> + 's>],
    ) -> Self {
        Self { ctx, passes }
    }

    /// Extends the [`Visit`] trait functionality with a hook that can run after the initial
    /// traversal.
    pub fn post_source_unit(&mut self, ast: &'ast ast::SourceUnit<'ast>) {
        for pass in self.passes.iter_mut() {
            pass.check_full_source_unit(self.ctx, ast);
        }
    }
}

impl<'s, 'ast> Visit<'ast> for EarlyLintVisitor<'_, 's, 'ast>
where
    's: 'ast,
{
    type BreakValue = Never;

    fn visit_doc_comment(&mut self, cmnt: &'ast ast::DocComment) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_doc_comment(self.ctx, cmnt)
        }
        self.walk_doc_comment(cmnt)
    }

    fn visit_expr(&mut self, expr: &'ast ast::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_expr(self.ctx, expr)
        }
        self.walk_expr(expr)
    }

    fn visit_variable_definition(
        &mut self,
        var: &'ast ast::VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_variable_definition(self.ctx, var)
        }
        self.walk_variable_definition(var)
    }

    fn visit_item_struct(
        &mut self,
        strukt: &'ast ast::ItemStruct<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_item_struct(self.ctx, strukt)
        }
        self.walk_item_struct(strukt)
    }

    fn visit_item_function(
        &mut self,
        func: &'ast ast::ItemFunction<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_item_function(self.ctx, func)
        }
        self.walk_item_function(func)
    }

    fn visit_import_directive(
        &mut self,
        import: &'ast ast::ImportDirective<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_import_directive(self.ctx, import);
        }
        self.walk_import_directive(import)
    }

    fn visit_using_directive(
        &mut self,
        using: &'ast ast::UsingDirective<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_using_directive(self.ctx, using);
        }
        self.walk_using_directive(using)
    }

    fn visit_item_contract(
        &mut self,
        contract: &'ast ast::ItemContract<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_item_contract(self.ctx, contract);
        }
        self.walk_item_contract(contract)
    }

    // TODO: Add methods for each required AST node type, mirroring `solar_ast::visit::Visit` method
    // sigs + adding `LintContext`
}
