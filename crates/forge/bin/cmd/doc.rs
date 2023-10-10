use clap::{Parser, ValueHint};
use eyre::Result;
use forge_doc::{ContractInheritance, Deployments, DocBuilder, GitSource, Inheritdoc, Server};
use foundry_cli::opts::GH_REPO_PREFIX_REGEX;
use foundry_config::{find_project_root_path, load_config_with_root};
use std::{path::PathBuf, process::Command};

#[derive(Debug, Clone, Parser)]
pub struct DocArgs {
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[clap(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// The doc's output path.
    ///
    /// By default, it is the `docs/` in project root.
    #[clap(
        long,
        short,
        value_hint = ValueHint::DirPath,
        value_name = "PATH",
    )]
    out: Option<PathBuf>,

    /// Build the `mdbook` from generated files.
    #[clap(long, short)]
    build: bool,

    /// Serve the documentation.
    #[clap(long, short)]
    serve: bool,

    /// Hostname for serving documentation.
    #[clap(long, requires = "serve")]
    hostname: Option<String>,

    /// Port for serving documentation.
    #[clap(long, short, requires = "serve")]
    port: Option<usize>,

    /// The relative path to the `hardhat-deploy` or `forge-deploy` artifact directory. Leave blank
    /// for default.
    #[clap(long)]
    deployments: Option<Option<PathBuf>>,
}

impl DocArgs {
    pub fn run(self) -> Result<()> {
        let root = self.root.clone().unwrap_or(find_project_root_path(None)?);
        let config = load_config_with_root(Some(root.clone()));

        let mut doc_config = config.doc.clone();
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

        let commit = foundry_cli::utils::Git::new(&root).commit_hash(false, "HEAD").ok();

        let mut builder = DocBuilder::new(root.clone(), config.project_paths().sources)
            .with_should_build(self.build)
            .with_config(doc_config.clone())
            .with_fmt(config.fmt)
            .with_preprocessor(ContractInheritance::default())
            .with_preprocessor(Inheritdoc::default())
            .with_preprocessor(GitSource {
                root: root.clone(),
                commit,
                repository: doc_config.repository.clone(),
            });

        // If deployment docgen is enabled, add the [Deployments] preprocessor
        if let Some(deployments) = self.deployments {
            builder = builder.with_preprocessor(Deployments { root, deployments });
        }

        builder.build()?;

        if self.serve {
            Server::new(doc_config.out)
                .with_hostname(self.hostname.unwrap_or("localhost".to_owned()))
                .with_port(self.port.unwrap_or(3000))
                .serve()?;
        }

        Ok(())
    }
}
