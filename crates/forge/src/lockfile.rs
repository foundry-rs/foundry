//! foundry.lock handler type.

use std::{
    collections::hash_map::Entry,
    path::{Path, PathBuf},
};

use alloy_primitives::map::HashMap;
use eyre::Result;
use foundry_cli::utils::Git;
use serde::{Deserialize, Serialize};

pub const FOUNDRY_LOCK: &str = "foundry.lock";

/// A type alias for a HashMap of dependencies keyed by relative path to the submodule dir.
pub type DepMap = HashMap<PathBuf, DepIdentifier>;

/// A lockfile handler that keeps track of the dependencies and their current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile<'a> {
    /// A map of the dependencies keyed by relative path to the submodule dir.
    #[serde(flatten)]
    deps: DepMap,
    /// This is optional to handle no-git scencarios.
    #[serde(skip)]
    git: Option<&'a Git<'a>>,
    /// Absolute path to the project root. This may not be the git root. e.g monorepo setups.
    #[serde(skip)]
    lockfile_path: PathBuf,
}

impl<'a> Lockfile<'a> {
    /// Create a new [`Lockfile`] instance.
    ///
    /// `project_root` is the absolute path to the project root.
    ///
    /// You will need to call [`Lockfile::read`] or [`Lockfile::sync`] to load the lockfile.
    pub fn new(project_root: &Path) -> Self {
        Self { deps: HashMap::default(), git: None, lockfile_path: project_root.join(FOUNDRY_LOCK) }
    }

    /// Set the git instance to be used for submodule operations.
    pub fn with_git(mut self, git: &'a Git<'_>) -> Self {
        self.git = Some(git);
        self
    }

    /// Sync the foundry.lock file with the current state of `git submodules`.
    ///
    /// If the lockfile and git submodules are out of sync, it returns a [`DepMap`] consisting of
    /// _only_ the out-of-sync dependencies.
    ///
    /// This method writes the lockfile to project root if:
    /// - The lockfile does not exist.
    /// - The lockfile is out of sync with the git submodules.
    pub fn sync(&mut self) -> Result<Option<DepMap>> {
        match self.read() {
            Ok(_) => {}
            Err(e) => {
                if !e.to_string().contains("Lockfile not found") {
                    return Err(e);
                }
            }
        }

        if let Some(git) = &self.git {
            let submodules = git.submodules()?;

            if submodules.is_empty() {
                trace!("No submodules found. Skipping sync.");
                if !self.lockfile_path.exists() && !self.deps.is_empty() {
                    self.write()?;
                }
                return Ok(None);
            }

            let mut out_of_sync: DepMap = HashMap::default();
            for sub in &submodules {
                let rel_path = sub.path();
                let rev = sub.rev();

                let entry = self.deps.entry(rel_path.to_path_buf());

                match entry {
                    Entry::Occupied(e) => {
                        if e.get().rev() != rev {
                            out_of_sync.insert(rel_path.to_path_buf(), e.get().clone());
                        }
                    }
                    Entry::Vacant(e) => {
                        // Find out if rev has a tag associated with it.
                        let maybe_tag =
                            git.tag_for_commit(rev, &git.root.join(rel_path)).or_else(|err| {
                                // Ignore Err: No such file or directory as it is possible that lib/
                                // dir has been cleaned.
                                if err.to_string().contains("No such file or directory") {
                                    return Ok(None)
                                }
                                Err(err)
                            })?;

                        let dep_id = if let Some(tag) = maybe_tag {
                            DepIdentifier::Tag { name: tag, rev: rev.to_string() }
                        } else {
                            DepIdentifier::Rev(rev.to_string())
                        };
                        e.insert(dep_id.clone());
                        out_of_sync.insert(rel_path.to_path_buf(), dep_id);
                    }
                }
            }

            // Write the updated lockfile
            if !out_of_sync.is_empty() || !self.lockfile_path.exists() {
                self.write()?;
            }

            return Ok(if out_of_sync.is_empty() { None } else { Some(out_of_sync) });
        }

        Ok(None)
    }

    /// Loads the lockfile from the project root.
    ///
    /// Throws an error if the lockfile does not exist.
    pub fn read(&mut self) -> Result<()> {
        if !self.lockfile_path.exists() {
            return Err(eyre::eyre!("Lockfile not found at {}", self.lockfile_path.display()));
        }

        let lockfile_str = foundry_common::fs::read_to_string(&self.lockfile_path)?;

        self.deps = serde_json::from_str(&lockfile_str)?;

        trace!(lockfile = ?self.deps, "loaded lockfile");

        Ok(())
    }

    /// Writes the lockfile to the project root.
    pub fn write(&self) -> Result<()> {
        foundry_common::fs::write_json_file(&self.lockfile_path, &self.deps)?;
        trace!(at= ?self.lockfile_path, "wrote lockfile");

        Ok(())
    }

    /// Insert a dependency into the lockfile.
    /// If the dependency already exists, it will be updated.
    ///
    /// Note: This does not write the updated lockfile to disk, only inserts the dep in-memory.
    pub fn insert(&mut self, path: PathBuf, dep_id: DepIdentifier) {
        self.deps.insert(path, dep_id);
    }

