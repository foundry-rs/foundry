use crate::{DepIdentifier, FOUNDRY_LOCK, Lockfile};
use clap::{Parser, ValueHint};
use eyre::{Context, Result};
use foundry_cli::{
    opts::Dependency,
    utils::{Git, LoadConfig},
};
use foundry_common::fs;
use foundry_config::{Config, impl_figment_convert_basic};
use regex::Regex;
use semver::Version;
use soldeer_commands::{Command, Verbosity, commands::install::Install};
use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
    str,
    sync::LazyLock,
};
use yansi::Paint;

static DEPENDENCY_VERSION_TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^v?\d+(\.\d+)*$").unwrap());

/// CLI arguments for `forge install`.
#[derive(Clone, Debug, Parser)]
#[command(override_usage = "forge install [OPTIONS] [DEPENDENCIES]...
    forge install [OPTIONS] <github username>/<github project>@<tag>...
    forge install [OPTIONS] <alias>=<github username>/<github project>@<tag>...
    forge install [OPTIONS] <https://<github token>@git url>...)]
    forge install [OPTIONS] <https:// git url>...")]
pub struct InstallArgs {
    /// The dependencies to install.
    ///
    /// A dependency can be a raw URL, or the path to a GitHub repository.
    ///
    /// Additionally, a ref can be provided by adding @ to the dependency path.
    ///
    /// A ref can be:
    /// - A branch: master
    /// - A tag: v1.2.3
    /// - A commit: 8e8128
    ///
    /// For exact match, a ref can be provided with `@tag=`, `@branch=` or `@rev=` prefix.
    ///
    /// Target installation directory can be added via `<alias>=` suffix.
    /// The dependency will installed to `lib/<alias>`.
    dependencies: Vec<Dependency>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// Do not create a commit after installing.
    ///
    /// This is a noop flag kept for backwards compatibility, as `forge install` no longer commits
    /// by default. Use `--commit` to opt into creating a commit.
    #[arg(long, hide = true)]
    pub no_commit: bool,

    #[command(flatten)]
    opts: DependencyInstallOpts,
}

impl_figment_convert_basic!(InstallArgs);

impl InstallArgs {
    pub async fn run(self) -> Result<()> {
        let mut config = self.load_config()?;
        self.opts.install(&mut config, self.dependencies).await
    }
}

#[derive(Clone, Copy, Debug, Default, Parser)]
pub struct DependencyInstallOpts {
    /// Perform shallow clones instead of deep ones.
    ///
    /// Improves performance and reduces disk usage, but prevents switching branches or tags.
    #[arg(long)]
    pub shallow: bool,

    /// Install without adding the dependency as a submodule.
    #[arg(long)]
    pub no_git: bool,

    /// Create a commit after installing the dependencies.
    #[arg(long)]
    pub commit: bool,
}

