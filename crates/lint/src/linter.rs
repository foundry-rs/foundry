use foundry_compilers::Language;
use foundry_config::lint::Severity;
use solar_ast::{self as ast, visit::Visit};
use solar_interface::{
    Session, Span,
    data_structures::Never,
    diagnostics::{DiagBuilder, DiagId, DiagMsg, MultiSpan, Style},
};
use solar_sema::{ParsingContext, hir};
use std::{ops::ControlFlow, path::PathBuf};

use crate::inline_config::InlineConfig;

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
/// - `init`: Creates a new solar `Session` with the appropiate linter configuration.
/// - `early_lint`: Scans the source files (using the AST) emitting a diagnostic for lints found.
/// - `late_lint`: Scans the source files (using the HIR) emitting a diagnostic for lints found.
pub trait Linter: Send + Sync + Clone {
    type Language: Language;
    type Lint: Lint;

    fn init(&self) -> Session;
    fn early_lint<'sess>(&self, input: &[PathBuf], pcx: ParsingContext<'sess>);
    fn late_lint<'sess>(&self, input: &[PathBuf], pcx: ParsingContext<'sess>);
}

pub trait Lint {
    fn id(&self) -> &'static str;
    fn severity(&self) -> Severity;
    fn description(&self) -> &'static str;
    fn help(&self) -> &'static str;
}

pub struct LintContext<'s> {
    sess: &'s Session,
    with_description: bool,
    pub inline_config: InlineConfig,
}

impl<'s> LintContext<'s> {
    pub fn new(sess: &'s Session, with_description: bool, config: InlineConfig) -> Self {
        Self { sess, with_description, inline_config: config }
    }

    /// Helper method to emit diagnostics easily from passes.
    pub fn emit<L: Lint>(&self, lint: &'static L, span: Span) {
        if self.inline_config.is_disabled(span, lint.id()) {
            return;
        }

        let desc = if self.with_description { lint.description() } else { "" };
        let diag: DiagBuilder<'_, ()> = self
            .sess
            .dcx
            .diag(lint.severity().into(), desc)
            .code(DiagId::new_str(lint.id()))
            .span(MultiSpan::from_span(span))
            .help(lint.help());

        diag.emit();
    }

    /// Emit a diagnostic with a code fix proposal.
    ///
    /// For Diff snippets, if no span is provided, it will use the lint's span.
    /// If unable to get code from the span, it will fall back to a Block snippet.
    pub fn emit_with_fix<L: Lint>(&self, lint: &'static L, span: Span, snippet: Snippet) {
        if self.inline_config.is_disabled(span, lint.id()) {
            return;
        }

        // Convert the snippet to ensure we have the appropriate type
        let snippet = match snippet {
            Snippet::Diff { desc, span: diff_span, add } => {
                // Use the provided span or fall back to the lint span
                let target_span = diff_span.unwrap_or(span);

                // Check if we can get the original code
                if self.span_to_snippet(target_span).is_some() {
                    Snippet::Diff { desc, span: Some(target_span), add }
                } else {
                    // Fall back to Block if we can't get the original code
                    Snippet::Block { desc, code: add }
                }
            }
            // Block snippets remain unchanged
            block => block,
        };

        let desc = if self.with_description { lint.description() } else { "" };
        let diag: DiagBuilder<'_, ()> = self
            .sess
            .dcx
            .diag(lint.severity().into(), desc)
            .code(DiagId::new_str(lint.id()))
            .span(MultiSpan::from_span(span))
            .highlighted_note(snippet.to_note(self))
            .help(lint.help());

        diag.emit();
    }

