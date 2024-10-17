use clap::{ArgAction, Parser, Subcommand, ValueHint};
use eyre::Result;
use foundry_compilers::{artifacts::EvmVersion, Graph};
use foundry_config::Config;
use semver::Version;
use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf};

/// CLI arguments for `forge compiler`.
#[derive(Debug, Parser)]
pub struct CompilerArgs {
    #[command(subcommand)]
    pub sub: CompilerSubcommands,
}

impl CompilerArgs {
    pub fn run(self) -> Result<()> {
        match self.sub {
            CompilerSubcommands::Resolve(args) => args.run(),
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum CompilerSubcommands {
    /// Retrieves the resolved version(s) of the compiler within the project.
    #[command(visible_alias = "r")]
    Resolve(ResolveArgs),
}

/// Resolved compiler within the project.
#[derive(Serialize)]
struct ResolvedCompiler {
    /// Compiler version.
    version: Version,
    /// Max supported EVM version of compiler.
    #[serde(skip_serializing_if = "Option::is_none")]
    evm_version: Option<EvmVersion>,
    /// Source paths.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    paths: Vec<String>,
}

/// CLI arguments for `forge compiler resolve`.
#[derive(Debug, Parser)]
pub struct ResolveArgs {
    /// The root directory
    #[arg(long, short, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Skip files that match the given regex pattern.
    #[arg(long, short, value_name = "REGEX")]
    skip: Option<regex::Regex>,

    /// Verbosity of the output.
    ///
    /// Pass multiple times to increase the verbosity (e.g. -v, -vv, -vvv).
    ///
    /// Verbosity levels:
    /// - 0: Print compiler versions.
    /// - 1: Print compiler version and source paths.
    /// - 2: Print compiler version, source paths and max supported EVM version of the compiler.
    #[arg(long, short, verbatim_doc_comment, action = ArgAction::Count, help_heading = "Display options")]
    pub verbosity: u8,

    /// Print as JSON.
    #[arg(long, short, help_heading = "Display options")]
    json: bool,
}

impl ResolveArgs {
    pub fn run(self) -> Result<()> {
        let Self { root, skip, verbosity, json } = self;

        let root = root.unwrap_or_else(|| PathBuf::from("."));
        let config = Config::load_with_root(&root);
        let project = config.project()?;

        let graph = Graph::resolve(&project.paths)?;
        let (sources, _) = graph.into_sources_by_version(
            project.offline,
            &project.locked_versions,
            &project.compiler,
        )?;

        let mut output: BTreeMap<String, Vec<ResolvedCompiler>> = BTreeMap::new();

        for (language, sources) in sources {
            let mut versions_with_paths: Vec<ResolvedCompiler> = sources
                .iter()
                .map(|(version, sources)| {
                    let paths: Vec<String> = sources
                        .iter()
                        .filter_map(|(path_file, _)| {
                            let path_str = path_file
                                .strip_prefix(&project.paths.root)
                                .unwrap_or(path_file)
                                .to_path_buf()
                                .display()
                                .to_string();

                            // Skip files that match the given regex pattern.
                            if let Some(ref regex) = skip {
                                if regex.is_match(&path_str) {
                                    return None;
                                }
                            }

                            Some(path_str)
                        })
                        .collect();

                    let evm_version = if verbosity > 1 {
                        Some(
                            EvmVersion::default()
                                .normalize_version_solc(version)
                                .unwrap_or_default(),
                        )
                    } else {
                        None
                    };

                    ResolvedCompiler { version: version.clone(), evm_version, paths }
                })
                .filter(|version| !version.paths.is_empty())
                .collect();

            // Sort by SemVer version.
            versions_with_paths.sort_by(|v1, v2| Version::cmp(&v1.version, &v2.version));

            // Skip language if no paths are found after filtering.
            if !versions_with_paths.is_empty() {
                // Clear paths if verbosity is 0, performed only after filtering to avoid being
                // skipped.
                if verbosity == 0 {
                    versions_with_paths.iter_mut().for_each(|version| version.paths.clear());
                }

                output.insert(language.to_string(), versions_with_paths);
            }
        }

        if json {
            println!("{}", serde_json::to_string(&output)?);
            return Ok(());
        }

        for (language, compilers) in &output {
            match verbosity {
                0 => println!("{language}:"),
                _ => println!("{language}:\n"),
            }

            for resolved_compiler in compilers {
                let version = &resolved_compiler.version;
                match verbosity {
                    0 => println!("- {version}"),
                    _ => {
                        if let Some(evm) = &resolved_compiler.evm_version {
                            println!("{version} (<= {evm}):")
                        } else {
                            println!("{version}:")
                        }
                    }
                }

                if verbosity > 0 {
                    let paths = &resolved_compiler.paths;
                    for (idx, path) in paths.iter().enumerate() {
                        if idx == paths.len() - 1 {
                            println!("└── {path}\n");
                        } else {
                            println!("├── {path}");
                        }
                    }
                }
            }

            if verbosity == 0 {
                println!();
            }
        }

        Ok(())
    }
}
