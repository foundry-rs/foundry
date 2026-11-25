use clap::{Parser, Subcommand, ValueHint};
use eyre::Result;
use foundry_cli::install;
use foundry_common::shell;
use foundry_compilers::{
    Graph, Project,
    artifacts::EvmVersion,
    compilers::{
        multi::{MultiCompiler, MultiCompilerLanguage},
        solc::{Solc, SolcCompiler},
    },
};
use foundry_config::{Config, SolcReq};
use semver::{BuildMetadata, Version};
use serde::Serialize;
use std::{collections::BTreeMap, fmt, path::PathBuf};

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
            CompilerSubcommands::Install(args) => args.run(),
            CompilerSubcommands::List(args) => args.run(),
            CompilerSubcommands::Remove(args) => args.run(),
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum CompilerSubcommands {
    /// Retrieves the resolved version(s) of the compiler within the project.
    #[command(visible_alias = "r")]
    Resolve(ResolveArgs),

    /// Installs solc or Solar releases.
    #[command(visible_alias = "i")]
    Install(InstallArgs),

    /// Lists the installed solc and Solar releases.
    #[command(visible_alias = "ls")]
    List(ListArgs),

    /// Removes installed solc or Solar releases.
    #[command(visible_alias = "rm")]
    Remove(RemoveArgs),
}

/// Resolved compiler within the project.
#[derive(Serialize)]
struct ResolvedCompiler {
    /// Compiler language.
    #[serde(skip)]
    language: MultiCompilerLanguage,
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

    /// Print only the executable path for a single resolved compiler.
    #[arg(long)]
    path: bool,
}

