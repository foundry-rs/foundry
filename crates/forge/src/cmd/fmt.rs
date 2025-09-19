use super::watch::WatchArgs;
use clap::{Parser, ValueHint};
use eyre::{Context, Result};
use foundry_cli::utils::{FoundryPathExt, LoadConfig};
use foundry_common::fs;
use foundry_compilers::{compilers::solc::SolcLanguage, solc::SOLC_EXTENSIONS};
use foundry_config::{filter::expand_globs, impl_figment_convert_basic};
use rayon::prelude::*;
use similar::{ChangeTag, TextDiff};
use solar::sema::Compiler;
use std::{
    fmt::{self, Write},
    io,
    io::{Read, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
};
use yansi::{Color, Paint, Style};

/// CLI arguments for `forge fmt`.
#[derive(Clone, Debug, Parser)]
pub struct FmtArgs {
    /// Path to the file, directory or '-' to read from stdin.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH", num_args(1..))]
    paths: Vec<PathBuf>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Run in 'check' mode.
    ///
    /// Exits with 0 if input is formatted correctly.
    /// Exits with 1 if formatting is required.
    #[arg(long)]
    check: bool,

    /// In 'check' and stdin modes, outputs raw formatted code instead of the diff.
    #[arg(long, short)]
    raw: bool,

    #[command(flatten)]
    pub watch: WatchArgs,
}

impl_figment_convert_basic!(FmtArgs);

impl FmtArgs {
    pub fn run(self) -> Result<()> {
        let config = self.load_config()?;

        // Expand ignore globs and canonicalize from the get go
        let ignored = expand_globs(&config.root, config.fmt.ignore.iter())?
            .iter()
            .flat_map(foundry_common::fs::canonicalize_path)
            .collect::<Vec<_>>();

        let cwd = std::env::current_dir()?;
        let input = match &self.paths[..] {
            [] => {
                // Retrieve the project paths, and filter out the ignored ones.
                let project_paths: Vec<PathBuf> = config
                    .project_paths::<SolcLanguage>()
                    .input_files_iter()
                    .filter(|p| !(ignored.contains(p) || ignored.contains(&cwd.join(p))))
                    .collect();
                Input::Paths(project_paths)
            }
            [one] if one == Path::new("-") => {
                let mut s = String::new();
                io::stdin().read_to_string(&mut s).wrap_err("failed to read from stdin")?;
                Input::Stdin(s)
            }
            paths => {
                let mut inputs = Vec::with_capacity(paths.len());
                for path in paths {
                    if !ignored.is_empty()
                        && ((path.is_absolute() && ignored.contains(path))
                            || ignored.contains(&cwd.join(path)))
                    {
                        continue;
                    }

                    if path.is_dir() {
                        inputs.extend(foundry_compilers::utils::source_files_iter(
                            path,
                            SOLC_EXTENSIONS,
                        ));
                    } else if path.is_sol() {
                        inputs.push(path.to_path_buf());
                    } else {
                        warn!("Cannot process path {}", path.display());
                    }
                }
                Input::Paths(inputs)
            }
        };

        // Handle stdin on its own
        if let Input::Stdin(original) = input {
            let formatted = forge_fmt::format(&original, config.fmt)
                .into_result()
                .map_err(|_| eyre::eyre!("failed to format stdin"))?;

            let diff = TextDiff::from_lines(&original, &formatted);
            if diff.ratio() < 1.0 {
                if self.raw {
                    sh_print!("{formatted}")?;
                } else {
                    sh_print!("{}", format_diff_summary("stdin", &diff))?;
                }
                if self.check {
                    std::process::exit(1);
                }
            }
            return Ok(());
        }

        // Unwrap and check input paths
        let paths_to_fmt = match input {
            Input::Paths(paths) => {
                if paths.is_empty() {
                    sh_warn!(
                        "Nothing to format.\n\
                    HINT: If you are working outside of the project, \
                    try providing paths to your source files: `forge fmt <paths>`"
                    )?;
                    return Ok(());
                }
                paths
            }
            Input::Stdin(_) => unreachable!(),
        };

        let mut compiler = Compiler::new(
            solar::interface::Session::builder().with_buffer_emitter(Default::default()).build(),
        );

        // Parse, format, and check the diffs.
        let res = compiler.enter_mut(|c| -> Result<()> {
            let mut pcx = c.parse();
            pcx.set_resolve_imports(false);
            let _ = pcx.par_load_files(paths_to_fmt);
            pcx.parse();

            let gcx = c.gcx();
            let fmt_config = Arc::new(config.fmt);
            let diffs: Vec<String> = gcx
                .sources
                .raw
                .par_iter()
                .filter_map(|source_unit| {
                    let path = source_unit.file.name.as_real()?;
                    let original = &source_unit.file.src;
                    let formatted = forge_fmt::format_ast(gcx, source_unit, fmt_config.clone())?;

                    if original.as_str() == formatted {
                        return None;
                    }

                    if self.check {
                        let name =
                            path.strip_prefix(&config.root).unwrap_or(path).display().to_string();
                        let summary = format_diff_summary(
                            &name,
                            &TextDiff::from_lines(original.as_str(), &formatted),
                        );
                        Some(Ok(summary))
                    } else {
                        match fs::write(path, formatted) {
                            Ok(_) => {
                                let _ = sh_println!("Formatted {}", path.display());
                                None
                            }
                            Err(e) => Some(Err(eyre::eyre!(
                                "Failed to write to {}: {}",
                                path.display(),
                                e
                            ))),
                        }
                    }
                })
                .collect::<Result<_>>()?;

            if !diffs.is_empty() {
                // This block is only reached in --check mode when files need formatting.
                let mut stdout = io::stdout().lock();
                for (i, diff) in diffs.iter().enumerate() {
                    if i > 0 {
                        let _ = stdout.write_all(b"\n");
                    }
                    let _ = stdout.write_all(diff.as_bytes());
                }
                std::process::exit(1);
            }
            Ok(())
        });
        res?;

        // TODO(dani): convert solar errors

        Ok(())
    }

    /// Returns whether `FmtArgs` was configured with `--watch`
    pub fn is_watch(&self) -> bool {
        self.watch.watch.is_some()
    }
}

struct Line(Option<usize>);

#[derive(Debug)]
enum Input {
    Stdin(String),
    Paths(Vec<PathBuf>),
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            None => f.write_str("    "),
            Some(idx) => write!(f, "{:<4}", idx + 1),
        }
    }
}

