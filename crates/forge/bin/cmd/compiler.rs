use clap::{ArgAction, Parser, Subcommand, ValueHint};
use eyre::Result;
use foundry_compilers::Graph;
use foundry_config::Config;
use std::{collections::BTreeMap, path::PathBuf};

/// CLI arguments for `forge compiler`.
#[derive(Debug, Parser)]
pub struct CompilerArgs {
    #[command(subcommand)]
    pub sub: CompilerSubcommands,
}

impl CompilerArgs {
    pub async fn run(self) -> Result<()> {
        match self.sub {
            CompilerSubcommands::Resolve(args) => args.run().await,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum CompilerSubcommands {
    /// Retrieves the resolved version(s) of the compiler within the project.
    Resolve(ResolveArgs),
}

/// CLI arguments for `forge compiler resolve`.
#[derive(Debug, Parser)]
pub struct ResolveArgs {
    /// The root directory
    #[arg(value_hint = ValueHint::DirPath, default_value = ".", value_name = "PATH")]
    root: PathBuf,

    /// Verbosity of the output.
    ///
    /// Pass multiple times to increase the verbosity (e.g. -v, -vv, -vvv).
    ///
    /// Verbosity levels:
    /// - 2: Print sources
    #[arg(long, short, verbatim_doc_comment, action = ArgAction::Count, help_heading = "Display options")]
    pub verbosity: u8,

    /// Print as JSON.
    #[arg(long, short, help_heading = "Display options")]
    json: bool,
}

impl ResolveArgs {
    pub async fn run(self) -> Result<()> {
        let Self { root, verbosity, json } = self;
        let config = Config::load_with_root(root);
        let project = config.project()?;

        let graph = Graph::resolve(&project.paths)?;
        let (sources, _) = graph.into_sources_by_version(
            project.offline,
            &project.locked_versions,
            &project.compiler,
        )?;

        if json {
            let mut output = BTreeMap::new();

            for (language, sources) in sources {
                let versions: Vec<BTreeMap<String, Vec<String>>> = sources
                    .iter()
                    .map(|(version, sources)| {
                        let paths: Vec<String> = sources
                            .iter()
                            .map(|(path_file, _)| {
                                path_file
                                    .strip_prefix(&project.paths.root)
                                    .unwrap_or(path_file)
                                    .to_path_buf()
                                    .display()
                                    .to_string()
                            })
                            .collect();

                        let mut version_map = BTreeMap::new();
                        version_map.insert(version.to_string(), paths);
                        version_map
                    })
                    .collect();

                // Insert language and its versions into the output map
                output.insert(language.to_string(), versions);
            }

            println!("{}", serde_json::to_string(&output)?);
        } else {
            for (language, sources) in sources {
                println!("{language}:\n");

                for (version, sources) in sources {
                    if verbosity >= 1 {
                        println!("{version}:");
                        for (idx, (path_file, _)) in sources.iter().enumerate() {
                            let path = path_file
                                .strip_prefix(&project.paths.root)
                                .unwrap_or(path_file)
                                .to_path_buf();

                            if idx == sources.len() - 1 {
                                println!("└── {}\n", path.display());
                            } else {
                                println!("├── {}", path.display());
                            }
                        }
                    } else {
                        println!("- {language} {version}");
                    }
                }
            }
        }

        Ok(())
    }
}
