use clap::ValueEnum;
use solar_ast::{Expr, ItemStruct, Span, VariableDefinition};
use solar_interface::{diagnostics::Level, Session};
use core::fmt;
use foundry_compilers::{artifacts::SourceUnit, Language};
use std::{hash::Hash, marker::PhantomData, ops::ControlFlow, path::PathBuf};
use yansi::Paint;

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

#[derive(Debug, Clone, Copy)]
pub struct Lint {
    pub id: &'static str,
    pub description: &'static str,
    pub help: Option<&'static str>,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Severity {
    High,
    Med,
    Low,
    Info,
    Gas,
}

impl Severity {
    pub fn color(&self, message: &str) -> String {
        match self {
            Self::High => Paint::red(message).bold().to_string(),
            Self::Med => Paint::rgb(message, 255, 135, 61).bold().to_string(),
            Self::Low => Paint::yellow(message).bold().to_string(),
            Self::Info => Paint::cyan(message).bold().to_string(),
            Self::Gas => Paint::green(message).bold().to_string(),
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let colored = match self {
            Self::High => self.color("High"),
            Self::Med => self.color("Med"),
            Self::Low => self.color("Low"),
            Self::Info => self.color("Info"),
            Self::Gas => self.color("Gas"),
        };
        write!(f, "{colored}")
    }
}

pub struct LintContext<'s, 'ast> {
    pub sess: &'s Session,
    _phantom: PhantomData<&'ast ()>,
}

impl<'s, 'ast> LintContext<'s, 'ast> {
    pub fn new(sess: &'s Session) -> Self {
        Self { sess, _phantom: PhantomData }
    }

    // Helper method to emit diagnostics easily from passes
    pub fn emit(&self, lint: &'static Lint, span: Span) {
         let mut diag = self.sess.dcx.diag(lint.severity, format!("{}: {}", lint.id, lint.description));
         diag.span(span);
         if let Some(help) = lint.help {
             diag.help(help);
         }
         diag.emit();
    }
}

pub trait EarlyLintPass<'ast>: fmt::Debug + Send + Sync + Clone {
    // TODO: Add methods for each required AST node type, mirroring `solar_ast::visit::Visit` method sigs + adding `LintContext`
    fn check_expr(&mut self, _cx: &LintContext<'_, 'ast>, _expr: &'ast Expr<'ast>) -> ControlFlow<()> { ControlFlow::Continue(()) }
    fn check_variable_definition(&mut self, _cx: &LintContext<'_, 'ast>, _var: &'ast VariableDefinition<'ast>) -> ControlFlow<()> { ControlFlow::Continue(()) }
    fn check_item_struct(&mut self, _cx: &LintContext<'_, 'ast>, _struct: &'ast ItemStruct<'ast>) -> ControlFlow<()> { ControlFlow::Continue(()) }
}
