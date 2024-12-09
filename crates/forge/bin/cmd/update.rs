use alloy_primitives::map::HashMap;
use clap::{Parser, ValueHint};
use eyre::{Context, Result};
use foundry_cli::{
    opts::Dependency,
    utils::{Git, LoadConfig, TagType},
};
use foundry_common::fs;
use foundry_config::{impl_figment_convert_basic, Config};
use std::{collections::hash_map::Entry, os::unix::process::CommandExt, path::PathBuf};

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
        let config = self.try_load_config_emit_warnings()?;
        let (root, paths) = dependencies_paths(&self.dependencies, &config)?;
        let mut submodule_infos: HashMap<PathBuf, TagType> =
            fs::read_json_file(&root.join("submodules-info.json")).unwrap_or_default();

        let prev_len = submodule_infos.len();

        let git = Git::new(&root);
        let submodules = git.submodules()?;
        // Lock the submodule to current revs unless a branch has been specified for a submodule
        for submodule in submodules {
            if let Ok(Some(tag)) = git.tag_for_commit(submodule.rev(), &root.join(submodule.path()))
            {
                match submodule_infos.entry(submodule.path().to_path_buf()) {
                    Entry::Vacant(entry) => {
                        entry.insert(TagType::Tag(tag));
                    }
                    _ => {}
                }
            }
        }
        // fetch the latest changes for each submodule (recursively if flag is set)
        let git = Git::new(&root);
        if self.recursive {
            // update submodules recursively
            git.submodule_update(self.force, true, false, true, paths)?;
        } else {
            // update root submodules
            git.submodule_update(self.force, true, false, false, paths)?;
            // initialize submodules of each submodule recursively (otherwise direct submodule
            // dependencies will revert to last commit)
            git.submodule_foreach(false, "git submodule update --init --progress --recursive")?;
        }
        for (path, tag) in &submodule_infos {
            // We don't need to check for branches as they are supported by `git submodules`
            // internally
            if let TagType::Tag(tag) | TagType::Rev(tag) = tag {
                git.checkout_at(&tag, &root.join(path))?;
            }
        }

        if prev_len < submodule_infos.len() {
            fs::write_json_file(&root.join("submodules-info.json"), &submodule_infos)?;
        }

        Ok(())
    }
}

/// Returns `(root, paths)` where `root` is the root of the Git repository and `paths` are the
/// relative paths of the dependencies.
pub fn dependencies_paths(deps: &[Dependency], config: &Config) -> Result<(PathBuf, Vec<PathBuf>)> {
    let git_root = Git::root_of(&config.root)?;
    let libs = config.install_lib_dir();

    let mut paths = Vec::with_capacity(deps.len());
    for dep in deps {
        let name = dep.name();
        let dep_path = libs.join(name);
        let rel_path = dep_path
            .strip_prefix(&git_root)
            .wrap_err("Library directory is not relative to the repository root")?;
        if !dep_path.exists() {
            eyre::bail!("Could not find dependency {name:?} in {}", dep_path.display());
        }
        paths.push(rel_path.to_owned());
    }
    Ok((git_root, paths))
}
