use clap::{ArgAction, Parser, Subcommand, ValueHint};
use eyre::Result;
use foundry_compilers::Graph;
use foundry_config::Config;
use semver::Version;
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
    /// - 2: Print source paths.
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

        let mut output: BTreeMap<String, Vec<(Version, Vec<String>)>> = BTreeMap::new();

        for (language, sources) in sources {
            let mut versions_with_paths: Vec<(Version, Vec<String>)> = sources
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

                    (version.clone(), paths)
                })
                .filter(|(_, paths)| !paths.is_empty())
                .collect();

            // Sort by SemVer version.
            versions_with_paths.sort_by(|(v1, _), (v2, _)| Version::cmp(v1, v2));

            // Skip language if no paths are found after filtering.
            if !versions_with_paths.is_empty() {
                output.insert(language.to_string(), versions_with_paths);
            }
        }

        if json {
            println!("{}", serde_json::to_string(&output)?);
            return Ok(());
        }

        for (language, versions) in &output {
            if verbosity < 1 {
                println!("{language}:");
            } else {
                println!("{language}:\n");
            }

            for (version, paths) in versions {
                if verbosity >= 1 {
                    println!("{version}:");
                    for (idx, path) in paths.iter().enumerate() {
                        if idx == paths.len() - 1 {
                            println!("└── {path}\n");
                        } else {
                            println!("├── {path}");
                        }
                    }
                } else {
                    println!("- {version}");
                }
            }

            if verbosity < 1 {
                println!();
            }
        }

        Ok(())
    }
}
