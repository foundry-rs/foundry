use super::watch::WatchArgs;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::utils::{FoundryPathExt, LoadConfig};
use foundry_common::{errors::convert_solar_errors, fs};
use foundry_compilers::{compilers::solc::SolcLanguage, solc::SOLC_EXTENSIONS};
use foundry_config::{filter::expand_globs, impl_figment_convert_basic};
use rayon::prelude::*;
use similar::{ChangeTag, TextDiff};
use solar::sema::Compiler;
use std::{
    fmt::{self, Write},
    io,
    io::Write as _,
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
        let cwd = std::env::current_dir()?;

        // Expand ignore globs and canonicalize from the get go
        let ignored = expand_globs(&config.root, config.fmt.ignore.iter())?
            .iter()
            .flat_map(fs::canonicalize_path)
            .collect::<Vec<_>>();

        // Expand lib globs separately - we only exclude these during discovery, not explicit paths
        let libs = expand_globs(&config.root, config.libs.iter().filter_map(|p| p.to_str()))?
            .iter()
            .flat_map(fs::canonicalize_path)
            .collect::<Vec<_>>();

        // Helper to check if a file path is under any ignored or lib directory
        let is_under_ignored_dir = |file_path: &Path, include_libs: bool| -> bool {
            let check_against_dir = |dir: &PathBuf| {
                file_path.starts_with(dir)
                    || cwd.join(file_path).starts_with(dir)
                    || fs::canonicalize_path(file_path).is_ok_and(|p| p.starts_with(dir))
            };

            ignored.iter().any(&check_against_dir)
                || (include_libs && libs.iter().any(&check_against_dir))
        };

        let input = match &self.paths[..] {
            [] => {
                // Retrieve the project paths, and filter out the ignored ones and libs.
                let project_paths: Vec<PathBuf> = config
                    .project_paths::<SolcLanguage>()
                    .input_files_iter()
                    .filter(|p| {
                        !(ignored.contains(p)
                            || ignored.contains(&cwd.join(p))
                            || is_under_ignored_dir(p, true))
                    })
                    .collect();
                Input::Paths(project_paths)
            }
            [one] if one == Path::new("-") => Input::Stdin,
            paths => {
                let mut inputs = Vec::with_capacity(paths.len());
                for path in paths {
                    // Check if path is in ignored directories
                    if !ignored.is_empty()
                        && ((path.is_absolute() && ignored.contains(path))
                            || ignored.contains(&cwd.join(path)))
                    {
                        continue;
                    }

                    if path.is_dir() {
                        // If the input directory is not a lib directory, make sure to ignore libs.
                        let exclude_libs = !is_under_ignored_dir(path, true);
                        inputs.extend(
                            foundry_compilers::utils::source_files_iter(path, SOLC_EXTENSIONS)
                                .filter(|p| {
                                    !(ignored.contains(p)
                                        || ignored.contains(&cwd.join(p))
                                        || is_under_ignored_dir(p, exclude_libs))
                                }),
                        );
                    } else if path.is_sol() {
                        // Explicit file paths are always included, even if in a lib
                        inputs.push(path.to_path_buf());
                    } else {
                        warn!("Cannot process path {}", path.display());
                    }
                }
                Input::Paths(inputs)
            }
        };

        let mut compiler = Compiler::new(
            solar::interface::Session::builder().with_buffer_emitter(Default::default()).build(),
        );

        // Parse, format, and check the diffs.
        compiler.enter_mut(|compiler| {
            let mut pcx = compiler.parse();
            pcx.set_resolve_imports(false);
            match input {
                Input::Paths(paths) if paths.is_empty() => {
                    sh_warn!(
                        "Nothing to format.\n\
                         HINT: If you are working outside of the project, \
                         try providing paths to your source files: `forge fmt <paths>`"
                    )?;
                    return Ok(());
                }
                Input::Paths(paths) => _ = pcx.par_load_files(paths),
                Input::Stdin => _ = pcx.load_stdin(),
            }
            pcx.parse();

            let gcx = compiler.gcx();
            let fmt_config = Arc::new(config.fmt);
            let diffs: Vec<String> = gcx
                .sources
                .raw
                .par_iter()
                .filter_map(|source_unit| {
                    let path = source_unit.file.name.as_real();
                    let original = source_unit.file.src.as_str();
                    let formatted = forge_fmt::format_ast(gcx, source_unit, fmt_config.clone())?;
                    let from_stdin = path.is_none();

                    // Return formatted code when read from stdin and raw enabled.
                    // <https://github.com/foundry-rs/foundry/issues/11871>
                    if from_stdin && self.raw {
                        return Some(Ok(formatted));
                    }

                    if original == formatted {
                        return None;
                    }

                    if self.check || from_stdin {
                        let summary = if self.raw {
                            formatted
                        } else {
                            let name = match path {
                                Some(path) => path
                                    .strip_prefix(&config.root)
                                    .unwrap_or(path)
                                    .display()
                                    .to_string(),
                                None => "stdin".to_string(),
                            };
                            format_diff_summary(&name, &TextDiff::from_lines(original, &formatted))
                        };
                        Some(Ok(summary))
                    } else if let Some(path) = path {
                        match fs::write(path, formatted) {
                            Ok(()) => {}
                            Err(e) => return Some(Err(e.into())),
                        }
                        let _ = sh_println!("Formatted {}", path.display());
                        None
                    } else {
                        unreachable!()
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
                if self.check {
                    std::process::exit(1);
                }
            }

            convert_solar_errors(compiler.dcx())
        })
    }

    /// Returns whether `FmtArgs` was configured with `--watch`
    pub fn is_watch(&self) -> bool {
        self.watch.watch.is_some()
    }
}

#[derive(Debug)]
enum Input {
    Stdin,
    Paths(Vec<PathBuf>),
}

struct Line(Option<usize>);

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