impl DependencyInstallOpts {
    pub fn git(self, config: &Config) -> Git<'_> {
        Git::from_config(config).shallow(self.shallow)
    }

    /// Installs all missing dependencies.
    ///
    /// See also [`Self::install`].
    ///
    /// Returns true if any dependency was installed.
    pub async fn install_missing_dependencies(self, config: &mut Config) -> bool {
        let lib = config.install_lib_dir();
        if self.git(config).has_missing_dependencies(Some(lib)).unwrap_or(false) {
            let _ = sh_status!("Missing dependencies found. Installing now...");

            if self.install(config, Vec::new()).await.is_err() {
                let _ =
                    sh_warn!("Your project has missing dependencies that could not be installed.");
            }
            true
        } else {
            false
        }
    }

    /// Installs all dependencies
    pub async fn install(self, config: &mut Config, dependencies: Vec<Dependency>) -> Result<()> {
        let Self { no_git, commit, .. } = self;

        let git = self.git(config);

        let install_lib_dir = config.install_lib_dir();
        let libs = git.root.join(install_lib_dir);

        let mut lockfile = Lockfile::new(&config.root);
        if !no_git {
            lockfile = lockfile.with_git(&git);

            // Initialize all existing submodules when no explicit dependencies were requested so
            // that foundry.lock synchronization can inspect their commits and tags.
            if dependencies.is_empty() && git.submodules_uninitialized()? {
                trace!(lib = %libs.display(), "submodules uninitialized");
                git.submodule_update(false, false, false, true, Some(&libs))?;
            }
        }

        let out_of_sync_deps = lockfile.sync(config.install_lib_dir())?;

        if dependencies.is_empty() && !no_git {
            // Use the root of the git repository to look for submodules.
            let root = Git::root_of(git.root)?;
            match git.has_submodules(Some(&root)) {
                Ok(true) => {
                    sh_status!("Updating dependencies in {}", libs.display())?;

                    // recursively fetch all submodules (without fetching latest)
                    git.submodule_update(false, false, false, true, Some(&libs))?;

                    // checkout submodules at the revs recorded in `foundry.lock`
                    if let Some(out_of_sync) = &out_of_sync_deps {
                        for (rel_path, dep_id) in out_of_sync {
                            git.checkout_at(dep_id.checkout_id(), &git.root.join(rel_path))?;
                        }
                    }

                    lockfile.write()?;
                }
                Err(err) => {
                    sh_err!("Failed to check for submodules: {err}")?;
                }
                _ => {
                    // no submodules, nothing to do
                }
            }
        }

        fs::create_dir_all(&libs)?;

        let installer = Installer { git, commit };
        for dep in dependencies {
            let path = libs.join(dep.name());
            let rel_path = path
                .strip_prefix(git.root)
                .wrap_err("Library directory is not relative to the repository root")?;
            sh_status!(
                "Installing {} in {} (url: {}, tag: {})",
                dep.name,
                path.display(),
                dep.url.as_deref().unwrap_or("None"),
                dep.tag.as_deref().unwrap_or("None")
            )?;

            // this tracks the actual installed tag
            let installed_tag;
            let mut dep_id = None;
            if no_git {
                installed_tag = installer.install_as_folder(&dep, &path)?;
            } else {
                if commit {
                    git.ensure_clean()?;
                }
                let (tag, mut transaction) =
                    installer.install_as_submodule(&dep, &path, &config.root)?;
                installed_tag = tag;

                let mut new_insertion = false;
                // Pin branch to submodule if branch is used
                if let Some(tag_or_branch) = &installed_tag {
                    // First, check if this tag has a branch
                    dep_id = Some(DepIdentifier::resolve_type(&git, &path, tag_or_branch)?);
                    if git.has_branch(tag_or_branch, &path)?
                        && dep_id.as_ref().is_some_and(|id| id.is_branch())
                    {
                        // always work with relative paths when directly modifying submodules
                        git.set_submodule_branch(rel_path, tag_or_branch)?;
                        let root = Git::root_of(git.root)?;
                        git.root(&root).add(Some(".gitmodules"))?;

                        let rev = git.get_rev(tag_or_branch, &path)?;

                        dep_id = Some(DepIdentifier::Branch {
                            name: tag_or_branch.clone(),
                            rev,
                            r#override: false,
                        });
                    }

                    trace!(?dep_id, ?tag_or_branch, "resolved dep id");
                    if let Some(dep_id) = &dep_id {
                        new_insertion = true;
                        lockfile.insert(rel_path.to_path_buf(), dep_id.clone());
                    }

                    if commit {
                        // update .gitmodules which is at the root of the repo,
                        // not necessarily at the root of the current Foundry project
                        let root = Git::root_of(git.root)?;
                        git.root(&root).add(Some(".gitmodules"))?;
                    }
                }

                if new_insertion
                    || out_of_sync_deps.as_ref().is_some_and(|o| !o.is_empty())
                    || !lockfile.exists()
                {
                    if let Some(transaction) = &mut transaction {
                        transaction.mark_lockfile_touched();
                    }
                    lockfile.write()?;
                }

                // commit the installation
                if commit {
                    let mut msg = String::with_capacity(128);
                    msg.push_str("forge install: ");
                    msg.push_str(dep.name());

                    if let Some(tag) = &installed_tag {
                        msg.push_str("\n\n");

                        if let Some(dep_id) = &dep_id {
                            msg.push_str(&dep_id.to_string());
                        } else {
                            msg.push_str(tag);
                        }
                    }

                    if !lockfile.is_empty() {
                        git.root(&config.root).add(Some(FOUNDRY_LOCK))?;
                    }
                    git.commit(&msg)?;
                }
                if let Some(transaction) = &mut transaction {
                    transaction.disarm();
                }
            }

            let mut msg = format!("    {} {}", "Installed".green(), dep.name);
            if let Some(tag) = dep.tag.or(installed_tag) {
                msg.push(' ');

                if let Some(dep_id) = dep_id {
                    msg.push_str(&dep_id.to_string());
                } else {
                    msg.push_str(tag.as_str());
                }
            }
            sh_status!("{msg}")?;

            // Check if the dependency has soldeer.lock and install soldeer dependencies
            if let Err(e) = install_soldeer_deps_if_needed(&path).await {
                sh_warn!("Failed to install soldeer dependencies for {}: {e}", dep.name)?;
            }
        }

        // update `libs` in config if not included yet
        if !config.libs.iter().any(|p| p == install_lib_dir) {
            config.libs.push(install_lib_dir.to_path_buf());
            config.update_libs()?;
        }

        Ok(())
    }
}