fn format_diff_summary<'a>(name: &str, diff: &'a TextDiff<'a, 'a, '_, str>) -> String {
    let cap = 128;
    let mut diff_summary = String::with_capacity(cap);

    let _ = writeln!(diff_summary, "Diff in {name}:");
    for (j, group) in diff.grouped_ops(3).into_iter().enumerate() {
        if j > 0 {
            let s =
                "--------------------------------------------------------------------------------";
            diff_summary.push_str(s);
        }
        for op in group {
            for change in diff.iter_inline_changes(&op) {
                let dimmed = Style::new().dim();
                let (sign, s) = match change.tag() {
                    ChangeTag::Delete => ("-", Color::Red.foreground()),
                    ChangeTag::Insert => ("+", Color::Green.foreground()),
                    ChangeTag::Equal => (" ", dimmed),
                };

                let _ = write!(
                    diff_summary,
                    "{}{} |{}",
                    Line(change.old_index()).paint(dimmed),
                    Line(change.new_index()).paint(dimmed),
                    sign.paint(s.bold()),
                );

                for (emphasized, value) in change.iter_strings_lossy() {
                    let s = if emphasized { s.underline().bg(Color::Black) } else { s };
                    let _ = write!(diff_summary, "{}", value.paint(s));
                }

                if change.missing_newline() {
                    diff_summary.push('\n');
                }
            }
        }
    }

    diff_summary
}