    /// Get the [`DepIdentifier`] for a submodule at a given path.
    pub fn get(&self, path: &Path) -> Option<&DepIdentifier> {
        self.deps.get(path)
    }

    /// Removes a dependency from the lockfile.
    ///
    /// Note: This does not write the updated lockfile to disk, only removes the dep in-memory.
    pub fn remove(&mut self, path: &Path) -> Option<DepIdentifier> {
        self.deps.remove(path)
    }

    /// Returns the num of dependencies in the lockfile.
    pub fn len(&self) -> usize {
        self.deps.len()
    }

    /// Returns whether the lockfile is empty.
    pub fn is_empty(&self) -> bool {
        self.deps.is_empty()
    }

    /// Returns an iterator over the lockfile.
    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &DepIdentifier)> {
        self.deps.iter()
    }
}

// Implement .iter() for &LockFile

/// Identifies whether a dependency (submodule) is referenced by a branch,
/// tag or rev (commit hash).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DepIdentifier {
    /// `name` of the branch and the `rev`  it is currently pointing to.
    /// Running `forge update`, will update the `name` branch to the latest `rev`.
    #[serde(rename = "branch")]
    Branch { name: String, rev: String },
    /// Release tag `name` and the `rev` it is currently pointing to.
    /// Running `forge update` does not update the tag/rev.
    /// Dependency will remain pinned to the existing tag/rev unless overridden like so `forge
    /// update owner/dep@tag=diffent_tag`.
    #[serde(rename = "tag")]
    Tag { name: String, rev: String },
    /// Commit hash `rev` the submodule is currently pointing to.
    /// Running `forge update` does not update the rev.
    /// Dependency will remain pinned to the existing rev unless overridden.
    #[serde(rename = "rev")]
    Rev(String),
}

impl DepIdentifier {
    /// Resolves the [`DepIdentifier`] for a submodule at a given path.
    /// `lib_path` is the absolute path to the submodule.
    pub fn resolve_type(git: &Git<'_>, lib_path: &Path, s: &str) -> Result<Self> {
        // Get the tags for the submodule
        if git.has_tag(s, lib_path)? {
            let rev = git.get_rev(s, lib_path)?;
            return Ok(Self::Tag { name: String::from(s), rev });
        }

        if git.has_branch(s, lib_path)? {
            let rev = git.get_rev(s, lib_path)?;
            return Ok(Self::Branch { name: String::from(s), rev });
        }

        if git.has_rev(s, lib_path)? {
            return Ok(Self::Rev(String::from(s)));
        }

        Err(eyre::eyre!("Could not resolve tag type for submodule at path {}", lib_path.display()))
    }

    /// Get the commit hash of the dependency.
    pub fn rev(&self) -> &str {
        match self {
            Self::Branch { rev, .. } => rev,
            Self::Tag { rev, .. } => rev,
            Self::Rev(rev) => rev,
        }
    }

    /// Get the name/rev to checkout at.
    pub fn checkout_id(&self) -> &str {
        match self {
            Self::Branch { name, .. } => name,
            Self::Tag { name, .. } => name,
            Self::Rev(rev) => rev,
        }
    }
}

impl std::fmt::Display for DepIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Branch { name, rev } => write!(f, "branch={name}@{rev}"),
            Self::Tag { name, rev } => write!(f, "tag={name}@{rev}"),
            Self::Rev(rev) => write!(f, "rev={rev}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_dep_identifier() {
        let branch = DepIdentifier::Branch {
            name: "main".to_string(),
            rev: "b7954c3e9ce1d487b49489f5800f52f4b77b7351".to_string(),
        };

        let tag = DepIdentifier::Tag {
            name: "v0.1.0".to_string(),
            rev: "b7954c3e9ce1d487b49489f5800f52f4b77b7351".to_string(),
        };

        let rev = DepIdentifier::Rev("b7954c3e9ce1d487b49489f5800f52f4b77b7351".to_string());

        let branch_str = serde_json::to_string(&branch).unwrap();
        let tag_str = serde_json::to_string(&tag).unwrap();
        let rev_str = serde_json::to_string(&rev).unwrap();

        assert_eq!(
            branch_str,
            r#"{"branch":{"name":"main","rev":"b7954c3e9ce1d487b49489f5800f52f4b77b7351"}}"#
        );
        assert_eq!(
            tag_str,
            r#"{"tag":{"name":"v0.1.0","rev":"b7954c3e9ce1d487b49489f5800f52f4b77b7351"}}"#
        );
        assert_eq!(rev_str, r#"{"rev":"b7954c3e9ce1d487b49489f5800f52f4b77b7351"}"#);

        let branch_de: DepIdentifier = serde_json::from_str(&branch_str).unwrap();
        let tag_de: DepIdentifier = serde_json::from_str(&tag_str).unwrap();
        let rev_de: DepIdentifier = serde_json::from_str(&rev_str).unwrap();

        assert_eq!(branch, branch_de);
        assert_eq!(tag, tag_de);
        assert_eq!(rev, rev_de);
    }
}