pub async fn install_missing_dependencies(config: &mut Config) -> bool {
    DependencyInstallOpts::default().install_missing_dependencies(config).await
}

/// Checks if a dependency has soldeer.lock and installs soldeer dependencies if needed.
async fn install_soldeer_deps_if_needed(dep_path: &Path) -> Result<()> {
    let soldeer_lock = dep_path.join("soldeer.lock");

    if soldeer_lock.exists() {
        sh_status!("    Found soldeer.lock, installing soldeer dependencies...")?;

        // Change to the dependency directory and run soldeer install
        let original_dir = std::env::current_dir()?;
        std::env::set_current_dir(dep_path)?;

        let result = soldeer_commands::run(
            Command::Install(Install::default()),
            Verbosity::new(
                foundry_common::shell::verbosity(),
                if foundry_common::shell::is_quiet() { 1 } else { 0 },
            ),
        )
        .await;

        // Change back to original directory
        std::env::set_current_dir(original_dir)?;

        result.map_err(|e| eyre::eyre!("Failed to run soldeer install: {e}"))?;
        sh_status!("    Soldeer dependencies installed successfully")?;
    }

    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct Installer<'a> {
    git: Git<'a>,
    commit: bool,
}

struct NewSubmoduleTransaction {
    root: PathBuf,
    relative_path: PathBuf,
    path: PathBuf,
    module_dir: PathBuf,
    gitmodules_contents: Option<Vec<u8>>,
    submodule_config: Vec<(String, String)>,
    lockfile_path: PathBuf,
    lockfile_contents: Option<Vec<u8>>,
    stage_lockfile: bool,
    lockfile_touched: bool,
    armed: bool,
}

impl NewSubmoduleTransaction {
    const fn mark_lockfile_touched(&mut self) {
        self.lockfile_touched = true;
    }

    const fn disarm(&mut self) {
        self.armed = false;
    }

    fn rollback(&self) {
        let git = Git::new(&self.root);
        let owns_worktree = !self.path.is_symlink()
            && self.path.exists()
            && Git::new(&self.path).absolute_git_dir().ok().as_deref() == Some(&self.module_dir);
        if owns_worktree && let Err(err) = git.submodule_deinit(true, &self.relative_path) {
            warn!(%err, "failed to remove submodule config after installation failure");
        }
        if let Err(err) = git.remove_index_path(&self.relative_path) {
            warn!(%err, "failed to remove submodule after installation failure");
        }
        if owns_worktree
            && self.module_dir.exists()
            && let Err(err) = fs::remove_dir_all(&self.module_dir)
        {
            warn!(%err, "failed to remove submodule Git directory after installation failure");
        }
        restore_file(&self.root.join(".gitmodules"), self.gitmodules_contents.as_deref());
        if let Err(err) = git.add(Some(".gitmodules")) {
            warn!(%err, "failed to restore staged .gitmodules after installation failure");
        }
        if self.lockfile_touched {
            restore_file(&self.lockfile_path, self.lockfile_contents.as_deref());
            if self.stage_lockfile
                && let Err(err) = git.add(Some(&self.lockfile_path))
            {
                warn!(%err, "failed to restore staged lockfile after installation failure");
            }
        }
        if owns_worktree
            && self.path.exists()
            && let Err(err) = fs::remove_dir_all(&self.path)
        {
            warn!(%err, "failed to remove dependency after installation failure");
        }
        if let Err(err) = git.restore_submodule_config(&self.relative_path, &self.submodule_config)
        {
            warn!(%err, "failed to restore submodule config after installation failure");
        }
    }
}

