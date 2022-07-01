use crate::{cmd::Cmd, utils::FoundryPathExt};
use clap::{Parser, ValueHint};
use console::{style, Style};
use forge_fmt::{Comments, Formatter, FormatterConfig, Visitable};
use foundry_common::fs;
use rayon::prelude::*;
use similar::{ChangeTag, TextDiff};
use std::{
    fmt::{self, Write},
    io,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Parser)]
pub struct FmtArgs {
    #[clap(
        help = "path to the file, directory or '-' to read from stdin",
        conflicts_with = "root",
        value_hint = ValueHint::FilePath,
        value_name = "PATH"
    )]
    path: Option<PathBuf>,
    #[clap(
        help = "The project's root path.",
        long_help = "The project's root path. By default, this is the root directory of the current Git repository, or the current working directory.",
        long,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
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

// === impl FmtArgs ===

impl FmtArgs {
    /// Returns all inputs to format
    fn inputs(&self) -> Vec<Input> {
        if let Some(ref path) = self.path {
            if path == Path::new("-") || !atty::is(atty::Stream::Stdin) {
                let mut buf = String::new();
                io::stdin().read_to_string(&mut buf).expect("Failed to read from stdin");
                vec![Input::Stdin(buf)]
            } else if path.is_dir() {
                ethers::solc::utils::source_files(path).into_iter().map(Input::Path).collect()
            } else if path.is_sol() {
                vec![Input::Path(path.to_path_buf())]
            } else {
                vec![]
            }
        } else {
            let config = foundry_config::load_config_with_root(self.root.clone());
            let paths = config.project_paths();
            paths.input_files().into_iter().map(Input::Path).collect()
        }
    }
}

impl Cmd for FmtArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let inputs = self.inputs();

        let diffs = inputs
            .par_iter()
            .enumerate()
            .map(|(i, input)| {
                let source = match input {
                    Input::Path(path) => fs::read_to_string(&path)?,
                    Input::Stdin(source) => source.to_string()
                };

                let (mut source_unit, comments) = solang_parser::parse(&source, i)
                    .map_err(|diags| eyre::eyre!(
                            "Failed to parse Solidity code for {}. Leaving source unchanged.\nDebug info: {:?}",
                            input,
                            diags
                        ))?;
                let comments = Comments::new(comments, &source);

                let mut output = String::new();
                let mut formatter =
                    Formatter::new(&mut output, &source, comments, FormatterConfig::default());

                source_unit.visit(&mut formatter).unwrap();

                solang_parser::parse(&output, 0).map_err(|diags| {
                    eyre::eyre!(
                            "Failed to construct valid Solidity code for {}. Leaving source unchanged.\nDebug info: {:?}",
                            input,
                            diags
                        )
                })?;

                if self.check || matches!(input, Input::Stdin(_)) {
                    if self.raw {
                        print!("{}", output);
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
