use clap::{Parser, Subcommand, ValueHint};
use eyre::Result;
use foundry_common::shell;
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
}

impl ResolveArgs {
    pub fn run(self) -> Result<()> {
        let Self { root, skip } = self;

        let root = root.unwrap_or_else(|| PathBuf::from("."));
        let config = Config::load_with_root(&root);
        let project = config.project()?;

        let graph = Graph::resolve(&project.paths)?;
        let (sources, _) = graph.into_sources_by_version(&project)?;

        let mut output: BTreeMap<String, Vec<ResolvedCompiler>> = BTreeMap::new();

        for (language, sources) in sources {
            let mut versions_with_paths: Vec<ResolvedCompiler> = sources
                .iter()
                .map(|(version, sources, _)| {
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

                    let evm_version = if shell::verbosity() > 1 {
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
                if shell::verbosity() == 0 {
                    versions_with_paths.iter_mut().for_each(|version| version.paths.clear());
                }

                output.insert(language.to_string(), versions_with_paths);
            }
        }

        if shell::is_json() {
            sh_println!("{}", serde_json::to_string(&output)?)?;
            return Ok(());
        }

        for (language, compilers) in &output {
            match shell::verbosity() {
                0 => sh_println!("{language}:")?,
                _ => sh_println!("{language}:\n")?,
            }

            for resolved_compiler in compilers {
                let version = &resolved_compiler.version;
                match shell::verbosity() {
                    0 => sh_println!("- {version}")?,
                    _ => {
                        if let Some(evm) = &resolved_compiler.evm_version {
                            sh_println!("{version} (<= {evm}):")?
                        } else {
                            sh_println!("{version}:")?
                        }
                    }
                }

                if shell::verbosity() > 0 {
                    let paths = &resolved_compiler.paths;
                    for (idx, path) in paths.iter().enumerate() {
                        if idx == paths.len() - 1 {
                            sh_println!("└── {path}\n")?
                        } else {
                            sh_println!("├── {path}")?
                        }
                    }
                }
            }

            if shell::verbosity() == 0 {
                sh_println!()?
            }
        }

        Ok(())
    }
}