impl Drop for NewSubmoduleTransaction {
    fn drop(&mut self) {
        if self.armed {
            self.rollback();
        }
    }
}

fn restore_file(path: &Path, contents: Option<&[u8]>) {
    let result = match contents {
        Some(contents) => fs::write(path, contents),
        None if path.exists() => fs::remove_file(path),
        None => Ok(()),
    };
    if let Err(err) = result {
        warn!(%err, path = %path.display(), "failed to restore file after installation failure");
    }
}

impl Installer<'_> {
    /// Installs the dependency as an ordinary folder instead of a submodule
    fn install_as_folder(self, dep: &Dependency, path: &Path) -> Result<Option<String>> {
        let url = dep.require_url()?;
        Git::clone(dep.tag.is_none(), url, Some(&path))?;
        let mut dep = dep.clone();

        if dep.tag.is_none() {
            // try to find latest semver release tag
            dep.tag = self.last_tag(path);
        }

        // checkout the tag if necessary, using recursive checkout to properly clean up
        // nested submodules that may exist on the default branch but not on the target tag.
        // See: https://github.com/foundry-rs/foundry/issues/13688
        if let Err(err) = self.git_checkout(&dep, path, true) {
            if let Err(cleanup_err) = fs::remove_dir_all(path) {
                warn!(%cleanup_err, "failed to remove dependency after installation failure");
            }
            return Err(err);
        }

        trace!("updating dependency submodules recursively");
        self.git.root(path).submodule_update(
            false,
            false,
            false,
            true,
            std::iter::empty::<PathBuf>(),
        )?;

        // remove nested .git directories from submodules before removing the top-level .git
        Self::remove_nested_git_dirs(path)?;

        // remove git artifacts
        fs::remove_dir_all(path.join(".git"))?;

        Ok(dep.tag)
    }

    /// Recursively removes `.git` files/directories from nested submodules within `root`.
    ///
    /// Submodules typically have a `.git` file (not a directory) pointing to the parent's
    /// `.git/modules/` directory. This cleans those up so the result is a plain folder tree.
    fn remove_nested_git_dirs(root: &Path) -> Result<()> {
        Self::remove_nested_git_dirs_inner(root, root)
    }

    fn remove_nested_git_dirs_inner(root: &Path, dir: &Path) -> Result<()> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return Ok(()),
        };
        for entry in entries {
            let entry = entry?;
            let ft = entry.file_type()?;

            // never follow symlinks
            if ft.is_symlink() {
                continue;
            }

            let path = entry.path();
            if path.file_name() == Some(".git".as_ref()) && path.parent() != Some(root) {
                if ft.is_dir() {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_file(&path)?;
                }
            } else if ft.is_dir() {
                Self::remove_nested_git_dirs_inner(root, &path)?;
            }
        }
        Ok(())
    }

    /// Installs the dependency as new submodule.
    ///
    /// This will add the git submodule to the given dir, initialize it and checkout the tag if
    /// provided or try to find the latest semver, release tag.
    fn install_as_submodule(
        self,
        dep: &Dependency,
        path: &Path,
        project_root: &Path,
    ) -> Result<(Option<String>, Option<NewSubmoduleTransaction>)> {
        let root = Git::root_of(self.git.root)?;
        let relative_path = path.strip_prefix(&root)?;
        let git = self.git.root(&root);
        let gitmodules = root.join(".gitmodules");
        let gitmodules_existed = gitmodules.exists();
        let gitmodules_contents = gitmodules_existed.then(|| fs::read(&gitmodules)).transpose()?;
        let (submodule_name_collision, mapping) = git.submodule_mapping_for_path(relative_path)?;
        let module_name =
            mapping.as_ref().map_or(relative_path, |mapping| Path::new(&mapping.name));
        let module_dir = git.absolute_git_dir()?.join("modules").join(module_name);
        let submodule_config = git.submodule_config(module_name)?;
        if gitmodules_existed && !git.is_tracked(Path::new(".gitmodules"))? {
            eyre::bail!(
                "cannot safely install dependency at {} because the target or .gitmodules has existing changes",
                relative_path.display()
            );
        }
        let existing_submodule = mapping.is_some() && git.is_gitlink(relative_path)?;
        if submodule_name_collision || mapping.is_some() && !existing_submodule {
            eyre::bail!(
                "cannot safely install dependency at {} because .gitmodules already contains a matching submodule",
                relative_path.display()
            );
        }
        if !existing_submodule {
            let can_rollback = !path.is_symlink()
                && !path.exists()
                && !module_dir.exists()
                && !git.has_index_entries(relative_path)?
                && git.is_path_clean(relative_path)?
                && git.is_path_worktree_clean(Path::new(".gitmodules"))?;
            if !can_rollback {
                eyre::bail!(
                    "cannot safely install dependency at {} because the target or .gitmodules has existing changes",
                    relative_path.display()
                );
            }
        } else if mapping.as_ref().and_then(|mapping| mapping.url.as_deref())
            != Some(dep.require_url()?)
        {
            eyre::bail!(
                "cannot install dependency at {} because its registered URL does not match {}",
                relative_path.display(),
                dep.require_url()?
            );
        }
        let uninitialized_worktree = existing_submodule
            && !path.is_symlink()
            && path.is_dir()
            && std::fs::read_dir(path)?.next().is_none();
        let worktree_existed = path.exists();
        let module_dir_existed = module_dir.exists();
        if existing_submodule && (path.is_symlink() || path.exists()) && !uninitialized_worktree {
            self.ensure_submodule_worktree(path, &module_dir, relative_path)?;
        }
        let original_head = (existing_submodule && worktree_existed && !uninitialized_worktree)
            .then(|| self.git.root(path).head())
            .transpose()?;

        let lockfile_path = project_root.join(FOUNDRY_LOCK);
        let lockfile_contents =
            lockfile_path.exists().then(|| fs::read(&lockfile_path)).transpose()?;
        let mut transaction = (!existing_submodule).then(|| NewSubmoduleTransaction {
            root: root.clone(),
            relative_path: relative_path.to_path_buf(),
            path: path.to_path_buf(),
            module_dir: module_dir.clone(),
            gitmodules_contents: gitmodules_contents.clone(),
            submodule_config: submodule_config.clone(),
            lockfile_path,
            lockfile_contents,
            stage_lockfile: self.commit,
            lockfile_touched: false,
            armed: true,
        });
        let setup = if existing_submodule {
            if !path.exists() || uninitialized_worktree {
                git.submodule_update(false, false, false, false, Some(relative_path))
            } else {
                Ok(())
            }
        } else {
            self.git_submodule(dep, path)
        };
        let result = setup.and_then(|()| {
            if existing_submodule {
                self.ensure_submodule_worktree(path, &module_dir, relative_path)?;
            }

            let mut dep = dep.clone();
            if dep.tag.is_none() {
                // try to find latest semver release tag
                dep.tag = self.last_tag(path);
            }

            // checkout the tag if necessary
            self.git_checkout(&dep, path, true)?;

            trace!("updating dependency submodules recursively");
            self.git.root(path).submodule_update(
                false,
                false,
                false,
                true,
                std::iter::empty::<PathBuf>(),
            )?;

            // sync submodules config with changes in .gitmodules, see <https://github.com/foundry-rs/foundry/issues/9611>
            self.git.root(path).submodule_sync()?;

            if self.commit {
                self.git.add_literal(path)?;
            }

            Ok(dep.tag)
        });

        if result.is_err() && existing_submodule {
            if let Some(head) = original_head
                && let Err(err) = self.git.root(path).checkout(false, head)
            {
                warn!(%err, "failed to restore submodule HEAD after installation failure");
            }
            if (uninitialized_worktree || !worktree_existed)
                && let Err(err) = git.submodule_deinit(true, relative_path)
            {
                warn!(%err, "failed to restore deinitialized submodule after installation failure");
            }
            if !worktree_existed
                && path.exists()
                && let Err(err) = fs::remove_dir_all(path)
            {
                warn!(%err, "failed to restore missing submodule worktree");
            }
            if !module_dir_existed
                && module_dir.exists()
                && let Err(err) = fs::remove_dir_all(&module_dir)
            {
                warn!(%err, "failed to remove created submodule Git directory");
            }
            if let Err(err) = git.restore_submodule_config(module_name, &submodule_config) {
                warn!(%err, "failed to restore submodule config after installation failure");
            }
        }

        result.map(|tag| (tag, transaction.take()))
    }

    fn ensure_submodule_worktree(
        self,
        path: &Path,
        module_dir: &Path,
        relative_path: &Path,
    ) -> Result<()> {
        let actual_module_dir = if path.is_symlink() || !path.join(".git").exists() {
            None
        } else {
            self.git.root(path).absolute_git_dir().ok()
        };
        if actual_module_dir.as_deref() != Some(module_dir) {
            eyre::bail!(
                "cannot safely install dependency at {} because the target is not its registered submodule worktree",
                relative_path.display()
            );
        }
        Ok(())
    }

    fn last_tag(self, path: &Path) -> Option<String> {
        if self.git.shallow {
            None
        } else {
            self.git_semver_tags(path).ok().and_then(|mut tags| tags.pop()).map(|(tag, _)| tag)
        }
    }

    /// Returns all semver git tags sorted in ascending order
    fn git_semver_tags(self, path: &Path) -> Result<Vec<(String, Version)>> {
        let out = self.git.root(path).tag()?;
        let mut tags = Vec::new();
        // tags are commonly prefixed which would make them not semver: v1.2.3 is not a semantic
        // version
        let common_prefixes = &["v-", "v", "release-", "release"];
        for tag in out.lines() {
            let mut maybe_semver = tag;
            for &prefix in common_prefixes {
                if let Some(rem) = tag.strip_prefix(prefix) {
                    maybe_semver = rem;
                    break;
                }
            }
            match Version::parse(maybe_semver) {
                Ok(v) => {
                    // ignore if additional metadata, like rc, beta, etc...
                    if v.build.is_empty() && v.pre.is_empty() {
                        tags.push((tag.to_string(), v));
                    }
                }
                Err(err) => {
                    warn!(?err, ?maybe_semver, "No semver tag");
                }
            }
        }

        tags.sort_by(|(_, a), (_, b)| a.cmp(b));

        Ok(tags)
    }

    /// Install the given dependency as git submodule in `target_dir`.
    fn git_submodule(self, dep: &Dependency, path: &Path) -> Result<()> {
        let url = dep.require_url()?;

        // make path relative to the git root, already checked above
        let path = path.strip_prefix(self.git.root).unwrap();

        trace!(?dep, url, ?path, "installing git submodule");
        self.git.submodule_add(true, url, path)
    }

    fn git_checkout(self, dep: &Dependency, path: &Path, recurse: bool) -> Result<String> {
        // no need to checkout if there is no tag
        let Some(mut tag) = dep.tag.clone() else { return Ok(String::new()) };

        let mut is_branch = false;
        // only try to match tag if current terminal is a tty
        if std::io::stdout().is_terminal() {
            if tag.is_empty() {
                tag = self.match_tag(&tag, path)?;
            } else if let Some(branch) = self.match_branch(&tag, path)? {
                trace!(?tag, ?branch, "selecting branch for given tag");
                tag = branch;
                is_branch = true;
            }
        }
        let url = dep.url.as_ref().unwrap();

        let res = self.git.root(path).checkout(recurse, &tag);
        if let Err(mut e) = res {
            if e.to_string().contains("did not match any file(s) known to git") {
                e = eyre::eyre!("Tag: \"{tag}\" not found for repo \"{url}\"!")
            }
            return Err(e);
        }

        if is_branch { Ok(tag) } else { Ok(String::new()) }
    }

    /// disambiguate tag if it is a version tag
    fn match_tag(self, tag: &str, path: &Path) -> Result<String> {
        // only try to match if it looks like a version tag
        if !DEPENDENCY_VERSION_TAG_REGEX.is_match(tag) {
            return Ok(tag.into());
        }

        // generate candidate list by filtering `git tag` output, valid ones are those "starting
        // with" the user-provided tag (ignoring the starting 'v'), for example, if the user
        // specifies 1.5, then v1.5.2 is a valid candidate, but v3.1.5 is not
        let trimmed_tag = tag.trim_start_matches('v').to_string();
        let output = self.git.root(path).tag()?;
        let mut candidates: Vec<String> = output
            .trim()
            .lines()
            .filter(|x| x.trim_start_matches('v').starts_with(&trimmed_tag))
            .map(|x| x.to_string())
            .rev()
            .collect();

        // no match found, fall back to the user-provided tag
        if candidates.is_empty() {
            return Ok(tag.into());
        }

        // have exact match
        for candidate in &candidates {
            if candidate == tag {
                return Ok(tag.into());
            }
        }

        // only one candidate, ask whether the user wants to accept or not
        if candidates.len() == 1 {
            let matched_tag = &candidates[0];
            let input = prompt!(
                "Found a similar version tag: {matched_tag}, do you want to use this instead? [Y/n] "
            )?;
            return if match_yn(input) { Ok(matched_tag.clone()) } else { Ok(tag.into()) };
        }

        // multiple candidates, ask the user to choose one or skip
        candidates.insert(0, String::from("SKIP AND USE ORIGINAL TAG"));
        sh_status!("There are multiple matching tags:")?;
        for (i, candidate) in candidates.iter().enumerate() {
            sh_status!("[{i}] {candidate}")?;
        }

        let n_candidates = candidates.len();
        loop {
            let input: String =
                prompt!("Please select a tag (0-{}, default: 1): ", n_candidates - 1)?;
            let s = input.trim();
            // default selection, return first candidate
            let n = if s.is_empty() { Ok(1) } else { s.parse() };
            // match user input, 0 indicates skipping and use original tag
            match n {
                Ok(0) => return Ok(tag.into()),
                Ok(i) if (1..=n_candidates).contains(&i) => {
                    let c = &candidates[i];
                    sh_status!("[{i}] {c} selected")?;
                    return Ok(c.clone());
                }
                _ => {}
            }
        }
    }

    fn match_branch(self, tag: &str, path: &Path) -> Result<Option<String>> {
        // fetch remote branches and check for tag
        let output = self.git.root(path).remote_branches()?;

        let mut candidates = output
            .lines()
            .map(|x| x.trim().trim_start_matches("origin/"))
            .filter(|x| x.starts_with(tag))
            .map(ToString::to_string)
            .rev()
            .collect::<Vec<_>>();

        trace!(?candidates, ?tag, "found branch candidates");

        // no match found, fall back to the user-provided tag
        if candidates.is_empty() {
            return Ok(None);
        }

        // have exact match
        for candidate in &candidates {
            if candidate == tag {
                return Ok(Some(tag.to_string()));
            }
        }

        // only one candidate, ask whether the user wants to accept or not
        if candidates.len() == 1 {
            let matched_tag = &candidates[0];
            let input = prompt!(
                "Found a similar branch: {matched_tag}, do you want to use this instead? [Y/n] "
            )?;
            return if match_yn(input) { Ok(Some(matched_tag.clone())) } else { Ok(None) };
        }

        // multiple candidates, ask the user to choose one or skip
        candidates.insert(0, format!("{tag} (original branch)"));
        sh_status!("There are multiple matching branches:")?;
        for (i, candidate) in candidates.iter().enumerate() {
            sh_status!("[{i}] {candidate}")?;
        }

        let n_candidates = candidates.len();
        let input: String = prompt!(
            "Please select a tag (0-{}, default: 1, Press <enter> to cancel): ",
            n_candidates - 1
        )?;
        let input = input.trim();

        // default selection, return None
        if input.is_empty() {
            sh_status!("Canceled branch matching")?;
            return Ok(None);
        }

        // match user input, 0 indicates skipping and use original tag
        match input.parse::<usize>() {
            Ok(0) => Ok(Some(tag.into())),
            Ok(i) if (1..=n_candidates).contains(&i) => {
                let c = &candidates[i];
                sh_status!("[{i}] {c} selected")?;
                Ok(Some(c.clone()))
            }
            _ => Ok(None),
        }
    }
}

/// Matches on the result of a prompt for yes/no.
///
/// Defaults to true.
fn match_yn(input: String) -> bool {
    let s = input.trim().to_lowercase();
    matches!(s.as_str(), "" | "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    #[ignore = "slow"]
    fn get_oz_tags() {
        let tmp = tempdir().unwrap();
        let git = Git::new(tmp.path());
        let installer = Installer { git, commit: false };

        git.init().unwrap();

        let dep: Dependency = "openzeppelin/openzeppelin-contracts".parse().unwrap();
        let libs = tmp.path().join("libs");
        fs::create_dir(&libs).unwrap();
        let submodule = libs.join("openzeppelin-contracts");
        installer.git_submodule(&dep, &submodule).unwrap();
        assert!(submodule.exists());

        let tags = installer.git_semver_tags(&submodule).unwrap();
        assert!(!tags.is_empty());
        let v480: Version = "4.8.0".parse().unwrap();
        assert!(tags.iter().any(|(_, v)| v == &v480));
    }
}
