use crate::Result;
use std::{
    fs::create_dir_all,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct SourceTreeEntry {
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Clone, Debug)]
pub struct SourceTree {
    pub entries: Vec<SourceTreeEntry>,
}

impl SourceTree {
    /// Expand the source tree into the provided directory.  This method sanitizes paths to ensure
    /// that no directory traversal happens.
    pub fn write_to(&self, dir: &Path) -> Result<()> {
        create_dir_all(dir)?;
        for entry in &self.entries {
            let mut sanitized_path = sanitize_path(&entry.path);
            if sanitized_path.extension().is_none() {
                let with_extension = sanitized_path.with_extension("sol");
                if !self.entries.iter().any(|e| e.path == with_extension) {
                    sanitized_path = with_extension;
                }
            }
            let joined = dir.join(sanitized_path);
            if let Some(parent) = joined.parent() {
                create_dir_all(parent)?;
                std::fs::write(joined, &entry.contents)?;
            }
        }
        Ok(())
    }
}

/// Remove any components in a smart contract source path that could cause a directory traversal.
pub(crate) fn sanitize_path(path: impl AsRef<Path>) -> PathBuf {
    let sanitized = path
        .as_ref()
        .components()
        .filter(|x| x.as_os_str() != Component::ParentDir.as_os_str())
        .collect::<PathBuf>();

    // Force absolute paths to be relative
    sanitized.strip_prefix("/").map(PathBuf::from).unwrap_or(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::read_dir;

    /// Ensure that the source tree is written correctly and .sol extension is added to a path with
    /// no extension.
    #[test]
    fn test_source_tree_write() {
        let tempdir = tempfile::tempdir().unwrap();
        let st = SourceTree {
            entries: vec![
                SourceTreeEntry { path: PathBuf::from("a/a.sol"), contents: String::from("Test") },
                SourceTreeEntry { path: PathBuf::from("b/b"), contents: String::from("Test 2") },
            ],
        };
        st.write_to(tempdir.path()).unwrap();
        let a_sol_path = PathBuf::new().join(&tempdir).join("a").join("a.sol");
        let b_sol_path = PathBuf::new().join(&tempdir).join("b").join("b.sol");
        assert!(a_sol_path.exists());
        assert!(b_sol_path.exists());
    }

    /// Ensure that the .. are ignored when writing the source tree to disk because of
    /// sanitization.
    #[test]
    fn test_malformed_source_tree_write() {
        let tempdir = tempfile::tempdir().unwrap();
        let st = SourceTree {
            entries: vec![
                SourceTreeEntry {
                    path: PathBuf::from("../a/a.sol"),
                    contents: String::from("Test"),
                },
                SourceTreeEntry {
                    path: PathBuf::from("../b/../b.sol"),
                    contents: String::from("Test 2"),
                },
                SourceTreeEntry {
                    path: PathBuf::from("/c/c.sol"),
                    contents: String::from("Test 3"),
                },
            ],
        };
        st.write_to(tempdir.path()).unwrap();
        let written_paths = read_dir(tempdir.path()).unwrap();
        let paths: Vec<PathBuf> =
            written_paths.into_iter().filter_map(|x| x.ok()).map(|x| x.path()).collect();
        assert_eq!(paths.len(), 3);
        assert!(paths.contains(&tempdir.path().join("a")));
        assert!(paths.contains(&tempdir.path().join("b")));
        assert!(paths.contains(&tempdir.path().join("c")));
    }
}
