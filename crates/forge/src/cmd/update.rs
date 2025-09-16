use crate::{DepIdentifier, DepMap, Lockfile};
use alloy_primitives::map::HashMap;
use clap::{Parser, ValueHint};
use eyre::{Context, Result};
use foundry_cli::{
    opts::Dependency,
    utils::{CommandUtils, Git, LoadConfig},
};
use foundry_config::{Config, impl_figment_convert_basic};
use std::path::{Path, PathBuf};
use yansi::Paint;

/// CLI arguments for `forge update`.
#[derive(Clone, Debug, Parser)]
pub struct UpdateArgs {
    /// The dependencies you want to update.
    dependencies: Vec<Dependency>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Override the up-to-date check.
    #[arg(short, long)]
    force: bool,

    /// Recursively update submodules.
    #[arg(short, long)]
    recursive: bool,
}
impl_figment_convert_basic!(UpdateArgs);

impl UpdateArgs {
    pub fn run(self) -> Result<()> {
        let config = self.load_config()?;
        // dep_overrides consists of absolute paths of dependencies and their tags
        let (root, _paths, dep_overrides) = dependencies_paths(&self.dependencies, &config)?;
        // Mapping of relative path of lib to its tag type
        // e.g "lib/forge-std" -> DepIdentifier::Tag { name: "v0.1.0", rev: "1234567" }
        let git = Git::new(&root);

        let mut foundry_lock = Lockfile::new(&config.root).with_git(&git);
        let out_of_sync_deps = foundry_lock.sync(config.install_lib_dir())?;

        // update the submodules' tags if any overrides are present
        let mut prev_dep_ids: DepMap = HashMap::default();
        if dep_overrides.is_empty() {
            // running `forge update`, update all deps
            foundry_lock.iter_mut().for_each(|(_path, dep_id)| {
                // Set r#override flag to true if the dep is a branch
                if let DepIdentifier::Branch { .. } = dep_id {
                    dep_id.mark_override();
                }
            });
        } else {
            for (dep_path, override_tag) in &dep_overrides {
                let rel_path = dep_path
                    .strip_prefix(&root)
                    .wrap_err("Dependency path is not relative to the repository root")?;

                if let Ok(mut dep_id) = DepIdentifier::resolve_type(&git, dep_path, override_tag) {
                    // Store the previous state before overriding
                    let prev = foundry_lock.get(rel_path).cloned();

                    // If it's a branch, mark it as overridden so it gets updated below
                    if let DepIdentifier::Branch { .. } = dep_id {
                        dep_id.mark_override();
                    }

                    // Update the lockfile
                    foundry_lock.override_dep(rel_path, dep_id)?;

                    // Only track as updated if there was a previous dependency
                    if let Some(prev) = prev {
                        prev_dep_ids.insert(rel_path.to_owned(), prev);
                    }
                } else {
                    sh_warn!(
                        "Could not r#override submodule at {} with tag {}, try using forge install",
                        rel_path.display(),
                        override_tag
                    )?;
                }
            }
        }

        // fetch the latest changes for each submodule (recursively if flag is set)
        let git = Git::new(&root);
        let update_paths = self.update_dep_paths(&foundry_lock);
        trace!(?update_paths, "updating deps at");

        if self.recursive {
            // update submodules recursively
            git.submodule_update(self.force, true, false, true, update_paths)?;
        } else {
            let is_empty = update_paths.is_empty();

            // update submodules
            git.submodule_update(self.force, true, false, false, update_paths)?;

            if !is_empty {
                // initialize submodules of each submodule recursively (otherwise direct submodule
                // dependencies will revert to last commit)
                git.submodule_foreach(false, "git submodule update --init --progress --recursive")?;
            }
        }

        // Update branches to their latest commit from origin
        // This handles both explicit updates (forge update dep@branch) and
        // general updates (forge update) for branch-tracked dependencies
        let branch_overrides = foundry_lock
            .iter_mut()
            .filter_map(|(path, dep_id)| {
                if dep_id.is_branch() && dep_id.overridden() {
                    return Some((path, dep_id));
                }
                None
            })
            .collect::<Vec<_>>();

        for (path, dep_id) in branch_overrides {
            let submodule_path = root.join(path);
            let name = dep_id.name();

            // Fetch and checkout the latest commit from the remote branch
            Self::fetch_and_checkout_branch(&git, &submodule_path, name)?;

            // Now get the updated revision after syncing with origin
            let (updated_rev, _) = git.current_rev_branch(&submodule_path)?;

            // Update the lockfile entry to reflect the latest commit
            let prev = std::mem::replace(
                dep_id,
                DepIdentifier::Branch {
                    name: name.to_string(),
                    rev: updated_rev,
                    r#override: true,
                },
            );

            // Only insert if we don't already have a previous state for this path
            // (e.g., from explicit overrides where we converted tag to branch)
            if !prev_dep_ids.contains_key(path) {
                prev_dep_ids.insert(path.to_owned(), prev);
            }
        }

        // checkout the submodules at the correct tags
        // Skip branches that were already updated above to avoid reverting to local branch
        for (path, dep_id) in foundry_lock.iter() {
            // Ignore other dependencies if single update.
            if !dep_overrides.is_empty() && !dep_overrides.contains_key(path) {
                continue;
            }

            // Skip branches that were already updated
            if dep_id.is_branch() && dep_id.overridden() {
                continue;
            }
            git.checkout_at(dep_id.checkout_id(), &root.join(path))?;
        }

        if out_of_sync_deps.is_some_and(|o| !o.is_empty())
            || foundry_lock.iter().any(|(_, dep_id)| dep_id.overridden())
        {
            foundry_lock.write()?;
        }

        // Print updates from => to
        for (path, prev) in prev_dep_ids {
            let curr = foundry_lock.get(&path).unwrap();
            sh_println!(
                "Updated dep at '{}', (from: {prev}, to: {curr})",
                path.display().green(),
                prev = prev,
                curr = curr.yellow()
            )?;
        }

        Ok(())
    }

