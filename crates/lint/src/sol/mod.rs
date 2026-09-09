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
    //! Checks the canonical lint documentation against the registered lints and page template.

    use super::{Severity, all_lints};
    use crate::linter::Lint;
    use eyre::{Result, ensure};
    use std::{collections::BTreeSet, fs, path::Path};

    #[test]
    fn registered_lints_have_docs() {
        let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
        let mut registered = BTreeSet::new();
        let mut errors = Vec::new();
        for lint in all_lints() {
            assert!(registered.insert(lint.id().to_owned()), "duplicate lint ID: {}", lint.id());
            let path = docs.join(format!("{}.md", lint.id()));
            let result = fs::read_to_string(&path)
                .map_err(eyre::Report::from)
                .and_then(|text| validate_doc(&text, lint.id(), lint.severity()));
            if let Err(error) = result {
                errors.push(format!("{}: {error}", path.display()));
            }
        }
        assert!(!registered.is_empty(), "no registered lints");
        let documented = fs::read_dir(&docs)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
            .map(|path| path.file_stem().unwrap().to_str().unwrap().to_owned())
            .filter(|id| !matches!(id.as_str(), "README" | "_template"))
            .collect::<BTreeSet<_>>();
        for id in documented.difference(&registered) {
            errors.push(format!("{id}.md: no registered lint"));
        }
        assert!(errors.is_empty(), "invalid lint documentation:\n{}", errors.join("\n"));
    }

    #[test]
    fn registered_lints_have_canonical_help_url() {
        for lint in all_lints() {
            let expected = format!("https://getfoundry.sh/forge/linting/{}", lint.id());
            assert_eq!(lint.help(), expected, "lint `{}` has a non-canonical help URL", lint.id());
        }
    }

    #[derive(Debug, PartialEq)]
    enum Kind<'a> {
        Heading(usize),
        Text,
        Code(&'a str),
    }

    #[derive(Debug)]
    struct Token<'a> {
        kind: Kind<'a>,
        text: String,
        line: usize,
    }

    fn fence(line: &str) -> Option<(u8, usize, &str)> {
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() > 3 {
            return None;
        }
        let marker = *trimmed.as_bytes().first()?;
        if !matches!(marker, b'`' | b'~') {
            return None;
        }
        let len = trimmed.bytes().take_while(|&byte| byte == marker).count();
        (len >= 3).then(|| (marker, len, trimmed[len..].trim()))
    }

    // Fenced examples can contain headings, metadata, and shorter fences; ignore that content.
    fn tokens(text: &str) -> Result<Vec<Token<'_>>> {
        let mut result = Vec::new();
        let mut lines = text.lines().enumerate();
        while let Some((index, line)) = lines.next() {
            let (kind, text) = if let Some((marker, len, language)) = fence(line) {
                let mut code = String::new();
                let mut closed = false;
                for (_, line) in lines.by_ref() {
                    if fence(line).is_some_and(|(end, width, tail)| {
                        end == marker && width >= len && tail.is_empty()
                    }) {
                        closed = true;
                        break;
                    }
                    code.push_str(line);
                    code.push('\n');
                }
                ensure!(closed, "line {}: unclosed code fence", index + 1);
                (Kind::Code(language), code)
            } else {
                let level = line.bytes().take_while(|&byte| byte == b'#').count();
                if (1..=6).contains(&level) && line[level..].starts_with(' ') {
                    (Kind::Heading(level), line[level..].trim().to_owned())
                } else if line.trim().is_empty() {
                    continue;
                } else {
                    (Kind::Text, line.trim().to_owned())
                }
            };
            result.push(Token { kind, text, line: index + 1 });
        }
        Ok(result)
    }

    fn validate_doc(text: &str, id: &str, severity: Severity) -> Result<()> {
        ensure!(
            id.starts_with(|c: char| c.is_ascii_lowercase())
                && id.split('-').all(|part| !part.is_empty()
                    && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())),
            "lint ID must be kebab-case"
        );
        let items = tokens(text)?;
        let [title, level, identity, body @ ..] = items.as_slice() else {
            eyre::bail!("expected title, severity, ID, and sections");
        };
        ensure!(title.kind == Kind::Heading(1) && !title.text.is_empty(), "start with one # title");
        ensure!(
            level.kind == Kind::Text && level.text == format!("**Severity**: `{severity:?}`"),
            "line {}: severity must match the registered lint ({severity:?})",
            level.line
        );
        ensure!(
            identity.kind == Kind::Text && identity.text == format!("**ID**: `{id}`"),
            "line {}: ID must match the registered lint and filename ({id})",
            identity.line
        );
        ensure!(
            body.first().is_some_and(|t| t.kind == Kind::Heading(2) && t.text == "What it does"),
            "start the body with ## What it does (no introductory summary)"
        );
        let mut core = Vec::new();
        let mut seen = BTreeSet::new();
        let mut remaining = body;
        while let Some((heading, tail)) = remaining.split_first() {
            let end = tail.iter().position(|t| matches!(t.kind, Kind::Heading(1 | 2)));
            let (content, rest) = tail.split_at(end.unwrap_or(tail.len()));
            remaining = rest;
            ensure!(heading.kind == Kind::Heading(2), "line {}: only one # title", heading.line);
            let name = heading.text.as_str();
            ensure!(seen.insert(name), "line {}: duplicate section {name}", heading.line);
            match name {
                "What it does" | "Why is this bad?" | "Why restrict this?" | "Example" => {
                    core.push(name);
                }
                "Configuration" | "Notes" | "Limitations" | "Known limitations" => {}
                _ => eyre::bail!("line {}: unexpected section {name}", heading.line),
            }
            ensure!(
                content
                    .iter()
                    .any(|t| !matches!(t.kind, Kind::Heading(_)) && !t.text.trim().is_empty()),
                "line {}: {name} must not be empty",
                heading.line
            );
            for token in content {
                ensure!(
                    !(matches!(token.kind, Kind::Heading(_))
                        && matches!(token.text.as_str(), "Bad" | "Good")),
                    "line {}: use Use instead: rather than Bad/Good headings",
                    token.line
                );
                ensure!(
                    !(token.kind == Kind::Text
                        && (token.text.starts_with("**Severity**:")
                            || token.text.starts_with("**ID**:"))),
                    "line {}: metadata belongs only below the title",
                    token.line
                );
            }
            if matches!(name, "What it does" | "Why is this bad?" | "Why restrict this?") {
                ensure!(
                    content.iter().any(|t| t.kind == Kind::Text),
                    "{name} needs explanatory prose"
                );
            }
            if name == "Example" {
                let separators = content
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.kind == Kind::Text && t.text == "Use instead:")
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                ensure!(separators.len() == 1, "Example needs exactly one Use instead: separator");
                let split = separators[0];
                for side in [&content[..split], &content[split + 1..]] {
                    ensure!(
                        side.iter()
                            .any(|t| t.kind == Kind::Code("solidity") && !t.text.trim().is_empty()),
                        "Example needs a nonempty solidity code block on each side of Use instead:"
                    );
                }
            }
        }
        ensure!(
            matches!(
                core.as_slice(),
                ["What it does", "Why is this bad?" | "Why restrict this?", "Example"]
            ),
            "expected What it does, exactly one Why section, then Example"
        );
        Ok(())
    }

    const VALID: &str = "# Example\n\n**Severity**: `Info`\n**ID**: `example`\n\n\
        ## What it does\n\nReports a pattern.\n\n## Why is this bad?\n\nExplains the consequence.\n\n\
        ## Example\n\n```solidity\nbad();\n```\n\nUse instead:\n\n```solidity\ngood();\n```\n";

    #[test]
    fn accepts_documentation_variants() {
        for severity in [
            Severity::High,
            Severity::Med,
            Severity::Low,
            Severity::Info,
            Severity::Gas,
            Severity::CodeSize,
        ] {
            validate_doc(&VALID.replace("`Info`", &format!("`{severity:?}`")), "example", severity)
                .unwrap();
        }
        for text in [
            VALID.replace("Why is this bad?", "Why restrict this?"),
            VALID.replace('\n', "\r\n"),
            VALID.replace("## Why", "## Known limitations\n\nAn exclusion.\n\n## Why")
                + "\n## Configuration\n\n```toml\nsetting = true\n```\n\n## Notes\n\nA note.\n\n## Limitations\n\nA limitation.\n",
            include_str!("../../docs/_template.md")
                .replace("`<High | Med | Low | Info | Gas | CodeSize>`", "`Info`")
                .replace("`<str_id>`", "`example`"),
        ] {
            validate_doc(&text, "example", Severity::Info).unwrap();
        }
        for marker in ["````", "~~~"] {
            let text = VALID
                .replace("```", marker)
                .replace("bad();", "## Example\n**ID**: `other`\nUse instead:\n### Bad\n```");
            validate_doc(&text, "example", Severity::Info).unwrap();
        }
    }

    #[test]
    fn rejects_invalid_documentation() {
        for (from, to, error) in [
            ("# Example", "Example", "one # title"),
            ("**Severity**: `Info`", "", "severity"),
            ("`Info`", "`High`", "severity"),
            ("`example`", "`other`", "ID must match"),
            ("## What", "Summary.\n\n## What", "no introductory summary"),
            ("## Why is this bad?\n\nExplains the consequence.\n\n", "", "exactly one Why"),
            ("Reports a pattern.", "", "must not be empty"),
            ("Reports a pattern.", "```solidity\nf();\n```", "explanatory prose"),
            ("Use instead:", "", "exactly one Use instead:"),
            ("Use instead:", "### Bad", "Bad/Good headings"),
            ("Use instead:", "### Good", "Bad/Good headings"),
            ("bad();", "", "nonempty solidity"),
            ("good();", " ", "nonempty solidity"),
            ("```solidity", "```text", "nonempty solidity"),
            ("```solidity", "~~~solidity", "unclosed code fence"),
            ("```solidity", "````solidity", "unclosed code fence"),
        ] {
            let text = VALID.replacen(from, to, 1);
            let result = validate_doc(&text, "example", Severity::Info).unwrap_err().to_string();
            assert!(result.contains(error), "{from:?} -> {to:?}: {result}");
        }
        for (suffix, error) in [
            ("# Extra\n", "only one # title"),
            ("## Example\n", "duplicate section"),
            ("## Why restrict this?\n\nPolicy.\n", "exactly one Why"),
            ("## Scope and controls\n\nText.\n", "unexpected section"),
            ("## Notes\n\n### Detail\n", "must not be empty"),
            ("**ID**: `example`\n", "metadata belongs only"),
            ("Use instead:\n", "exactly one Use instead:"),
        ] {
            let result = validate_doc(&(VALID.to_owned() + suffix), "example", Severity::Info)
                .unwrap_err()
                .to_string();
            assert!(result.contains(error), "{suffix:?}: {result}");
        }
        let reordered = VALID
            .replace("## Why is this bad?", "## Temporary")
            .replace("## Example", "## Why is this bad?")
            .replace("## Temporary", "## Example");
        assert!(validate_doc(&reordered, "example", Severity::Info).is_err());
        assert!(
            validate_doc(VALID.trim_end().trim_end_matches("```"), "example", Severity::Info)
                .is_err()
        );
        for id in ["Not_kebab", "example-", "example--lint", "1example"] {
            let text = VALID.replace("`example`", &format!("`{id}`"));
            assert!(validate_doc(&text, id, Severity::Info).is_err(), "{id}");
        }
    }
}
