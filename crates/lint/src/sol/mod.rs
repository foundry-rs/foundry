use crate::linter::{Lint, LintPolicy, Linter};
use foundry_common::{
    comments::{
        Comments,
        inline_config::{InlineConfig, InlineConfigItem},
    },
    errors::convert_solar_errors,
    sh_warn,
};
use foundry_compilers::{ProjectPathsConfig, solc::SolcLanguage};
use foundry_config::{
    DenyLevel,
    lint::{LintSpecificConfig, Severity},
};
use solar::{
    ast,
    interface::{
        ColorChoice, Session,
        diagnostics::{HumanEmitter, JsonEmitter, Level, SilentEmitter},
    },
    sema::Compiler,
};
use solar_lint::{LintRegistry, LintRunContext, LintRunError, LintSource, LintSuite, run_lints};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};
use thiserror::Error;

#[macro_use]
pub mod macros;

pub mod analysis;
pub mod codesize;
pub mod gas;
pub mod high;
pub mod info;
pub mod low;
pub mod med;
pub mod naming;

/// Every registered lint, in severity-group order.
fn all_lints() -> impl Iterator<Item = &'static SolLint> {
    [
        high::REGISTERED_LINTS,
        med::REGISTERED_LINTS,
        low::REGISTERED_LINTS,
        info::REGISTERED_LINTS,
        gas::REGISTERED_LINTS,
        codesize::REGISTERED_LINTS,
    ]
    .into_iter()
    .flatten()
}

static ALL_REGISTERED_LINTS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| all_lints().map(|lint| lint.id).collect());

static DEFAULT_LINT_SPECIFIC_CONFIG: LazyLock<LintSpecificConfig> =
    LazyLock::new(LintSpecificConfig::default);

struct OwnedLintPolicy {
    inline: Option<Arc<InlineConfig<Vec<String>>>>,
    active: Arc<Vec<&'static str>>,
    sources: Option<Arc<Vec<SourceLintPolicy>>>,
}

struct SourceLintPolicy {
    file: Arc<solar::interface::source_map::SourceFile>,
    inline: Arc<InlineConfig<Vec<String>>>,
    active: Vec<&'static str>,
}

impl LintPolicy for OwnedLintPolicy {
    fn is_lint_enabled(&self, id: &str) -> bool {
        self.active.contains(&id)
    }

    fn is_lint_suppressed(&self, id: &str, span: solar::interface::Span) -> bool {
        if !span.is_dummy()
            && let Some(sources) = &self.sources
        {
            // Late passes can follow inheritance or calls into another file. Apply the policy of
            // the file that owns the diagnostic span, not the file whose visitor emitted it.
            let source = sources
                .partition_point(|source| source.file.start_pos <= span.lo())
                .checked_sub(1)
                .map(|idx| &sources[idx])
                .filter(|source| source.file.contains(span.lo()));
            return source.is_none_or(|source| {
                !source.active.contains(&id) || source.inline.is_id_disabled(span, id)
            });
        }
        self.inline.as_ref().is_some_and(|inline| inline.is_id_disabled(span, id))
    }
}

/// A reusable collection of Forge lint passes and policy.
#[derive(Clone)]
pub struct ForgeLintSuite {
    path_config: ProjectPathsConfig,
    severity: Option<Vec<Severity>>,
    lints_included: Option<Vec<SolLint>>,
    lints_excluded: Option<Vec<SolLint>>,
    registry: Arc<LintRegistry>,
    sources: Option<Arc<Vec<SourceLintPolicy>>>,
    run_active: Option<Arc<Vec<&'static str>>>,
}

impl std::fmt::Debug for ForgeLintSuite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForgeLintSuite")
            .field("path_config", &self.path_config)
            .field("severity", &self.severity)
            .field("lints_included", &self.lints_included)
            .field("lints_excluded", &self.lints_excluded)
            .finish_non_exhaustive()
    }
}

impl ForgeLintSuite {
    fn include_lint(&self, lint: SolLint) -> bool {
        self.severity.as_ref().is_none_or(|sev| sev.contains(&lint.severity()))
            && self.lints_included.as_ref().is_none_or(|incl| incl.contains(&lint))
            && self.lints_excluded.as_ref().is_none_or(|excl| !excl.contains(&lint))
    }

