use crate::cmd::Cmd;
use ethers::solc::ProjectPathsConfig;
use forge_fmt::{Formatter, FormatterConfig, Visitable};
use rayon::prelude::*;
use std::path::PathBuf;

use clap::Parser;

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

        paths.par_iter().map(|path| {
            let source = std::fs::read_to_string(&path)?;
            let mut source_unit = solang_parser::parse(&source, 0)
                .map_err(|diags| eyre::eyre!(
                        "Failed to parse Solidity code for {}. Leave source unchanged.\nDebug info: {:?}",
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
                if source != output {
                    std::process::exit(1);
                }
            } else {
                std::fs::write(path, output)?;
            }

            Ok(())
        }).collect::<eyre::Result<_>>()?;

        Ok(())
    }
}
