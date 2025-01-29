use alloy_primitives::map::HashMap;
use clap::{Parser, ValueHint};
use eyre::{Context, Result};
use forge::{DepIdentifier, Lockfile, FOUNDRY_LOCK};
use foundry_cli::{
    opts::Dependency,
    utils::{Git, LoadConfig},
};
use foundry_common::fs;
use foundry_config::{impl_figment_convert_basic, Config};
use std::path::PathBuf;

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
        let foundry_lock_path = root.join(FOUNDRY_LOCK);

        let mut foundry_lock = Lockfile::new(&config.root).with_git(&git);
        let _out_of_sync_deps = foundry_lock.sync()?;
        let prev_len = foundry_lock.len();

        // update the submodules' tags if any overrides are present

        if dep_overrides.is_empty() {
            // running `forge update`, update all deps
            foundry_lock.iter_mut().for_each(|(_path, dep_id)| {
                // Set overide flag to true if the dep is a branch
                if let DepIdentifier::Branch { .. } = dep_id {
                    dep_id.mark_overide();
                }
            });
        } else {
            for (dep_path, override_tag) in &dep_overrides {
                let rel_path = dep_path
                    .strip_prefix(&root)
                    .wrap_err("Dependency path is not relative to the repository root")?;
                if let Ok(dep_id) = DepIdentifier::resolve_type(&git, dep_path, override_tag) {
                    foundry_lock.override_dep(rel_path, dep_id)?
                } else {
                    sh_warn!(
                        "Could not override submodule at {} with tag {}, try using forge install",
                        rel_path.display(),
                        override_tag
                    )?;
                }
            }
        }

        // fetch the latest changes for each submodule (recursively if flag is set)
        let git = Git::new(&root);
        if self.recursive {
            // update submodules recursively
            let update_paths = self.update_dep_paths(&foundry_lock);
            git.submodule_update(self.force, true, false, true, update_paths)?;
        } else {
            let update_paths = self.update_dep_paths(&foundry_lock);
            let is_empty = update_paths.is_empty();
            // update submodules
            git.submodule_update(self.force, true, false, false, update_paths)?;

            if !is_empty {
                // initialize submodules of each submodule recursively (otherwise direct submodule
                // dependencies will revert to last commit)
                git.submodule_foreach(false, "git submodule update --init --progress --recursive")?;
            }
        }

        // checkout the submodules at the correct tags
        for (path, dep_id) in foundry_lock.iter() {
            git.checkout_at(dep_id.checkout_id(), &root.join(path))?;
        }

        if prev_len != foundry_lock.len() ||
            foundry_lock.iter().any(|(_, dep_id)| dep_id.overriden())
        {
            fs::write_json_file(&foundry_lock_path, &foundry_lock)?;
        }

        Ok(())
    }

    /// Returns the `lib/paths` of the dependencies that have been updated/overriden.
    fn update_dep_paths(&self, foundry_lock: &Lockfile<'_>) -> Vec<PathBuf> {
        foundry_lock
            .iter()
            .filter_map(|(path, dep_id)| {
                if dep_id.overriden() {
                    return Some(path.to_path_buf());
                }
                None
            })
            .collect()
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
