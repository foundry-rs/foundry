//! build command

use ethers::solc::{
    artifacts::{Optimizer, Settings},
    MinimalCombinedArtifacts, Project, ProjectCompileOutput, ProjectPathsConfig, SolcConfig,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{cmd::Cmd, opts::forge::CompilerArgs, utils};

use clap::{Parser, ValueHint};

#[derive(Debug, Clone, Parser)]
pub struct BuildArgs {
    #[clap(
        help = "the project's root path. By default, this is the root directory of the current Git repository or the current working directory if it is not part of a Git repository",
        long,
        value_hint = ValueHint::DirPath
    )]
    pub root: Option<PathBuf>,

    #[clap(
        env = "DAPP_SRC",
        help = "the directory relative to the root under which the smart contracts are",
        long,
        short,
        value_hint = ValueHint::DirPath
    )]
    pub contracts: Option<PathBuf>,

    #[clap(help = "the remappings", long, short)]
    pub remappings: Vec<ethers::solc::remappings::Remapping>,
    #[clap(long = "remappings-env", env = "DAPP_REMAPPINGS")]
    pub remappings_env: Option<String>,

    #[clap(
        help = "the paths where your libraries are installed",
        long,
        value_hint = ValueHint::DirPath
    )]
    pub lib_paths: Vec<PathBuf>,

    #[clap(
        help = "path to where the contract artifacts are stored",
        long = "out",
        short,
        value_hint = ValueHint::DirPath
    )]
    pub out_path: Option<PathBuf>,

    #[clap(flatten)]
    pub compiler: CompilerArgs,

    #[clap(help = "ignore warnings with specific error codes", long)]
    pub ignored_error_codes: Vec<u64>,

    #[clap(
        help = "if set to true, skips auto-detecting solc and uses what is in the user's $PATH ",
        long
    )]
    pub no_auto_detect: bool,

    #[clap(
        help = "force recompilation of the project, deletes the cache and artifacts folders",
        long
    )]
    pub force: bool,

    #[clap(
        help = "uses hardhat style project layout. This a convenience flag and is the same as `--contracts contracts --lib-paths node_modules`",
        long,
        conflicts_with = "contracts",
        alias = "hh"
    )]
    pub hardhat: bool,

    #[clap(help = "add linked libraries", long, env = "DAPP_LIBRARIES")]
    pub libraries: Vec<String>,
}

impl Cmd for BuildArgs {
    type Output = ProjectCompileOutput<MinimalCombinedArtifacts>;
    fn run(self) -> eyre::Result<Self::Output> {
        let project = self.project()?;
        super::compile(&project)
    }
}

impl BuildArgs {
    /// Determines the source directory within the given root
    fn contracts_path(&self, root: impl AsRef<Path>) -> PathBuf {
        let root = root.as_ref();
        if let Some(ref contracts) = self.contracts {
            root.join(contracts)
        } else if self.hardhat {
            root.join("contracts")
        } else {
            // no contract source directory was provided, determine the source directory
            ProjectPathsConfig::find_source_dir(&root)
        }
    }

    /// Determines the artifacts directory within the given root
    fn artifacts_path(&self, root: impl AsRef<Path>) -> PathBuf {
        let root = root.as_ref();
        if let Some(ref artifacts) = self.out_path {
            root.join(artifacts)
        } else if self.hardhat {
            root.join("artifacts")
        } else {
            // no artifacts source directory was provided, determine the artifacts directory
            ProjectPathsConfig::find_artifacts_dir(&root)
        }
    }

    /// Converts all build arguments to the corresponding project config
    ///
    /// Defaults to DAppTools-style repo layout, but can be customized.
    pub fn project(&self) -> eyre::Result<Project> {
        // 1. Set the root dir
        let root = self.root.clone().unwrap_or_else(|| {
            utils::find_git_root_path().unwrap_or_else(|_| std::env::current_dir().unwrap())
        });
        let root = dunce::canonicalize(&root)?;

        // 2. Set the contracts dir
        let contracts = self.contracts_path(&root);

        // 3. Set the output dir
        let artifacts = self.artifacts_path(&root);

        // 4. Set where the libraries are going to be read from
        // default to the lib path being the `lib/` dir
        let lib_paths = utils::find_libs(&root, &self.lib_paths, self.hardhat);

        let remappings = utils::find_remappings(
            &lib_paths,
            &self.remappings,
            &root.join("remappings.txt"),
            &self.remappings_env,
        );

        // build the path
        let mut paths_builder =
            ProjectPathsConfig::builder().root(&root).sources(contracts).artifacts(artifacts);

        if !remappings.is_empty() {
            paths_builder = paths_builder.remappings(remappings);
        }

        let paths = paths_builder.build()?;

        let optimizer = Optimizer {
            enabled: Some(self.compiler.optimize),
            runs: Some(self.compiler.optimize_runs as usize),
        };

        // unflatten the libraries
        let mut libraries = BTreeMap::default();
        for l in self.libraries.iter() {
            let mut items = l.split(':');
            let file = String::from(items.next().expect("could not parse libraries"));
            let lib = String::from(items.next().expect("could not parse libraries"));
            let addr = String::from(items.next().expect("could not parse libraries"));
            libraries.entry(file).or_insert_with(BTreeMap::default).insert(lib, addr);
        }

        // build the project w/ allowed paths = root and all the libs
        let solc_settings = Settings {
            optimizer,
            evm_version: Some(self.compiler.evm_version),
            libraries,
            ..Default::default()
        };
        let mut builder = Project::builder()
            .paths(paths)
            .allowed_path(&root)
            .allowed_paths(lib_paths)
            .solc_config(SolcConfig::builder().settings(solc_settings).build()?);

        if self.no_auto_detect {
            builder = builder.no_auto_detect();
        }

        for error_code in &self.ignored_error_codes {
            builder = builder.ignore_error_code(*error_code);
        }

        let project = builder.build()?;

        // if `--force` is provided, it proceeds to remove the cache
        // and recompile the contracts.
        if self.force {
            project.cleanup()?;
        }

        Ok(project)
    }
}
