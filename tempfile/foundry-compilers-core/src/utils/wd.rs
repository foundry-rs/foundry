use super::SOLC_EXTENSIONS;
use crate::error::SolcError;
use semver::Version;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

/// Returns an iterator that yields all solidity/yul files funder under the given root path or the
/// `root` itself, if it is a sol/yul file
///
/// This also follows symlinks.
pub fn source_files_iter<'a>(
    root: &Path,
    extensions: &'a [&'a str],
) -> impl Iterator<Item = PathBuf> + 'a {
    WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path().extension().map(|ext| extensions.iter().any(|e| ext == *e)).unwrap_or_default()
        })
        .map(|e| e.path().into())
}

/// Returns a list of absolute paths to all the solidity files under the root, or the file itself,
/// if the path is a solidity file.
///
/// This also follows symlinks.
///
/// NOTE: this does not resolve imports from other locations
///
/// # Examples
///
/// ```no_run
/// use foundry_compilers_core::utils;
/// let sources = utils::source_files("./contracts".as_ref(), &utils::SOLC_EXTENSIONS);
/// ```
pub fn source_files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    source_files_iter(root, extensions).collect()
}

/// Same as [source_files] but only returns files acceptable by Solc compiler.
pub fn sol_source_files(root: &Path) -> Vec<PathBuf> {
    source_files(root, SOLC_EXTENSIONS)
}

/// Returns a list of _unique_ paths to all folders under `root` that contain at least one solidity
/// file (`*.sol`).
///
/// # Examples
///
/// ```no_run
/// use foundry_compilers_core::utils;
/// let dirs = utils::solidity_dirs("./lib".as_ref());
/// ```
///
/// for following layout will return
/// `["lib/ds-token/src", "lib/ds-token/src/test", "lib/ds-token/lib/ds-math/src", ...]`
///
/// ```text
/// lib
/// └── ds-token
///     ├── lib
///     │   ├── ds-math
///     │   │   └── src/Contract.sol
///     │   ├── ds-stop
///     │   │   └── src/Contract.sol
///     │   ├── ds-test
///     │       └── src//Contract.sol
///     └── src
///         ├── base.sol
///         ├── test
///         │   ├── base.t.sol
///         └── token.sol
/// ```
pub fn solidity_dirs(root: &Path) -> Vec<PathBuf> {
    let sources = sol_source_files(root);
    sources
        .iter()
        .filter_map(|p| p.parent())
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|p| p.to_path_buf())
        .collect()
}

/// Reads the list of Solc versions that have been installed in the machine.
///
/// The version list is sorted in ascending order.
///
/// Checks for installed solc versions under the given path as `<root>/<major.minor.path>`,
/// (e.g.: `~/.svm/0.8.10`) and returns them sorted in ascending order.
pub fn installed_versions(root: &Path) -> Result<Vec<Version>, SolcError> {
    let mut versions: Vec<_> = walkdir::WalkDir::new(root)
        .max_depth(1)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_dir())
        .filter_map(|e: walkdir::DirEntry| {
            e.path().file_name().and_then(|v| Version::parse(v.to_string_lossy().as_ref()).ok())
        })
        .collect();
    versions.sort();
    Ok(versions)
}

/// Attempts to find a file with different case that exists next to the `non_existing` file
pub fn find_case_sensitive_existing_file(non_existing: &Path) -> Option<PathBuf> {
    let non_existing_file_name = non_existing.file_name()?;
    let parent = non_existing.parent()?;
    WalkDir::new(parent)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .find_map(|e| {
            let existing_file_name = e.path().file_name()?;
            if existing_file_name.eq_ignore_ascii_case(non_existing_file_name)
                && existing_file_name != non_existing_file_name
            {
                return Some(e.path().to_path_buf());
            }
            None
        })
}

#[cfg(test)]
mod tests {
    use super::{super::tests::*, *};

    #[test]
    fn can_find_solidity_sources() {
        let tmp_dir = tempdir("contracts").unwrap();

        let file_a = tmp_dir.path().join("a.sol");
        let file_b = tmp_dir.path().join("a.sol");
        let nested = tmp_dir.path().join("nested");
        let file_c = nested.join("c.sol");
        let nested_deep = nested.join("deep");
        let file_d = nested_deep.join("d.sol");
        File::create(&file_a).unwrap();
        File::create(&file_b).unwrap();
        create_dir_all(nested_deep).unwrap();
        File::create(&file_c).unwrap();
        File::create(&file_d).unwrap();

        let files: HashSet<_> = sol_source_files(tmp_dir.path()).into_iter().collect();
        let expected: HashSet<_> = [file_a, file_b, file_c, file_d].into();
        assert_eq!(files, expected);
    }

    #[test]
    fn can_find_different_case() {
        let tmp_dir = tempdir("out").unwrap();
        let path = tmp_dir.path().join("forge-std");
        create_dir_all(&path).unwrap();
        let existing = path.join("Test.sol");
        let non_existing = path.join("test.sol");
        fs::write(&existing, b"").unwrap();

        #[cfg(target_os = "linux")]
        assert!(!non_existing.exists());

        let found = find_case_sensitive_existing_file(&non_existing).unwrap();
        assert_eq!(found, existing);
    }
}
