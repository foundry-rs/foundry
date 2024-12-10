use alloy_primitives::map::HashMap;
use clap::{Parser, ValueHint};
use eyre::{Context, Result};
use foundry_cli::{
    opts::Dependency,
    utils::{Git, LoadConfig, TagType},
};
use foundry_common::fs;
use foundry_config::{impl_figment_convert_basic, Config};
use std::{collections::hash_map::Entry, path::PathBuf};

use super::install::FORGE_SUBMODULES_INFO;

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
        // dep_overrides consists of absolute paths of dependencies and their tags
        let (root, paths, dep_overrides) = dependencies_paths(&self.dependencies, &config)?;
        // Mapping of relative path of lib to its tag type
        // e.g "lib/forge-std" -> TagType::Tag("v0.1.0")
        let mut submodule_infos: HashMap<PathBuf, TagType> =
            fs::read_json_file(&root.join(FORGE_SUBMODULES_INFO)).unwrap_or_default();

        let prev_len = submodule_infos.len();

        let git = Git::new(&root);
        let submodules = git.submodules()?;
        for submodule in submodules {
            if let Ok(Some(tag)) = git.tag_for_commit(submodule.rev(), &root.join(submodule.path()))
            {
                if let Entry::Vacant(entry) = submodule_infos.entry(submodule.path().to_path_buf())
                {
                    entry.insert(TagType::Tag(tag));
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

        let mut overridden = false;
        // update the submodules' tags if any overrides are present
        for (dep_path, override_tag) in &dep_overrides {
            let rel_path = dep_path
                .strip_prefix(&root)
                .wrap_err("Dependency path is not relative to the repository root")?;
            if let Ok(tag_type) = TagType::resolve_type(&git, dep_path, override_tag) {
                submodule_infos.insert(rel_path.to_path_buf(), tag_type);
                overridden = true;
            } else {
                sh_warn!(
                    "Could not override submodule at {} with tag {}, try using forge install",
                    rel_path.display(),
                    override_tag
                )?;
            }
        }

        for (path, tag) in &submodule_infos {
            git.checkout_at(tag.raw_string(), &root.join(path))?;
        }

        if prev_len < submodule_infos.len() || overridden {
            fs::write_json_file(&root.join(FORGE_SUBMODULES_INFO), &submodule_infos)?;
        }

        Ok(())
    }
}

/// Returns `(root, paths, overridden_deps_with_abosolute_paths)` where `root` is the root of the
/// Git repository and `paths` are the relative paths of the dependencies.
#[allow(clippy::type_complexity)]
pub fn dependencies_paths(
    deps: &[Dependency],
    config: &Config,
) -> Result<(PathBuf, Vec<PathBuf>, Vec<(PathBuf, String)>)> {
    let git_root = Git::root_of(&config.root)?;
    let libs = config.install_lib_dir();

    let mut paths = Vec::with_capacity(deps.len());
    let mut overrides = Vec::with_capacity(deps.len());
    for dep in deps {
        let name = dep.name();
        let dep_path = libs.join(name);
        let rel_path = dep_path
            .strip_prefix(&git_root)
            .wrap_err("Library directory is not relative to the repository root")?;
        if !dep_path.exists() {
            eyre::bail!("Could not find dependency {name:?} in {}", dep_path.display());
        }

        if let Some(tag) = &dep.tag {
            overrides.push((dep_path.to_owned(), tag.to_owned()));
        }
        paths.push(rel_path.to_owned());
    }
    Ok((git_root, paths, overrides))
}