    fn active_lints(&self, path: Option<&Path>) -> Vec<&'static str> {
        all_lints()
            .filter(|lint| {
                self.include_lint(**lint)
                    && path.is_none_or(|path| {
                        !self.path_config.is_test_or_script(path)
                            || matches!(
                                lint.id,
                                "unsafe-cheatcode"
                                    | "block-number-across-roll"
                                    | "block-timestamp-across-warp"
                            )
                    })
            })
            .map(|lint| lint.id)
            .collect()
    }
}

impl LintSuite for ForgeLintSuite {
    fn registry(&self) -> &LintRegistry {
        &self.registry
    }

    fn source_policy(&self, source: LintSource<'_, '_>) -> Arc<dyn LintPolicy> {
        let inline = self
            .sources
            .as_ref()
            .and_then(|sources| {
                sources
                    .binary_search_by_key(&source.file.start_pos, |source| source.file.start_pos)
                    .ok()
                    .map(|idx| sources[idx].inline.clone())
            })
            .unwrap_or_else(|| {
                let comments =
                    Comments::new(source.file, source.session.source_map(), false, false, None);
                Arc::new(parse_inline_config(source.session, &comments, source.ast))
            });
        Arc::new(OwnedLintPolicy {
            inline: Some(inline),
            active: self
                .run_active
                .clone()
                .unwrap_or_else(|| Arc::new(self.active_lints(Some(source.path)))),
            sources: self.sources.clone(),
        })
    }

    fn project_policy(&self) -> Arc<dyn LintPolicy> {
        Arc::new(OwnedLintPolicy {
            inline: None,
            active: Arc::new(self.active_lints(None)),
            sources: None,
        })
    }
}

/// Linter implementation to analyze Solidity source code responsible for identifying
/// vulnerabilities gas optimizations, and best practices.
#[derive(Debug)]
pub struct SolidityLinter<'a> {
    path_config: ProjectPathsConfig,
    severity: Option<Vec<Severity>>,
    lints_included: Option<Vec<SolLint>>,
    lints_excluded: Option<Vec<SolLint>>,
    with_description: bool,
    with_json_emitter: bool,
    json_emitter_stdout: bool,
    // lint-specific configuration
    lint_specific: &'a LintSpecificConfig,
}

impl<'a> SolidityLinter<'a> {
    pub fn new(path_config: ProjectPathsConfig) -> Self {
        Self {
            path_config,
            with_description: true,
            severity: None,
            lints_included: None,
            lints_excluded: None,
            with_json_emitter: false,
            json_emitter_stdout: false,
            lint_specific: &DEFAULT_LINT_SPECIFIC_CONFIG,
        }
    }

    pub fn with_severity(mut self, severity: Option<Vec<Severity>>) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_lints(mut self, lints: Option<Vec<SolLint>>) -> Self {
        self.lints_included = lints;
        self
    }

    pub fn without_lints(mut self, lints: Option<Vec<SolLint>>) -> Self {
        self.lints_excluded = lints;
        self
    }

    pub const fn with_description(mut self, with: bool) -> Self {
        self.with_description = with;
        self
    }

    pub const fn with_json_emitter(mut self, with: bool) -> Self {
        self.with_json_emitter = with;
        self
    }

    pub const fn with_json_emitter_stdout(mut self, with: bool) -> Self {
        self.json_emitter_stdout = with;
        self
    }

    pub const fn with_lint_specific(mut self, lint_specific: &'a LintSpecificConfig) -> Self {
        self.lint_specific = lint_specific;
        self
    }

    /// Returns an owned lint suite suitable for CLI or LSP execution.
    pub fn to_suite(&self) -> ForgeLintSuite {
        let lint_specific = Arc::new(self.lint_specific.clone());
        let mut registry = LintRegistry::new();
        high::register_lints(&mut registry, &lint_specific);
        med::register_lints(&mut registry, &lint_specific);
        low::register_lints(&mut registry, &lint_specific);
        info::register_lints(&mut registry, &lint_specific);
        gas::register_lints(&mut registry, &lint_specific);
        codesize::register_lints(&mut registry, &lint_specific);

        ForgeLintSuite {
            path_config: self.path_config.clone(),
            severity: self.severity.clone(),
            lints_included: self.lints_included.clone(),
            lints_excluded: self.lints_excluded.clone(),
            registry: Arc::new(registry),
            sources: None,
            run_active: None,
        }
    }
}

impl<'a> Linter for SolidityLinter<'a> {
    type Language = SolcLanguage;
    type Lint = SolLint;

