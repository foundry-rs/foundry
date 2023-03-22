use crate::{cmd::Cmd, opts::GH_REPO_PREFIX_REGEX};
use clap::{Parser, ValueHint};
use forge_doc::{ContractInheritance, DocBuilder, GitSource, Inheritdoc, Server};
use foundry_config::{find_project_root_path, load_config_with_root};
use std::{path::PathBuf, process::Command};

#[derive(Debug, Clone, Parser)]
pub struct DocArgs {
    #[clap(
        help = "The project's root path.",
        long_help = "The project's root path. By default, this is the root directory of the current Git repository, or the current working directory.",
        long,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    root: Option<PathBuf>,

    #[clap(
        help = "The doc's output path.",
        long_help = "The output path for the generated mdbook. By default, it is the `docs/` in project root.",
        long = "out",
        short,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    out: Option<PathBuf>,

    #[clap(help = "Build the `mdbook` from generated files.", long, short)]
    build: bool,

    #[clap(help = "Serve the documentation.", long, short)]
    serve: bool,

    #[clap(help = "Hostname for serving documentation.", long, requires = "serve")]
    hostname: Option<String>,

    #[clap(help = "Port for serving documentation.", long, short, requires = "serve")]
    port: Option<usize>,
}

impl Cmd for DocArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let root = self.root.clone().unwrap_or(find_project_root_path()?);
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

        let commit =
            Command::new("git").args(["rev-parse", "HEAD"]).output().ok().and_then(|output| {
                if !output.stdout.is_empty() {
                    String::from_utf8(output.stdout).ok().map(|commit| commit.trim().to_owned())
                } else {
                    None
                }
            });

        DocBuilder::new(root.clone(), config.project_paths().sources)
            .with_should_build(self.build)
            .with_config(doc_config.clone())
            .with_fmt(config.fmt)
            .with_preprocessor(ContractInheritance::default())
            .with_preprocessor(Inheritdoc::default())
            .with_preprocessor(GitSource {
                root,
                commit,
                repository: doc_config.repository.clone(),
            })
            .build()?;

        if self.serve {
            Server::new(doc_config.out)
                .with_hostname(self.hostname.unwrap_or("localhost".to_owned()))
                .with_port(self.port.unwrap_or(3000))
                .serve()?;
        }

        Ok(())
    }
}
