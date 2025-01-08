use clap::{Parser, ValueHint};
use eyre::{Result, WrapErr};
use foundry_cli::utils::LoadConfig;
use foundry_compilers::{resolver::parse::SolData, Graph};
use foundry_config::{impl_figment_convert_basic, Config};
use itertools::Itertools;
use solar_parse::{ast, ast::visit::Visit, interface::Session};
use std::{
    ops::ControlFlow,
    path::{Path, PathBuf},
};

/// CLI arguments for `forge geiger`.
#[derive(Clone, Debug, Parser)]
pub struct GeigerArgs {
    /// Paths to files or directories to detect.
    #[arg(
        conflicts_with = "root",
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
        num_args(1..),
    )]
    paths: Vec<PathBuf>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Globs to ignore.
    #[arg(
        long,
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
        num_args(1..),
    )]
    ignore: Vec<PathBuf>,

    #[arg(long, hide = true)]
    check: bool,
    #[arg(long, hide = true)]
    full: bool,
}

impl_figment_convert_basic!(GeigerArgs);

impl GeigerArgs {
    pub fn sources(&self, config: &Config) -> Result<Vec<PathBuf>> {
        let cwd = std::env::current_dir()?;

        let mut sources: Vec<PathBuf> = {
            if self.paths.is_empty() {
                let paths = config.project_paths();
                Graph::<SolData>::resolve(&paths)?
                    .files()
                    .keys()
                    .filter(|f| !paths.has_library_ancestor(f))
                    .cloned()
                    .collect()
            } else {
                self.paths
                    .iter()
                    .flat_map(|path| foundry_common::fs::files_with_ext(path, "sol"))
                    .unique()
                    .collect()
            }
        };

        sources.retain_mut(|path| {
            let abs_path = if path.is_absolute() { path.clone() } else { cwd.join(&path) };
            *path = abs_path.strip_prefix(&cwd).unwrap_or(&abs_path).to_path_buf();
            !self.ignore.iter().any(|ignore| {
                if ignore.is_absolute() {
                    abs_path.starts_with(ignore)
                } else {
                    abs_path.starts_with(cwd.join(ignore))
                }
            })
        });

        Ok(sources)
    }

    pub fn run(self) -> Result<usize> {
        if self.check {
            sh_warn!("`--check` is deprecated as it's now the default behavior\n")?;
        }
        if self.full {
            sh_warn!("`--full` is deprecated as reports are not generated anymore\n")?;
        }

        let config = self.try_load_config_emit_warnings()?;
        let sources = self.sources(&config).wrap_err("Failed to resolve files")?;

        if config.ffi {
            sh_warn!("FFI enabled\n")?;
        }

        let mut sess = Session::builder().with_stderr_emitter().build();
        sess.dcx = sess.dcx.set_flags(|flags| flags.track_diagnostics = false);
        let unsafe_cheatcodes = &[
            "ffi".to_string(),
            "readFile".to_string(),
            "readLine".to_string(),
            "writeFile".to_string(),
            "writeLine".to_string(),
            "removeFile".to_string(),
            "closeFile".to_string(),
            "setEnv".to_string(),
            "deriveKey".to_string(),
        ];
        Ok(sess
            .enter(|| sources.iter().map(|file| lint_file(&sess, unsafe_cheatcodes, file)).sum()))
    }
}

fn lint_file(sess: &Session, unsafe_cheatcodes: &[String], path: &Path) -> usize {
    try_lint_file(sess, unsafe_cheatcodes, path).unwrap_or(0)
}

fn try_lint_file(
    sess: &Session,
    unsafe_cheatcodes: &[String],
    path: &Path,
) -> solar_parse::interface::Result<usize> {
    let arena = solar_parse::ast::Arena::new();
    let mut parser = solar_parse::Parser::from_file(sess, &arena, path)?;
    let ast = parser.parse_file().map_err(|e| e.emit())?;
    let mut visitor = Visitor::new(sess, unsafe_cheatcodes);
    visitor.visit_source_unit(&ast);
    Ok(visitor.count)
}

struct Visitor<'a> {
    sess: &'a Session,
    count: usize,
    unsafe_cheatcodes: &'a [String],
}

impl<'a> Visitor<'a> {
    fn new(sess: &'a Session, unsafe_cheatcodes: &'a [String]) -> Self {
        Self { sess, count: 0, unsafe_cheatcodes }
    }
}

impl<'ast> Visit<'ast> for Visitor<'_> {
    type BreakValue = solar_parse::interface::data_structures::Never;

    fn visit_expr(&mut self, expr: &'ast ast::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        if let ast::ExprKind::Call(lhs, _args) = &expr.kind {
            if let ast::ExprKind::Member(_lhs, member) = &lhs.kind {
                if self.unsafe_cheatcodes.iter().any(|c| c.as_str() == member.as_str()) {
                    let msg = format!("usage of unsafe cheatcode `vm.{member}`");
                    self.sess.dcx.err(msg).span(member.span).emit();
                    self.count += 1;
                }
            }
        }
        self.walk_expr(expr)
    }
}