    fn lint(
        &self,
        input: &[PathBuf],
        deny: DenyLevel,
        compiler: &mut Compiler,
    ) -> eyre::Result<()> {
        convert_solar_errors(compiler.dcx())?;

        // Cache diagnostic count before linting to isolate from the build phase.
        let mut warn_count_before = compiler.dcx().warn_count();
        let mut note_count_before = compiler.dcx().note_count();

        let ui_testing = std::env::var_os("FOUNDRY_LINT_UI_TESTING").is_some();

        let sm = compiler.sess().clone_source_map();
        let prev_emitter = compiler.dcx().set_emitter(if self.with_json_emitter {
            let writer: Box<dyn std::io::Write + Send> = if self.json_emitter_stdout && !ui_testing
            {
                Box::new(std::io::BufWriter::new(std::io::stdout()))
            } else {
                Box::new(std::io::BufWriter::new(std::io::stderr()))
            };
            let json_emitter = JsonEmitter::new(writer, sm, ColorChoice::Never)
                .rustc_like(true)
                .ui_testing(ui_testing);
            Box::new(json_emitter)
        } else {
            Box::new(HumanEmitter::stderr(Default::default()).source_map(Some(sm)))
        });
        let sess = compiler.sess_mut();
        sess.dcx.set_flags_mut(|f| f.track_diagnostics = false);
        if ui_testing {
            sess.opts.unstable.ui_testing = true;
            sess.reconfigure();
        }

        compiler.enter_mut(|compiler| -> eyre::Result<()> {
            if compiler.gcx().stage() < Some(solar::config::CompilerStage::Lowering) {
                let _ = compiler.lower_asts();
            }
            convert_solar_errors(compiler.dcx())?;
            if compiler.gcx().stage() < Some(solar::config::CompilerStage::Analysis) {
                // Typeck is used as a data source for lints. Its diagnostics are still
                // experimental and should not leak into `forge lint` output.
                let prev_emitter =
                    compiler.dcx().set_emitter(Box::new(SilentEmitter::new_boxed(None)));
                let _ = compiler.analysis();
                compiler.dcx().set_emitter(prev_emitter);
            }
            warn_count_before = compiler.dcx().warn_count();
            note_count_before = compiler.dcx().note_count();

            let gcx = compiler.gcx();
            let mut targets = Vec::with_capacity(input.len());
            for path in input {
                let path = self.path_config.root.join(path);
                if gcx.get_ast_source(&path).is_none() {
                    // Issue a warning rather than panicking when some input files use old
                    // Solidity versions that Solar does not support.
                    _ = sh_warn!("AST source not found for {}", path.display());
                } else {
                    targets.push(path);
                }
            }

            let mut suite = self.to_suite();
            let mut sources = targets
                .iter()
                .map(|path| {
                    let (_, source) =
                        gcx.get_ast_source(path).expect("lint target was validated above");
                    let ast = source.ast.as_ref().expect("lint target AST was validated above");
                    let comments =
                        Comments::new(&source.file, gcx.sess.source_map(), false, false, None);
                    SourceLintPolicy {
                        file: source.file.clone(),
                        inline: Arc::new(parse_inline_config(gcx.sess, &comments, ast)),
                        active: suite.active_lints(Some(path)),
                    }
                })
                .collect::<Vec<_>>();
            sources.sort_unstable_by_key(|source| source.file.start_pos);
            suite.run_active = Some(Arc::new(
                suite
                    .active_lints(None)
                    .into_iter()
                    .filter(|id| sources.iter().any(|source| source.active.contains(id)))
                    .collect(),
            ));
            suite.sources = Some(Arc::new(sources));
            run_lints(
                &suite,
                LintRunContext {
                    gcx,
                    targets: &targets,
                    with_description: self.with_description,
                    with_ansi_help: !self.with_json_emitter,
                },
            )
            .unwrap_or_else(|error| match error {
                LintRunError::MissingAstSource(path) => {
                    unreachable!("prevalidated AST source missing for {}", path.display())
                }
                LintRunError::MissingAst(path) => {
                    panic!("AST missing for {}", path.display())
                }
                LintRunError::MissingHir(path) => {
                    panic!("HIR source not found for {}", path.display())
                }
                error => panic!("lint run failed: {error}"),
            });

            Ok(())
        })?;

        let sess = compiler.sess_mut();
        sess.dcx.set_emitter(prev_emitter);
        if ui_testing {
            sess.opts.unstable.ui_testing = false;
            sess.reconfigure();
        }

        let lint_warn_count = compiler.dcx().warn_count().saturating_sub(warn_count_before);
        let lint_note_count = compiler.dcx().note_count().saturating_sub(note_count_before);

        let (w, n) = (lint_warn_count, lint_note_count);
        let denied = match deny {
            DenyLevel::Warnings if w > 0 && n > 0 => {
                format!("{w} linter warning(s); {n} note(s) were also emitted")
            }
            DenyLevel::Warnings if w > 0 => format!("{w} linter warning(s)"),
            DenyLevel::Notes if w > 0 && n > 0 => format!("{w} linter warning(s) and {n} note(s)"),
            DenyLevel::Notes if w > 0 => format!("{w} linter warning(s)"),
            DenyLevel::Notes if n > 0 => format!("{n} linter note(s)"),
            _ => return Ok(()),
        };
        Err(DeniedLintDiagnostics(format!(
            "aborting due to {denied}
"
        ))
        .into())
    }
}

