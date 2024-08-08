use super::watch::WatchArgs;
use clap::{Parser, ValueHint};
use eyre::Result;
use forge_doc::{
    ContractInheritance, Deployments, DocBuilder, GitSource, InferInlineHyperlinks, Inheritdoc,
};
use foundry_cli::opts::GH_REPO_PREFIX_REGEX;
use foundry_common::compile::ProjectCompiler;
use foundry_config::{find_project_root_path, load_config_with_root, Config};
use std::{path::PathBuf, process::Command};

mod server;
use server::Server;

#[derive(Clone, Debug, Parser)]
pub struct DocArgs {
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// The doc's output path.
    ///
    /// By default, it is the `docs/` in project root.
    #[arg(
        long,
        short,
        value_hint = ValueHint::DirPath,
        value_name = "PATH",
    )]
    out: Option<PathBuf>,

    /// Build the `mdbook` from generated files.
    #[arg(long, short)]
    build: bool,

    /// Serve the documentation.
    #[arg(long, short)]
    serve: bool,

    /// Open the documentation in a browser after serving.
    #[arg(long, requires = "serve")]
    open: bool,

    /// Hostname for serving documentation.
    #[arg(long, requires = "serve")]
    hostname: Option<String>,

    #[command(flatten)]
    pub watch: WatchArgs,

    /// Port for serving documentation.
    #[arg(long, short, requires = "serve")]
    port: Option<usize>,

    /// The relative path to the `hardhat-deploy` or `forge-deploy` artifact directory. Leave blank
    /// for default.
    #[arg(long)]
    deployments: Option<Option<PathBuf>>,

    /// Whether to create docs for external libraries.
    #[arg(long, short)]
    include_libraries: bool,
}

impl DocArgs {
    pub async fn run(self) -> Result<()> {
        let config = self.config()?;
        let root = &config.root.0;
        let project = config.project()?;
        let compiler = ProjectCompiler::new().quiet(true);
        let _output = compiler.compile(&project)?;

        let mut doc_config = config.doc;
        if let Some(out) = self.out {
            doc_config.out = out;
        }
        if doc_config.repository.is_none() {
            // Attempt to read repo from git
            if let Ok(output) = Command::new("git").args(["remote", "get-url", "origin"]).output() {
                if !output.stdout.is_empty() {
                    let remote = String::from_utf8(output.stdout)?.trim().to_owned();
                    if let Some(captures) = GH_REPO_PREFIX_REGEX.captures(&remote) {
                        let brand = captures.name("brand").unwrap().as_str();
                        let tld = captures.name("tld").unwrap().as_str();
                        let project = GH_REPO_PREFIX_REGEX.replace(&remote, "");
                        doc_config.repository = Some(format!(
                            "https://{brand}.{tld}/{}",
                            project.trim_end_matches(".git")
                        ));
                    }
                }
            }
        }

        let commit = foundry_cli::utils::Git::new(root).commit_hash(false, "HEAD").ok();

        let mut builder = DocBuilder::new(
            root.clone(),
            project.paths.sources,
            project.paths.libraries,
            self.include_libraries,
        )
        .with_should_build(self.build)
        .with_config(doc_config.clone())
        .with_fmt(config.fmt)
        .with_preprocessor(ContractInheritance { include_libraries: self.include_libraries })
        .with_preprocessor(Inheritdoc::default())
        .with_preprocessor(InferInlineHyperlinks::default())
        .with_preprocessor(GitSource {
            root: root.clone(),
            commit,
            repository: doc_config.repository.clone(),
        });

        // If deployment docgen is enabled, add the [Deployments] preprocessor
        if let Some(deployments) = self.deployments {
            builder = builder.with_preprocessor(Deployments { root: root.clone(), deployments });
        }

        builder.build()?;

        if self.serve {
            Server::new(doc_config.out)
                .with_hostname(self.hostname.unwrap_or_else(|| "localhost".into()))
                .with_port(self.port.unwrap_or(3000))
                .open(self.open)
                .serve()?;
        }

        Ok(())
    }

    /// Returns whether watch mode is enabled
    pub fn is_watch(&self) -> bool {
        self.watch.watch.is_some()
    }

    pub fn config(&self) -> Result<Config> {
        let root = self.root.clone().unwrap_or(find_project_root_path(None)?);
        Ok(load_config_with_root(Some(root)))
    }
}