    /// Returns the `lib/paths` of the dependencies that have been updated/overridden.
    fn update_dep_paths(&self, foundry_lock: &Lockfile<'_>) -> Vec<PathBuf> {
        foundry_lock
            .iter()
            .filter_map(|(path, dep_id)| {
                if dep_id.overridden() {
                    return Some(path.to_path_buf());
                }
                None
            })
            .collect()
    }

    /// Fetches and checks out the latest version of a branch from origin
    fn fetch_and_checkout_branch(git: &Git<'_>, path: &Path, branch: &str) -> Result<()> {
        // Fetch the latest changes from origin for the branch
        git.cmd_at(path).args(["fetch", "origin", branch]).exec().wrap_err(format!(
            "Could not fetch latest changes for branch {} in submodule at {}",
            branch,
            path.display()
        ))?;

        // Checkout and track the remote branch to ensure we have the latest commit
        // Using checkout -B ensures the local branch tracks origin/branch
        git.cmd_at(path)
            .args(["checkout", "-B", branch, &format!("origin/{branch}")])
            .exec()
            .wrap_err(format!(
                "Could not checkout and track origin/{} for submodule at {}",
                branch,
                path.display()
            ))?;

        Ok(())
    }
}

/// Returns `(root, paths, overridden_deps_with_abosolute_paths)` where `root` is the root of the
/// Git repository and `paths` are the relative paths of the dependencies.
#[allow(clippy::type_complexity)]
pub fn dependencies_paths(
    deps: &[Dependency],
    config: &Config,
) -> Result<(PathBuf, Vec<PathBuf>, HashMap<PathBuf, String>)> {
    let git_root = Git::root_of(&config.root)?;
    let libs = config.install_lib_dir();

    if deps.is_empty() {
        return Ok((git_root, Vec::new(), HashMap::default()));
    }

    let mut paths = Vec::with_capacity(deps.len());
    let mut overrides = HashMap::with_capacity_and_hasher(deps.len(), Default::default());
    for dep in deps {
        let name = dep.name();
        let dep_path = libs.join(name);
        if !dep_path.exists() {
            eyre::bail!("Could not find dependency {name:?} in {}", dep_path.display());
        }
        let rel_path = dep_path
            .strip_prefix(&git_root)
            .wrap_err("Library directory is not relative to the repository root")?;

        if let Some(tag) = &dep.tag {
            overrides.insert(dep_path.to_owned(), tag.to_owned());
        }
        paths.push(rel_path.to_owned());
    }
    Ok((git_root, paths, overrides))
}
