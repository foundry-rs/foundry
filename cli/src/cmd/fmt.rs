use std::{fmt::Write, path::PathBuf};

use clap::Parser;
use console::{style, Style};
use ethers::solc::ProjectPathsConfig;
use rayon::prelude::*;
use similar::{ChangeTag, TextDiff};

use forge_fmt::{Formatter, FormatterConfig, Visitable};

use crate::cmd::Cmd;

#[derive(Debug, Clone, Parser)]
pub struct FmtArgs {
    #[clap(help = "path to the file or directory", conflicts_with = "root")]
    path: Option<PathBuf>,
    #[clap(help = "project's root path, default being the current working directory", long)]
    root: Option<PathBuf>,
    #[clap(
        help = "run in 'check' mode. Exits with 0 if input is formatted correctly. Exits with 1 if formatting is required.",
        long
    )]
    check: bool,
}

struct Line(Option<usize>);

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.0 {
            None => write!(f, "    "),
            Some(idx) => write!(f, "{:<4}", idx + 1),
        }
    }
}

impl Cmd for FmtArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let root = if let Some(path) = self.path {
            path
        } else {
            let root = self.root.unwrap_or_else(|| {
                std::env::current_dir().expect("failed to get current directory")
            });
            if !root.is_dir() {
                return Err(eyre::eyre!("Root path should be a directory"))
            }

            ProjectPathsConfig::find_source_dir(&root)
        };

        let paths = if root.is_dir() {
            ethers::solc::utils::source_files(root)
        } else if root.file_name().unwrap().to_string_lossy().ends_with(".sol") {
            vec![root]
        } else {
            vec![]
        };

        let diffs = paths
            .par_iter()
            .enumerate()
            .map(|(i, path)| {
                let source = std::fs::read_to_string(&path)?;
                let (mut source_unit, _comments) = solang_parser::parse(&source, i)
                    .map_err(|diags| eyre::eyre!(
                            "Failed to parse Solidity code for {}. Leaving source unchanged.\nDebug info: {:?}",
                            path.to_string_lossy(),
                            diags
                        ))?;

                let mut output = String::new();
                let mut formatter =
                    Formatter::new(&mut output, &source, FormatterConfig::default());

                source_unit.visit(&mut formatter).unwrap();

                solang_parser::parse(&output, 0).map_err(|diags| {
                    eyre::eyre!(
                            "Failed to construct valid Solidity code for {}. Leaving source unchanged.\nDebug info: {:?}",
                            path.to_string_lossy(),
                            diags
                        )
                })?;

                if self.check {
                    let diff = TextDiff::from_lines(&source, &output);

                    if diff.ratio() < 1.0 {
                        let mut diff_summary = String::new();

                        writeln!(diff_summary, "Diff in {}:", path.to_string_lossy())?;
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
                } else {
                    std::fs::write(path, output)?;
                }

                Ok(None)
            })
            .collect::<eyre::Result<Vec<Option<String>>>>()?;

        if !diffs.is_empty() {
            for (i, diff) in diffs.iter().flatten().enumerate() {
                if i > 0 {
                    println!();
                }
                print!("{}", diff);
            }

            std::process::exit(1);
        }

        Ok(())
    }
}
