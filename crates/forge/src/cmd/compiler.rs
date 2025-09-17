use clap::{Parser, Subcommand, ValueHint};
use eyre::Result;
use foundry_common::shell;
use foundry_compilers::{
    Compiler, CompilerInput, Graph, artifacts::EvmVersion, multi::MultiCompilerInput,
};
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

/// Dependency info struct, exists only because tuple gets serialized as an array.
#[derive(Serialize)]
struct Dependency {
    name: String,
    version: Version,
}

/// Resolved compiler within the project.
#[derive(Serialize)]
struct ResolvedCompiler {
    /// Compiler name
    name: String,
    /// Compiler version.
    version: Version,
    /// Max supported EVM version of compiler.
    #[serde(skip_serializing_if = "Option::is_none")]
    evm_version: Option<EvmVersion>,
    /// Source paths.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// dependency of the compiler
    dependency: Option<Dependency>,
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

    /// Use resolc.
    #[arg(
        value_name = "RESOLC_COMPILE",
        help = "Enable compiling with resolc",
        long = "resolc-compile",
        visible_alias = "resolc",
        action = clap::ArgAction::SetTrue,
        default_value = "false"
    )]
    resolc_compile: bool,
}

impl ResolveArgs {
    pub fn run(self) -> Result<()> {
        let Self { root, skip, resolc_compile } = self;

        let root = root.unwrap_or_else(|| PathBuf::from("."));

        let config = {
            let mut config = Config::load_with_root(&root)?.canonic_at(root);

            if resolc_compile {
                config.resolc.resolc_compile = true;
            }
            config
        };

        let project = config.project()?;

        let graph = Graph::resolve(&project.paths)?;
        let sources = graph.into_sources_by_version(&project)?.sources;
        let mut output: BTreeMap<String, Vec<ResolvedCompiler>> = BTreeMap::new();

        for (language, sources) in sources {
            let mut versions_with_paths: Vec<ResolvedCompiler> = sources
                .iter()
                .map(|(version, sources, (_, settings))| {
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
                            if let Some(ref regex) = skip
                                && regex.is_match(&path_str)
                            {
                                return None;
                            }

                            Some(path_str)
                        })
                        .collect();

                    let evm_version = if shell::verbosity() > 1 {
                        let evm = EvmVersion::default()
                            .normalize_version_solc(version)
                            .unwrap_or_default();

                        Some(evm)
                    } else {
                        None
                    };
                    let input = MultiCompilerInput::build(
                        sources.clone(),
                        settings.to_owned().clone(),
                        language,
                        version.clone(),
                    );
                    let compiler_version = project.compiler.compiler_version(&input);
                    let mut compiler_name = project.compiler.compiler_name(&input).into_owned();

                    let dependency = {
                        // `Input.version` will always differ from `compiler_version`
                        if config.resolc.resolc_compile {
                            let names = compiler_name;
                            let mut names = names.split_whitespace();
                            compiler_name =
                                names.next().expect("Malformed compiler name").to_owned();
                            names
                                .last()
                                .map(|item| item.to_owned())
                                .zip(Some(version.clone()))
                                .map(|(name, version)| Dependency { name, version })
                        } else {
                            None
                        }
                    };

                    ResolvedCompiler {
                        name: compiler_name,
                        version: compiler_version,
                        evm_version,
                        paths,
                        dependency,
                    }
                })
                .filter(|version| !version.paths.is_empty())
                .collect();

            // Sort by SemVer version.
            versions_with_paths.sort_by(|v1, v2| {
                (&v1.version, &v1.dependency.as_ref().map(|x| &x.version))
                    .cmp(&(&v2.version, &v2.dependency.as_ref().map(|x| &x.version)))
            });

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
                let extras =
                    if let Some(Dependency { name, version }) = &resolved_compiler.dependency {
                        format!(", {name} v{version}")
                    } else {
                        String::new()
                    };
                match shell::verbosity() {
                    0 => sh_println!("- {} v{version}{}", resolved_compiler.name, extras)?,
                    _ => {
                        if let Some(evm) = &resolved_compiler.evm_version {
                            sh_println!(
                                "{} v{version}{} (<= {evm}):",
                                resolved_compiler.name,
                                extras
                            )?
                        } else {
                            sh_println!("{} v{version}{}:", resolved_compiler.name, extras)?
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
