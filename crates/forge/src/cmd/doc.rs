//! `forge doc`

use crate::cmd::watch::WatchArgs;
use clap::{Parser, ValueHint};
use eyre::{Result, WrapErr};
use forge_doc::DocBuilder;
use foundry_cli::{opts::GH_REPO_PREFIX_REGEX, utils::Git};
use foundry_common::compile::ProjectCompiler;
use foundry_config::{Config, load_config_with_root};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

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
    #[arg(long, short, value_hint = ValueHint::DirPath, value_name = "PATH")]
    out: Option<PathBuf>,

    /// Build the vocs site from the generated MDX files.
    #[arg(long, short)]
    build: bool,

    /// Serve the documentation after generating it (runs `npm run dev`).
    #[arg(long, short)]
    serve: bool,

    /// Document external library sources as well as the project's own sources.
    #[arg(long, short)]
    include_libraries: bool,

    /// Path to the `hardhat-deploy` or `forge-deploy` artifact directory.
    ///
    /// Leave blank to use the default (`<root>/deployments`).
    /// Omit the flag entirely to disable deployment address injection.
    #[arg(long, value_name = "PATH", num_args(0..=1))]
    deployments: Option<Option<PathBuf>>,

    // ── Serve options ─────────────────────────────────────────────────────────
    /// Open the documentation in a browser after serving.
    #[arg(long, requires = "serve", help_heading = "Serve options")]
    open: bool,

    /// Hostname to bind the documentation server to.
    #[arg(long, requires = "serve", value_name = "HOSTNAME", help_heading = "Serve options")]
    hostname: Option<String>,

    /// Port to bind the documentation server to.
    #[arg(long, short, requires = "serve", value_name = "PORT", help_heading = "Serve options")]
    port: Option<usize>,

    // ── Watch options (via WatchArgs) ─────────────────────────────────────────
    #[command(flatten)]
    pub watch: WatchArgs,
}

impl DocArgs {
    pub fn run(self) -> Result<()> {
        let config = self.config()?;
        let root = &config.root;
        let project = config.project()?;

        let mut output = ProjectCompiler::new().quiet(true).compile(&project)?;
        let compiler = output.parser_mut().solc_mut().compiler_mut();

        let mut doc_cfg = config.doc;
        if let Some(out) = self.out.clone() {
            doc_cfg.out = out;
        }

        // Auto-detect repository URL from git remote.
        if doc_cfg.repository.is_none()
            && let Some(remote) = Git::new(root).remote_url("origin")
            && let Some(captures) = GH_REPO_PREFIX_REGEX.captures(&remote)
        {
            let brand = captures.name("brand").unwrap().as_str();
            let tld = captures.name("tld").unwrap().as_str();
            let project_path = GH_REPO_PREFIX_REGEX.replace(&remote, "");
            doc_cfg.repository =
                Some(format!("https://{brand}.{tld}/{}", project_path.trim_end_matches(".git")));
        }

        let commit = Git::new(root).commit_hash(false, "HEAD").ok();

        let mut builder = DocBuilder::new(
            root.clone(),
            project.paths.sources,
            project.paths.libraries,
            self.include_libraries,
        );
        builder.should_build = self.build;
        builder.commit = commit;
        builder.deployments = self.deployments.clone();
        builder.config = doc_cfg.clone();

        let out_dir = if doc_cfg.out.is_absolute() { doc_cfg.out } else { root.join(&doc_cfg.out) };

        builder.build(compiler)?;

        if self.serve {
            run_vocs_dev(
                &out_dir,
                self.hostname.as_deref().unwrap_or("localhost"),
                self.port.unwrap_or(3000),
                self.open,
            )?;
        }

        Ok(())
    }

    /// Returns whether watch mode is enabled.
    pub const fn is_watch(&self) -> bool {
        self.watch.watch.is_some()
    }

    pub fn config(&self) -> Result<Config> {
        load_config_with_root(self.root.as_deref())
    }
}

fn run_vocs_dev(out_dir: &std::path::Path, hostname: &str, port: usize, open: bool) -> Result<()> {
    sh_println!("Serving on: http://{hostname}:{port}")?;
    let mut cmd = Command::new("npm");
    cmd.args(["run", "dev", "--", "--host"])
        .arg(hostname)
        .arg("--port")
        .arg(port.to_string())
        .current_dir(out_dir)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if open {
        cmd.arg("--open");
    }
    let status = cmd
        .status()
        .wrap_err("failed to spawn `npm run dev`; ensure Node.js / npm is installed and on PATH")?;
    if !status.success() {
        eyre::bail!("`npm run dev` exited with status {status}");
    }
    Ok(())
}
