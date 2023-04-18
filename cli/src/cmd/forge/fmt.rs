use crate::{
    cmd::{Cmd, LoadConfig},
    utils::FoundryPathExt,
};
use clap::{Parser, ValueHint};
use console::{style, Style};
use forge_fmt::{format, parse, print_diagnostics_report};
use foundry_common::{fs, term::cli_warn};
use foundry_config::{impl_figment_convert_basic, Config};
use foundry_utils::glob::expand_globs;
use rayon::prelude::*;
use similar::{ChangeTag, TextDiff};
use std::{
    fmt::{self, Write},
    io,
    io::Read,
    path::{Path, PathBuf},
};
use tracing::log::warn;

/// CLI arguments for `forge fmt`.
#[derive(Debug, Clone, Parser)]
pub struct FmtArgs {
    #[clap(
        help = "path to the file, directory or '-' to read from stdin",
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
        num_args(1..)
    )]
    paths: Vec<PathBuf>,
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[clap(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,
    #[clap(
        help = "run in 'check' mode. Exits with 0 if input is formatted correctly. Exits with 1 if formatting is required.",
        long
    )]
    check: bool,
    #[clap(
        help = "in 'check' and stdin modes, outputs raw formatted code instead of diff",
        long = "raw",
        short
    )]
    raw: bool,
}

impl_figment_convert_basic!(FmtArgs);

// === impl FmtArgs ===

impl FmtArgs {
    /// Returns all inputs to format
    fn inputs(&self, config: &Config) -> Vec<Input> {
        if self.paths.is_empty() {
            return config.project_paths().input_files().into_iter().map(Input::Path).collect()
        }

        let mut paths = self.paths.iter().peekable();

        if let Some(path) = paths.peek() {
            let mut stdin = io::stdin();
            if *path == Path::new("-") && !is_terminal::is_terminal(&stdin) {
                let mut buf = String::new();
                stdin.read_to_string(&mut buf).expect("Failed to read from stdin");
                return vec![Input::Stdin(buf)]
            }
        }

        let mut out = Vec::with_capacity(self.paths.len());
        for path in self.paths.iter() {
            if path.is_dir() {
                out.extend(ethers::solc::utils::source_files(path).into_iter().map(Input::Path));
            } else if path.is_sol() {
                out.push(Input::Path(path.to_path_buf()));
            } else {
                warn!("Cannot process path {}", path.display());
            }
        }
        out
    }
}

impl Cmd for FmtArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = self.try_load_config_emit_warnings()?;

        // Expand ignore globs
        let ignored = expand_globs(&config.__root.0, config.fmt.ignore.iter())?;

        let cwd = std::env::current_dir()?;
        let mut inputs = vec![];
        for input in self.inputs(&config) {
            match input {
                Input::Path(p) => {
                    if (p.is_absolute() && !ignored.contains(&p)) ||
                        !ignored.contains(&cwd.join(&p))
                    {
                        inputs.push(Input::Path(p));
                    }
                }
                other => inputs.push(other),
            };
        }

        if inputs.is_empty() {
            cli_warn!("Nothing to format.\nHINT: If you are working outside of the project, try providing paths to your source files: `forge fmt <paths>`");
            return Ok(())
        }

        let diffs = inputs
            .par_iter()
            .map(|input| {
                let source = match input {
                    Input::Path(path) => fs::read_to_string(path)?,
                    Input::Stdin(source) => source.to_string()
                };

                let parsed = match parse(&source) {
                    Ok(result) => result,
                    Err(diagnostics) => {
                        let path = if let Input::Path(path) = input {Some(path)} else {None};
                        print_diagnostics_report(&source,path,  diagnostics)?;
                        eyre::bail!(
                            "Failed to parse Solidity code for {input}. Leaving source unchanged."
                        )
                    }
                };

                if !parsed.invalid_inline_config_items.is_empty() {
                    let path = match input {
                        Input::Path(path) => {
                            let path = path.strip_prefix(&config.__root.0).unwrap_or(path);
                            format!("{}", path.display())
                        }
                        Input::Stdin(_) => "stdin".to_string()
                    };
                    for (loc, warning) in &parsed.invalid_inline_config_items {
                        let mut lines = source[..loc.start().min(source.len())].split('\n');
                        let col = lines.next_back().unwrap().len() + 1;
                        let row = lines.count() + 1;
                        cli_warn!("[{}:{}:{}] {}", path, row, col, warning);
                    }
                }

                let mut output = String::new();
                format(&mut output, parsed, config.fmt.clone()).unwrap();

                solang_parser::parse(&output, 0).map_err(|diags| {
                    eyre::eyre!(
                            "Failed to construct valid Solidity code for {}. Leaving source unchanged.\nDebug info: {:?}",
                            input,
                            diags
                        )
                })?;

                if self.check || matches!(input, Input::Stdin(_)) {
                    if self.raw {
                        print!("{output}");
                    }

                    let diff = TextDiff::from_lines(&source, &output);

                    if diff.ratio() < 1.0 {
                        let mut diff_summary = String::new();

                        writeln!(diff_summary, "Diff in {input}:")?;
                        for (j, group) in diff.grouped_ops(3).iter().enumerate() {
                            if j > 0 {
                                writeln!(diff_summary, "{:-^1$}", "-", 80)?;
                            }
                            for op in group {
                                for change in diff.iter_inline_changes(op) {
                                    let (sign, s) = match change.tag() {
                                        ChangeTag::Delete => ("-", Style::new().red()),
                                        ChangeTag::Insert => ("+", Style::new().green()),
                                        ChangeTag::Equal => (" ", Style::new().dim()),
                                    };
                                    write!(
                                        diff_summary,
                                        "{}{} |{}",
                                        style(Line(change.old_index())).dim(),
                                        style(Line(change.new_index())).dim(),
                                        s.apply_to(sign).bold(),
                                    )?;
                                    for (emphasized, value) in change.iter_strings_lossy() {
                                        if emphasized {
                                            write!(diff_summary, "{}", s.apply_to(value).underlined().on_black())?;
                                        } else {
                                            write!(diff_summary, "{}", s.apply_to(value))?;
                                        }
                                    }
                                    if change.missing_newline() {
                                        writeln!(diff_summary)?;
                                    }
                                }
                            }
                        }

                        return Ok(Some(diff_summary))
                    }
                } else if let Input::Path(path) = input {
                    fs::write(path, output)?;
                }

                Ok(None)
            })
            .collect::<eyre::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<String>>();

        if !diffs.is_empty() {
            // This branch is only reachable with stdin or --check

            if !self.raw {
                for (i, diff) in diffs.iter().enumerate() {
                    if i > 0 {
                        println!();
                    }
                    print!("{diff}");
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
    Path(PathBuf),
    Stdin(String),
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Input::Path(path) => write!(f, "{}", path.display()),
            Input::Stdin(_) => write!(f, "stdin"),
        }
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            None => write!(f, "    "),
            Some(idx) => write!(f, "{:<4}", idx + 1),
        }
    }
}
