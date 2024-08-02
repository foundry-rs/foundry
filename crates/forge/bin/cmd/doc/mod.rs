use clap::{Parser, ValueHint};
use eyre::Result;
use forge_doc::{
    ContractInheritance, Deployments, DocBuilder, GitSource, InferInlineHyperlinks, Inheritdoc,
};
use foundry_cli::opts::GH_REPO_PREFIX_REGEX;
use foundry_common::compile::ProjectCompiler;
use foundry_config::{find_project_root_path, load_config_with_root};
use notify::{EventKind, RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{channel, Receiver},
    time::Duration,
};

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

    /// Watch on files and recompile on change.
    #[arg(long, short)]
    watch: bool,

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
    pub fn run(self) -> Result<()> {
        self.generate_doc()?;

        if self.watch {
            let mut debouncer =
                new_debouncer(Duration::from_secs(2), None, move |result: DebounceEventResult| {
                    match result {
                        Ok(events) => events.iter().for_each(|event| match event.event.kind {
                            EventKind::Create(_) => {
                                self.generate_doc().unwrap();
                            }
                            EventKind::Modify(_) => {
                                self.generate_doc().unwrap();
                            }
                            EventKind::Remove(_) => {
                                self.generate_doc().unwrap();
                            }
                            _ => {}
                        }),
                        Err(errors) => errors.iter().for_each(|_error| {}),
                    }
                })
                .unwrap();

            let (_tx, rx): (
                std::sync::mpsc::Sender<notify::Result<notify::Event>>,
                Receiver<notify::Result<notify::Event>>,
            ) = channel();

            // Watch src files sice if watch the cwd infinite loops happen since the generate doc
            // edits the ./doc files
            debouncer.watcher().watch(Path::new("./src"), RecursiveMode::Recursive).unwrap();

            println!("Started watching...");

            loop {
                // Timeout to avoid blocking indefinitely
                match rx.recv_timeout(Duration::from_secs(1800)) {
                    Ok(_event_result) => {}
                    Err(err) => match err {
                        std::sync::mpsc::RecvTimeoutError::Timeout => {
                            break;
                        }
                        std::sync::mpsc::RecvTimeoutError::Disconnected => {
                            // The channel has been disconnected
                            break;
                        }
                    },
                }
            }
        }

        Ok(())
    }

    fn generate_doc(&self) -> Result<()> {
        let root = self.root.clone().unwrap_or(find_project_root_path(None)?);
        let config = load_config_with_root(Some(root.clone()));
        let project = config.project()?;
        let compiler = ProjectCompiler::new().quiet(true);
        let _output = compiler.compile(&project)?;

        let mut doc_config = config.doc.clone();
        if let Some(out) = self.out.clone() {
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

        let mut builder = DocBuilder::new(
            root.clone(),
            project.paths.sources.clone(),
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
        if let Some(deployments) = self.deployments.clone() {
            builder = builder.with_preprocessor(Deployments { root, deployments });
        }

        builder.build()?;

        if self.serve {
            Server::new(doc_config.out)
                .with_hostname(self.hostname.clone().unwrap_or("localhost".to_owned()))
                .with_port(self.port.unwrap_or(3000))
                .open(self.open)
                .serve()?;
        }

        Ok(())
    }
}