impl ResolveArgs {
    pub fn run(self) -> Result<()> {
        let Self { root, skip, path } = self;

        let root = root.unwrap_or_else(|| PathBuf::from("."));
        let mut config = Config::load_with_root(&root)?;
        install::install_missing_dependencies(&mut config, || Config::load_with_root(&root))?;
        let project = config.project()?;

        let graph = Graph::resolve(&project.paths)?;
        let sources = graph.into_sources_by_version(&project)?.sources;

        let mut output: BTreeMap<String, Vec<ResolvedCompiler>> = BTreeMap::new();

        for (language, sources) in sources {
            let mut versions_with_paths: Vec<ResolvedCompiler> = sources
                .iter()
                .map(|(version, sources, _)| {
                    let paths: Vec<String> = sources
                        .keys()
                        .filter_map(|path_file| {
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

                    let evm_version = (shell::verbosity() > 1).then(|| {
                        EvmVersion::default().normalize_version_solc(version).unwrap_or_default()
                    });

                    ResolvedCompiler { language, version: version.clone(), evm_version, paths }
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
                    for version in &mut versions_with_paths {
                        version.paths.clear();
                    }
                }

                output.insert(language.to_string(), versions_with_paths);
            }
        }

        if path {
            let mut compilers = output.values().flatten();
            let Some(compiler) = compilers.next() else {
                eyre::bail!("no compiler resolved");
            };
            eyre::ensure!(
                compilers.next().is_none(),
                "multiple compilers resolved; use `forge compiler resolve` to inspect them"
            );

            let path = resolved_compiler_path(&project, compiler)?;
            if shell::is_json() {
                sh_println!("{}", serde_json::to_string(&path)?)?;
            } else {
                sh_println!("{}", path.display())?;
            }
            return Ok(());
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

/// CLI arguments for `forge compiler install`.
#[derive(Debug, Parser)]
pub struct InstallArgs {
    /// The compilers to install: a solc version such as `0.8.30`, `solar` for the latest known
    /// Solar release, or `solar:<version>`.
    #[arg(required = true, value_name = "COMPILER", value_parser = parse_compiler)]
    compilers: Vec<Compiler>,
}

impl InstallArgs {
    pub fn run(self) -> Result<()> {
        let mut installed = Vec::with_capacity(self.compilers.len());
        for compiler in self.compilers {
            // The compilation reporter prints download progress to stdout, which carries only
            // the installed paths.
            let solc = match &compiler {
                Compiler::Solc(version) => match Solc::find_svm_installed_version(version)? {
                    Some(solc) => solc,
                    None => {
                        sh_status!("Installing {compiler}")?;
                        Solc::blocking_install(version)?
                    }
                },
                Compiler::Solar(version) => match Solc::find_installed_solar(version) {
                    Some(solc) => solc,
                    None => {
                        sh_status!("Installing {compiler}")?;
                        Solc::blocking_install_solar(version)?
                    }
                },
            };
            if !shell::is_json() {
                sh_println!("{compiler}: {}", solc.solc.display())?;
            }
            installed.push(InstalledCompiler::new(&compiler, solc.solc));
        }
        if shell::is_json() {
            sh_println!("{}", serde_json::to_string(&installed)?)?;
        }
        Ok(())
    }
}

/// An installed compiler binary, as `forge compiler install --json` reports it.
#[derive(Serialize)]
struct InstalledCompiler {
    compiler: &'static str,
    version: Version,
    path: PathBuf,
}

impl InstalledCompiler {
    fn new(compiler: &Compiler, path: PathBuf) -> Self {
        let (name, version) = match compiler {
            Compiler::Solc(version) => ("solc", version),
            Compiler::Solar(version) => ("solar", version),
        };
        Self { compiler: name, version: version.clone(), path }
    }
}

/// Installed solc and Solar releases.
#[derive(Serialize)]
struct InstalledCompilers {
    solc: Vec<Version>,
    solar: Vec<Version>,
}

/// CLI arguments for `forge compiler list`.
#[derive(Debug, Parser)]
pub struct ListArgs;

impl ListArgs {
    pub fn run(self) -> Result<()> {
        let installed = InstalledCompilers {
            solc: Solc::installed_versions(),
            solar: Solc::installed_solar_versions(),
        };
        if shell::is_json() {
            sh_println!("{}", serde_json::to_string(&installed)?)?;
            return Ok(());
        }
        for (name, versions) in [("solc", &installed.solc), ("solar", &installed.solar)] {
            if versions.is_empty() {
                continue;
            }
            sh_println!("{name}:")?;
            for version in versions {
                sh_println!("- {version}")?;
            }
            sh_println!()?;
        }
        Ok(())
    }
}

/// CLI arguments for `forge compiler remove`.
#[derive(Debug, Parser)]
pub struct RemoveArgs {
    /// The compilers to remove: a solc version such as `0.8.30`, `solar` for the latest known
    /// Solar release, or `solar:<version>`.
    #[arg(required = true, value_name = "COMPILER", value_parser = parse_compiler)]
    compilers: Vec<Compiler>,
}

impl RemoveArgs {
    pub fn run(self) -> Result<()> {
        for compiler in self.compilers {
            let removed = match &compiler {
                Compiler::Solc(version) => remove_solc(version)?,
                Compiler::Solar(version) => Solc::remove_solar(version)?,
            };
            eyre::ensure!(removed, "{compiler} is not installed");
            sh_status!("Removed {compiler}")?;
        }
        Ok(())
    }
}

/// Removes an svm-installed solc release and returns whether it was installed.
///
/// Like `svm remove`, a removed global version is replaced by the newest remaining release, or
/// cleared if none remains, so the svm `solc` shim keeps working.
fn remove_solc(version: &Version) -> Result<bool> {
    let mut installed = Solc::installed_versions();
    let Some(index) = installed.iter().position(|v| v == version) else { return Ok(false) };
    svm::remove_version(version)?;
    installed.remove(index);
    if svm::get_global_version().ok().flatten().as_ref() == Some(version) {
        match installed.iter().max() {
            Some(newest) => svm::set_global_version(newest)?,
            None => svm::unset_global_version()?,
        }
    }
    Ok(true)
}

/// A solc or Solar release named on the command line.
#[derive(Clone, Debug)]
enum Compiler {
    Solc(Version),
    Solar(Version),
}

impl fmt::Display for Compiler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Solc(version) => write!(f, "solc {version}"),
            Self::Solar(version) => write!(f, "solar {version}"),
        }
    }
}

fn resolved_compiler_path(
    project: &Project<MultiCompiler>,
    compiler: &ResolvedCompiler,
) -> Result<PathBuf> {
    match compiler.language {
        MultiCompilerLanguage::Solc(_) => {
            let solc = project
                .compiler
                .solc
                .as_ref()
                .ok_or_else(|| eyre::eyre!("Solidity compiler is not available"))?;
            match solc {
                SolcCompiler::AutoDetect => Solc::find_svm_installed_version(&compiler.version)?
                    .map(|solc| solc.solc)
                    .ok_or_else(|| {
                        eyre::eyre!("Solidity compiler {} is not installed", compiler.version)
                    }),
                SolcCompiler::Specific(solc) => Ok(solc.solc.clone()),
            }
        }
        MultiCompilerLanguage::Vyper(_) => project
            .compiler
            .vyper
            .as_ref()
            .map(|vyper| vyper.path.clone())
            .ok_or_else(|| eyre::eyre!("Vyper compiler is not available")),
    }
}

/// Parses a solc version, `solar`, or a `solar:<version>` pin, with the same rules as the `solc`
/// config key.
fn parse_compiler(s: &str) -> Result<Compiler, String> {
    // Like the build path, stable releases are named without build metadata, so
    // `0.8.30+commit.73712a01` installs and removes `0.8.30`. A prerelease keeps its full version,
    // which is the name a `solc` pin installs it under.
    let release = |version: Version| {
        if version.pre.is_empty() {
            Version { build: BuildMetadata::EMPTY, ..version }
        } else {
            version
        }
    };
    match SolcReq::from(s) {
        SolcReq::Version(version) => Ok(Compiler::Solc(release(version))),
        SolcReq::Solar(version) => Ok(Compiler::Solar(release(version))),
        SolcReq::Local(_) if s == "solar" => Solc::released_solar_versions()
            .pop()
            .map(Compiler::Solar)
            .ok_or_else(|| "no known Solar release".to_string()),
        SolcReq::Local(_) => Err("expected a solc version such as `0.8.30`, `solar`, or \
                                  `solar:<version>` naming a release 0.2.0 or later"
            .to_string()),
    }
}
