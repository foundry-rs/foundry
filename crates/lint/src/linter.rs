use foundry_compilers::Language;
use foundry_config::lint::Severity;
use solar_ast::{visit::Visit, Expr, ItemFunction, ItemStruct, VariableDefinition};
use solar_interface::{
    data_structures::Never,
    diagnostics::{DiagBuilder, DiagId, DiagMsg, MultiSpan, Style},
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

    /// Helper method to emit diagnostics easily from passes.
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

    /// Emit a diagnostic with a code fix proposal.
    pub fn emit_with_fix<L: Lint>(&self, lint: &'static L, span: Span, snippet: Snippet) {
        let desc = if self.desc { lint.description() } else { "" };

        let diag: DiagBuilder<'_, ()> = self
            .sess
            .dcx
            .diag(lint.severity().into(), desc)
            .code(DiagId::new_str(lint.id()))
            .span(MultiSpan::from_span(span))
            .highlighted_note(snippet.to_note())
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
    /// Represents a code diff.
    /// Includes an optional description, the code to remove, and a replacement proposal.
    Diff { desc: Option<&'static str>, rmv: String, add: String },
}

impl Snippet {
    pub fn to_note(self) -> Vec<(DiagMsg, Style)> {
        let mut output = Vec::new();
        match self.desc() {
            Some(desc) => {
                output.push((DiagMsg::from(desc), Style::NoStyle));
                output.push((DiagMsg::from("\n\n"), Style::NoStyle));
            }
            None => output.push((DiagMsg::from(" \n"), Style::NoStyle)),
        }
        match self {
            Self::Diff { rmv, add, .. } => {
                for line in rmv.lines() {
                    output.push((DiagMsg::from(format!("- {line}\n")), Style::Removal));
                }
                for line in add.lines() {
                    output.push((DiagMsg::from(format!("+ {line}\n")), Style::Addition));
                }
            }
            Self::Block { code, .. } => {
                for line in code.lines() {
                    output.push((DiagMsg::from(format!("- {line}\n")), Style::Removal));
                }
            }
        }
        output.push((DiagMsg::from("\n"), Style::NoStyle));
        output
    }

    fn desc(&self) -> Option<&'static str> {
        match self {
            Self::Diff { desc, .. } => *desc,
            Self::Block { desc, .. } => *desc,
        }
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

    // TODO: Add methods for each required AST node type
}

/// Visitor struct for `EarlyLintPass`es
pub struct EarlyLintVisitor<'a, 's, 'ast> {
    pub ctx: &'a LintContext<'s>,
    pub passes: &'a mut [Box<dyn EarlyLintPass<'ast> + 's>],
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

    // TODO: Add methods for each required AST node type, mirroring `solar_ast::visit::Visit` method
    // sigs + adding `LintContext`
}