fn parse_inline_config<'ast>(
    sess: &Session,
    comments: &Comments,
    ast: &'ast ast::SourceUnit<'ast>,
) -> InlineConfig<Vec<String>> {
    let items = comments.iter().filter_map(|comment| {
        let mut item = comment.lines.first()?.as_str();
        if let Some(prefix) = comment.prefix() {
            item = item.strip_prefix(prefix).unwrap_or(item);
        }
        if let Some(suffix) = comment.suffix() {
            item = item.strip_suffix(suffix).unwrap_or(item);
        }
        let item = item.trim_start().strip_prefix("forge-lint:")?.trim();
        let span = comment.span;
        match InlineConfigItem::parse(item, &ALL_REGISTERED_LINTS) {
            Ok(item) => Some((span, item)),
            Err(e) => {
                sess.dcx.warn(e.to_string()).span(span).emit();
                None
            }
        }
    });

    InlineConfig::from_ast(items, ast, sess.source_map())
}

#[derive(Error, Debug)]
pub enum SolLintError {
    #[error("Unknown lint ID: {0}")]
    InvalidId(String),
}

#[derive(Error, Debug)]
#[error("{0}")]
pub struct DeniedLintDiagnostics(String);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SolLint {
    id: &'static str,
    description: &'static str,
    diagnostic_help: Option<&'static str>,
    help: &'static str,
    severity: Severity,
}

impl SolLint {
    pub const fn severity(self) -> Severity {
        self.severity
    }
}

impl Lint for SolLint {
    fn id(&self) -> &'static str {
        self.id
    }
    fn level(&self) -> Level {
        self.severity.into()
    }
    fn description(&self) -> &'static str {
        self.description
    }
    fn diagnostic_help(&self) -> Option<&'static str> {
        self.diagnostic_help
    }
    fn help(&self) -> &'static str {
        self.help
    }
}

