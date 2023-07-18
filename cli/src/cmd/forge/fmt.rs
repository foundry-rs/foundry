use crate::{
    cmd::{Cmd, LoadConfig},
    utils::FoundryPathExt,
};
use clap::{Parser, ValueHint};
use forge_fmt::{format, parse, print_diagnostics_report};
use foundry_common::{fs, term::cli_warn};
use foundry_config::impl_figment_convert_basic;
use foundry_utils::glob::expand_globs;
use rayon::prelude::*;
use similar::{ChangeTag, TextDiff};
use std::{
    fmt::{self, Write},
    io,
    io::{Read, Write as _},
    path::{Path, PathBuf},
};
use tracing::log::warn;
use yansi::Color;

/// CLI arguments for `forge fmt`.
#[derive(Debug, Clone, Parser)]
pub struct FmtArgs {
    /// Path to the file, directory or '-' to read from stdin.
    #[clap(value_hint = ValueHint::FilePath, value_name = "PATH", num_args(1..))]
    paths: Vec<PathBuf>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[clap(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Run in 'check' mode.
    ///
    /// Exits with 0 if input is formatted correctly.
    /// Exits with 1 if formatting is required.
    #[clap(long)]
    check: bool,

    /// In 'check' and stdin modes, outputs raw formatted code instead of the diff.
    #[clap(long, short)]
    raw: bool,
}

impl_figment_convert_basic!(FmtArgs);

// === impl FmtArgs ===

impl Cmd for FmtArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = self.try_load_config_emit_warnings()?;

        // Expand ignore globs
        let ignored = expand_globs(&config.__root.0, config.fmt.ignore.iter())?;

        let cwd = std::env::current_dir()?;
        let input = match &self.paths[..] {
            [] => Input::Paths(config.project_paths().input_files_iter().collect()),
            [one] if one == Path::new("-") => {
                let mut s = String::new();
                io::stdin().read_to_string(&mut s).expect("Failed to read from stdin");
                Input::Stdin(s)
            }
            paths => {
                let mut inputs = Vec::with_capacity(paths.len());
                for path in paths {
                    if !ignored.is_empty() &&
                        ((path.is_absolute() && ignored.contains(path)) ||
                            ignored.contains(&cwd.join(path)))
                    {
                        continue
                    }

                    if path.is_dir() {
                        inputs.extend(ethers::solc::utils::source_files_iter(path));
                    } else if path.is_sol() {
                        inputs.push(path.to_path_buf());
                    } else {
                        warn!("Cannot process path {}", path.display());
                    }
                }
                Input::Paths(inputs)
            }
        };

        let format = |source: String, path: Option<&Path>| -> eyre::Result<_> {
            let name = match path {
                Some(path) => {
                    path.strip_prefix(&config.__root.0).unwrap_or(&path).display().to_string()
                }
                None => "stdin".to_string(),
            };

            let parsed = parse(&source).map_err(|diagnostics| {
                let _ = print_diagnostics_report(&source, path, diagnostics);
                eyre::eyre!("Failed to parse Solidity code for {name}. Leaving source unchanged.")
            })?;

            if !parsed.invalid_inline_config_items.is_empty() {
                for (loc, warning) in &parsed.invalid_inline_config_items {
                    let mut lines = source[..loc.start().min(source.len())].split('\n');
                    let col = lines.next_back().unwrap().len() + 1;
                    let row = lines.count() + 1;
                    cli_warn!("[{}:{}:{}] {}", name, row, col, warning);
                }
            }

            let mut output = String::new();
            format(&mut output, parsed, config.fmt.clone()).unwrap();

            solang_parser::parse(&output, 0).map_err(|diags| {
                eyre::eyre!(
                    "Failed to construct valid Solidity code for {name}. Leaving source unchanged.\n\
                     Debug info: {diags:?}"
                )
            })?;

            if self.check || path.is_none() {
                if self.raw {
                    print!("{output}");
                }

                let diff = TextDiff::from_lines(&source, &output);
                if diff.ratio() < 1.0 {
                    return Ok(Some(format_diff_summary(&name, &diff)))
                }
            } else if let Some(path) = path {
                fs::write(path, output)?;
            }
            Ok(None)
        };

        let diffs = match input {
            Input::Stdin(source) => format(source, None).map(|diff| vec![diff]),
            Input::Paths(paths) => {
                if paths.is_empty() {
                    cli_warn!(
                        "Nothing to format.\n\
                         HINT: If you are working outside of the project, \
                         try providing paths to your source files: `forge fmt <paths>`"
                    );
                    return Ok(())
                }
                paths
                    .par_iter()
                    .map(|path| {
                        let source = fs::read_to_string(path)?;
                        format(source, Some(path))
                    })
                    .collect()
            }
        }?;

        let mut diffs = diffs.iter().flatten();
        if let Some(first) = diffs.next() {
            // This branch is only reachable with stdin or --check

            if !self.raw {
                let mut stdout = io::stdout().lock();
                let first = std::iter::once(first);
                for (i, diff) in first.chain(diffs).enumerate() {
                    if i > 0 {
                        let _ = stdout.write_all(b"\n");
                    }
                    let _ = stdout.write_all(diff.as_bytes());
                }
            }

            if self.check {
                std::process::exit(1);
            }
        }

        Ok(())
    }
}

struct Line(Option<usize>);

#[derive(Debug)]
enum Input {
    Stdin(String),
    Paths(Vec<PathBuf>),
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            None => f.write_str("    "),
            Some(idx) => write!(f, "{:<4}", idx + 1),
        }
    }
}

fn format_diff_summary<'a, 'b, 'r>(name: &str, diff: &'r TextDiff<'a, 'b, '_, str>) -> String
where
    'r: 'a + 'b,
{
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
                let dimmed = Color::Default.style().dimmed();
                let (sign, s) = match change.tag() {
                    ChangeTag::Delete => ("-", Color::Red.style()),
                    ChangeTag::Insert => ("+", Color::Green.style()),
                    ChangeTag::Equal => (" ", dimmed),
                };

                let _ = write!(
                    diff_summary,
                    "{}{} |{}",
                    dimmed.paint(Line(change.old_index())),
                    dimmed.paint(Line(change.new_index())),
                    s.bold().paint(sign),
                );

                for (emphasized, value) in change.iter_strings_lossy() {
                    let s = if emphasized { s.underline().bg(Color::Black) } else { s };
                    let _ = write!(diff_summary, "{}", s.paint(value));
                }

                if change.missing_newline() {
                    diff_summary.push('\n');
                }
            }
        }
    }

    diff_summary
}