    pub fn span_to_snippet(&self, span: Span) -> Option<String> {
        self.sess.source_map().span_to_snippet(span).ok()
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Snippet {
    /// Represents a code block. Can have an optional description.
    Block { desc: Option<&'static str>, code: String },
    /// Represents a code diff. Can have an optional description and a span for the code to remove.
    Diff { desc: Option<&'static str>, span: Option<Span>, add: String },
}

impl Snippet {
    pub fn to_note(self, ctx: &LintContext<'_>) -> Vec<(DiagMsg, Style)> {
        let mut output = Vec::new();
        match self.desc() {
            Some(desc) => {
                output.push((DiagMsg::from(desc), Style::NoStyle));
                output.push((DiagMsg::from("\n\n"), Style::NoStyle));
            }
            None => output.push((DiagMsg::from(" \n"), Style::NoStyle)),
        }
        match self {
            Self::Diff { span, add, .. } => {
                // Get the original code from the span if provided
                if let Some(span) = span {
                    if let Some(rmv) = ctx.span_to_snippet(span) {
                        for line in rmv.lines() {
                            output.push((DiagMsg::from(format!("- {line}\n")), Style::Removal));
                        }
                    }
                }
                for line in add.lines() {
                    output.push((DiagMsg::from(format!("+ {line}\n")), Style::Addition));
                }
            }
            Self::Block { code, .. } => {
                for line in code.lines() {
                    output.push((DiagMsg::from(format!("- {line}\n")), Style::NoStyle));
                }
            }
        }
        output.push((DiagMsg::from("\n"), Style::NoStyle));
        output
    }

    pub fn desc(&self) -> Option<&'static str> {
        match self {
            Self::Diff { desc, .. } => *desc,
            Self::Block { desc, .. } => *desc,
        }
    }
}

/// Trait for lints that operate directly on the AST.
/// Its methods mirror `solar_ast::visit::Visit`, with the addition of `LintCotext`.
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
    // TODO: Add methods for each required AST node type

    /// Should be called after the source unit has been visited. Enables lints that require
    /// knowledge of the entire AST to perform their analysis.
    fn check_full_source_unit(
        &mut self,
        _ctx: &LintContext<'_>,
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

/// Trait for lints that operate on the HIR (High-level Intermediate Representation).
/// Its methods mirror `solar_ast::visit::Visit`, with the addition of `LintCotext`.
pub trait LateLintPass<'hir>: Send + Sync {
    fn check_nested_source(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _id: hir::SourceId,
    ) {
    }
    fn check_nested_item(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::ItemId,
    ) {
    }
    fn check_nested_contract(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::ContractId,
    ) {
    }
    fn check_nested_function(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::FunctionId,
    ) {
    }
    fn check_nested_var(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _id: &'hir hir::VariableId,
    ) {
    }
    fn check_item(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _item: hir::Item<'hir, 'hir>,
    ) {
    }
    fn check_contract(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _contract: &'hir hir::Contract<'hir>,
    ) {
    }
    fn check_function(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _func: &'hir hir::Function<'hir>,
    ) {
    }
    fn check_modifier(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _mod: &'hir hir::Modifier<'hir>,
    ) {
    }
    fn check_var(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _var: &'hir hir::Variable<'hir>,
    ) {
    }
    fn check_expr(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _expr: &'hir hir::Expr<'hir>,
    ) {
    }
    fn check_call_args(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _args: &'hir hir::CallArgs<'hir>,
    ) {
    }
    fn check_stmt(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _stmt: &'hir hir::Stmt<'hir>,
    ) {
    }
    fn check_ty(
        &mut self,
        _ctx: &LintContext<'_>,
        _hir: &'hir hir::Hir<'hir>,
        _ty: &'hir hir::Type<'hir>,
    ) {
    }
}

/// Visitor struct for `LateLintPass`es
pub struct LateLintVisitor<'a, 's, 'hir> {
    ctx: &'a LintContext<'s>,
    passes: &'a mut [Box<dyn LateLintPass<'hir> + 's>],
    hir: &'hir hir::Hir<'hir>,
}

impl<'a, 's, 'hir> LateLintVisitor<'a, 's, 'hir>
where
    's: 'hir,
{
    pub fn new(
        ctx: &'a LintContext<'s>,
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
