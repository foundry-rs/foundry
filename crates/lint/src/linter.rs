use foundry_compilers::Language;
use foundry_config::lint::Severity;
use solar_ast::{
    visit::Visit, Expr, ImportDirective, ItemContract, ItemFunction, ItemStruct, SourceUnit,
    UsingDirective, VariableDefinition,
};
use solar_interface::{
    data_structures::Never,
    diagnostics::{DiagBuilder, DiagId, MultiSpan},
    Session, Span,
};
use std::{ops::ControlFlow, path::PathBuf};

/// Trait representing a generic linter for analyzing and reporting issues in smart contract source
/// code files. A linter can be implemented for any smart contract language supported by Foundry.
///
/// # Type Parameters
///
/// - `Language`: Represents the target programming language. Must implement the [`Language`] trait.
/// - `Lint`: Represents the types of lints performed by the linter. Must implement the [`Lint`]
///   trait.
///
/// # Required Methods
///
/// - `lint`: Scans the provided source files emitting a daignostic for lints found.
pub trait Linter: Send + Sync + Clone {
    type Language: Language;
    type Lint: Lint;

    fn lint(&self, input: &[PathBuf]);
}

pub trait Lint {
    fn id(&self) -> &'static str;
    fn severity(&self) -> Severity;
    fn description(&self) -> &'static str;
    fn help(&self) -> &'static str;
}

pub struct LintContext<'s> {
    sess: &'s Session,
    desc: bool,
}

impl<'s> LintContext<'s> {
    pub fn new(sess: &'s Session, with_description: bool) -> Self {
        Self { sess, desc: with_description }
    }

    /// Helper method to emit diagnostics easily from passes
    pub fn emit<L: Lint>(&self, lint: &'static L, span: Span) {
        let desc = if self.desc { lint.description() } else { "" };
        let diag: DiagBuilder<'_, ()> = self
            .sess
            .dcx
            .diag(lint.severity().into(), desc)
            .code(DiagId::new_str(lint.id()))
            .span(MultiSpan::from_span(span))
            .help(lint.help());

        diag.emit();
    }
}

/// Trait for lints that operate directly on the AST.
/// Its methods mirror `solar_ast::visit::Visit`, with the addition of `LintCotext`.
pub trait EarlyLintPass<'ast>: Send + Sync {
    fn check_expr(&mut self, _ctx: &LintContext<'_>, _expr: &'ast Expr<'ast>) {}
    fn check_item_struct(&mut self, _ctx: &LintContext<'_>, _struct: &'ast ItemStruct<'ast>) {}
    fn check_item_function(&mut self, _ctx: &LintContext<'_>, _func: &'ast ItemFunction<'ast>) {}
    fn check_variable_definition(
        &mut self,
        _ctx: &LintContext<'_>,
        _var: &'ast VariableDefinition<'ast>,
    ) {
    }
    fn check_import_directive(
        &mut self,
        _ctx: &LintContext<'_>,
        _import: &'ast ImportDirective<'ast>,
    ) {
    }
    fn check_using_directive(
        &mut self,
        _ctx: &LintContext<'_>,
        _using: &'ast UsingDirective<'ast>,
    ) {
    }
    fn check_item_contract(&mut self, _ctx: &LintContext<'_>, _contract: &'ast ItemContract<'ast>) {
    }
    // TODO: Add methods for each required AST node type

    /// Called after the entire source unit has been visited.
    /// This allows lints to perform analysis that requires seeing the whole AST.
    fn check_full_source_unit(&mut self, _ctx: &LintContext<'_>, _ast: &'ast SourceUnit<'ast>) {}
}

/// Visitor struct for `EarlyLintPass`es
pub struct EarlyLintVisitor<'a, 's, 'ast> {
    pub ctx: &'a LintContext<'s>,
    pub passes: &'a mut [Box<dyn EarlyLintPass<'ast> + 's>],
}

/// Extends the [`Visit`] trait with an additional hook that runs after the initial traversal.
impl<'s, 'ast> EarlyLintVisitor<'_, 's, 'ast>
where
    's: 'ast,
{
    pub fn post_source_unit(&mut self, ast: &'ast SourceUnit<'ast>) {
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

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_expr(self.ctx, expr)
        }
        self.walk_expr(expr)
    }

    fn visit_variable_definition(
        &mut self,
        var: &'ast VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_variable_definition(self.ctx, var)
        }
        self.walk_variable_definition(var)
    }

    fn visit_item_struct(
        &mut self,
        strukt: &'ast ItemStruct<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_item_struct(self.ctx, strukt)
        }
        self.walk_item_struct(strukt)
    }

    fn visit_item_function(
        &mut self,
        func: &'ast ItemFunction<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_item_function(self.ctx, func)
        }
        self.walk_item_function(func)
    }

    fn visit_import_directive(
        &mut self,
        import: &'ast ImportDirective<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_import_directive(self.ctx, import);
        }
        self.walk_import_directive(import)
    }

    fn visit_using_directive(
        &mut self,
        using: &'ast UsingDirective<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_using_directive(self.ctx, using);
        }
        self.walk_using_directive(using)
    }

    fn visit_item_contract(
        &mut self,
        contract: &'ast ItemContract<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        for pass in self.passes.iter_mut() {
            pass.check_item_contract(self.ctx, contract);
        }
        self.walk_item_contract(contract)
    }

    // TODO: Add methods for each required AST node type, mirroring `solar_ast::visit::Visit` method
    // sigs + adding `LintContext`
}
