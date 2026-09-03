use foundry_compilers::Language;
use foundry_config::DenyLevel;
use solar::sema::Compiler;
use std::path::PathBuf;

pub use solar_lint::{
    EarlyLintPass, EarlyLintVisitor, LateLintPass, LateLintVisitor, Lint, LintContext, LintPolicy,
    ProjectLintContext as ProjectLintEmitter, ProjectLintPass, ProjectSource, Suggestion,
    SuggestionKind,
};

/// Trait representing a linter for a language supported by Foundry.
pub trait Linter: Send + Sync {
    /// The target language.
    type Language: Language;
    /// The lint metadata type.
    type Lint: Lint;

    /// Runs all configured lints against an already parsed Solar compiler.
    fn lint(&self, input: &[PathBuf], deny: DenyLevel, compiler: &mut Compiler)
    -> eyre::Result<()>;
}