impl<'a> TryFrom<&'a str> for SolLint {
    type Error = SolLintError;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        all_lints()
            .find(|lint| lint.id == value)
            .copied()
            .ok_or_else(|| SolLintError::InvalidId(value.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks the documentation contract without treating example code as Markdown structure.
    fn follows_doc_template(content: &str, id: &str, severity: &str) -> bool {
        let mut lines = Vec::new();
        let mut fence = None;
        let mut has_code = false;
        for line in content.lines().map(str::trim) {
            if let Some(solidity) = fence {
                if line == "```" {
                    if solidity && has_code {
                        lines.push("```solidity");
                    }
                    fence = None;
                } else if !line.is_empty() {
                    has_code = true;
                }
            } else if line.starts_with("```") {
                fence = Some(line == "```solidity");
                has_code = false;
            } else if !line.is_empty() {
                lines.push(line);
            }
        }
        if fence.is_some() || !lines.first().is_some_and(|line| line.starts_with("# ")) {
            return false;
        }

        let rationales = lines
            .iter()
            .filter(|line| matches!(**line, "## Why is this bad?" | "## Why restrict this?"))
            .copied()
            .collect::<Vec<_>>();
        if rationales.len() != 1 {
            return false;
        }
        let required = [
            format!("**Severity**: `{severity}`"),
            format!("**ID**: `{id}`"),
            "## What it does".to_string(),
            rationales[0].to_string(),
            "## Example".to_string(),
        ];
        let mut offset = 0;
        for section in &required {
            if lines.iter().filter(|line| **line == section.as_str()).count() != 1 {
                return false;
            }
            let Some(index) = lines[offset..].iter().position(|line| *line == section.as_str())
            else {
                return false;
            };
            offset += index + 1;
            if section.starts_with("## ")
                && !lines.get(offset).is_some_and(|line| !line.starts_with('#'))
            {
                return false;
            }
        }
        let example = &lines[offset..];
        let end = example.iter().position(|line| line.starts_with("## ")).unwrap_or(example.len());
        let example = &example[..end];
        let Some(replacement) = example.iter().position(|line| *line == "Use instead:") else {
            return false;
        };
        example[..replacement].contains(&"```solidity")
            && example[replacement + 1..].contains(&"```solidity")
    }

    #[test]
    fn lint_doc_template_requires_real_sections_and_examples() {
        let doc = "# Example\n\n**Severity**: `Info`\n**ID**: `example`\n\nSummary.\n\n\
                   ## What it does\n\nChecks a pattern.\n\n\
                   ## Why is this bad?\n\nExplains the impact.\n\n\
                   ## Example\n\n```solidity\ncontract C {}\n```\n\n\
                   Use instead:\n\n```solidity\ncontract D {}\n```\n";
        assert!(follows_doc_template(doc, "example", "Info"));
        assert!(follows_doc_template(
            &doc.replace("Why is this bad?", "Why restrict this?"),
            "example",
            "Info"
        ));
        for invalid in [
            doc.replace("Checks a pattern.", ""),
            doc.replace("Explains the impact.", ""),
            doc.replace("Use instead:", "### Good"),
            doc.replace("contract C {}", "   "),
            doc.replace("contract D {}", ""),
            doc.replace("## What it does", "```text\n## What it does\n```"),
            doc.replace("## Why is this bad?", "## Why is this bad?\n## Why restrict this?"),
            doc.replace("**ID**: `example`", "**ID**: `other`"),
            doc.replace("**Severity**: `Info`", "**Severity**: `High`"),
        ] {
            assert!(!follows_doc_template(&invalid, "example", "Info"), "accepted {invalid}");
        }
    }

    /// Every registered lint must have a markdown documentation file at
    /// `crates/lint/docs/<str_id>.md` with matching metadata and the standard section structure.
    /// This test enforces that contract so that the `help` URL generated by `declare_forge_lint!`
    /// always resolves to valid documentation.
    ///
    /// When this test fails, add a new file at `crates/lint/docs/<str_id>.md` describing the
    /// lint. See [`crates/lint/docs/_template.md`](../../docs/_template.md) for the expected
    /// structure.
    #[test]
    fn registered_lints_have_docs() {
        let docs_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
        assert!(docs_dir.is_dir(), "missing docs directory at {}", docs_dir.display());

        let all_lints = all_lints().collect::<Vec<_>>();

        let registered_ids =
            all_lints.iter().map(|lint| lint.id()).collect::<std::collections::HashSet<_>>();
        let mut missing = Vec::new();
        let mut invalid = Vec::new();
        for lint in &all_lints {
            let path = docs_dir.join(format!("{}.md", lint.id()));
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let severity = format!("{:?}", lint.severity());
                    if !follows_doc_template(&content, lint.id(), &severity) {
                        invalid.push(lint.id());
                    }
                }
                Err(_) => missing.push(lint.id()),
            }
        }

        let mut unexpected = Vec::new();
        for entry in std::fs::read_dir(&docs_dir).expect("failed to read lint docs directory") {
            let path = entry.expect("failed to read lint docs entry").path();
            if path.extension().is_none_or(|extension| extension != "md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else { continue };
            if !matches!(stem, "README" | "_template") && !registered_ids.contains(stem) {
                unexpected.push(stem.to_string());
            }
        }

        assert!(
            missing.is_empty(),
            "the following registered lints are missing a docs file at \
             `crates/lint/docs/<id>.md`: {missing:?}\n\
             See `crates/lint/docs/_template.md` for the expected structure."
        );
        assert!(
            invalid.is_empty(),
            "the following lint docs do not match their registered ID/severity or the required \
             template structure: {invalid:?}"
        );
        assert!(
            unexpected.is_empty(),
            "the following lint docs do not correspond to a registered lint: {unexpected:?}"
        );
    }

    /// The auto-generated `help` URL must point at the canonical Foundry docs site so that the
    /// link printed in diagnostics resolves correctly.
    #[test]
    fn registered_lints_have_canonical_help_url() {
        for lint in all_lints() {
            let expected = format!("https://getfoundry.sh/forge/linting/{}", lint.id());
            assert_eq!(lint.help(), expected, "lint `{}` has a non-canonical help URL", lint.id());
        }
    }
}
