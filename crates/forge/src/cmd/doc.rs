//! `forge doc`

use crate::cmd::{install, watch::WatchArgs};
use clap::{Parser, ValueHint};
use eyre::Result;
use forge_doc::DocBuilder;
use foundry_cli::{opts::GH_REPO_PREFIX_REGEX, utils::Git};
use foundry_common::{compile::ProjectCompiler, shell};
use foundry_config::{Config, load_config_with_root};
use std::{path::PathBuf, time::Instant};

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
    /// By default, it is the `docs/` directory in the project root.
    /// The directory is created if it does not exist, and contains a
    /// ready-to-use [vocs](https://vocs.dev) site scaffold alongside the
    /// generated MDX pages.
    #[arg(long, short, value_hint = ValueHint::DirPath, value_name = "PATH")]
    out: Option<PathBuf>,

    /// Document external library sources as well as the project's own sources.
    #[arg(long, short)]
    pub include_libraries: bool,

    /// Path to the `hardhat-deploy` or `forge-deploy` artifact directory.
    ///
    /// Leave blank to use the default (`<root>/deployments`).
    /// Omit the flag entirely to disable deployment address injection.
    #[arg(long, value_name = "PATH", num_args(0..=1))]
    pub deployments: Option<Option<PathBuf>>,

    #[command(flatten)]
    pub watch: WatchArgs,

    /// Deprecated flag after the migration to Vocs. Previously, it was used to serve docs locally.
    #[arg(long, hide = true)]
    serve: bool,
}

impl DocArgs {
    pub async fn run(self) -> Result<()> {
        if self.serve {
            eyre::bail!(
                "`--serve` has been removed. Generate the docs with `forge doc`, \
                 then run `npm run dev` from the generated docs directory."
            );
        }
        let mut config = self.config()?;

        if install::install_missing_dependencies(&mut config).await && config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.config()?;
        }

        let root = &config.root;
        let project = config.project()?;

        // Compile the project first so the doc renderer has access to a fully
        // resolved HIR (needed for `@inheritdoc`, cross-references, etc.).
        // We let the standard compilation reporter print progress so users get
        // the same visual feedback as `forge build`; pass `--quiet` to silence.
        let mut output = ProjectCompiler::new().compile(&project)?;
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

        let git = Git::new(root);
        let commit = git.commit_hash(false, "HEAD").ok();
        // Best-effort branch detection for editLink. May yield "HEAD" when in
        // detached HEAD state; treat that as unknown.
        let branch = git.current_rev_branch(root).ok().map(|(_, b)| b).filter(|b| b != "HEAD");

        let mut builder = DocBuilder::new(
            root.clone(),
            project.paths.sources,
            project.paths.libraries,
            self.include_libraries,
        );
        builder.commit = commit;
        builder.branch = branch;
        builder.deployments = self.deployments;
        builder.config = doc_cfg.clone();

        let out_dir = if doc_cfg.out.is_absolute() { doc_cfg.out } else { root.join(&doc_cfg.out) };

        if !shell::is_quiet() {
            sh_println!("Generating documentation...")?;
        }
        let started = Instant::now();
        let stats = builder.build(compiler)?;
        let elapsed = started.elapsed();

        if !shell::is_quiet() {
            sh_println!(
                "Generated {pages} page{ps} from {sources} source{ss} in {elapsed:.2?} (render {render:.2?}, site {site:.2?})",
                pages = stats.pages,
                ps = if stats.pages <= 1 { "" } else { "s" },
                sources = stats.sources,
                ss = if stats.sources <= 1 { "" } else { "s" },
                elapsed = elapsed,
                render = stats.render_elapsed,
                site = stats.site_elapsed,
            )?;

            // TODO: `--legacy-peer-deps` flag is required waku dependency conflict resolution.
            // Remove this flag once vocs v2 and waku v1 are released.
            sh_println!(
                "\nDocumentation written to: {}\n\nTo preview:\n  cd {}\n  npm install --legacy-peer-deps\n  npm run dev",
                out_dir.display(),
                out_dir.display(),
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
