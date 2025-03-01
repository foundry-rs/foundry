//! Utility functions

use crate::error::{SolcError, SolcIoError};
use alloy_primitives::{hex, keccak256};
use cfg_if::cfg_if;
use semver::{Version, VersionReq};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs,
    io::Write,
    ops::Range,
    path::{Component, Path, PathBuf},
    sync::LazyLock as Lazy,
};

#[cfg(feature = "regex")]
mod re;
#[cfg(feature = "regex")]
pub use re::*;

#[cfg(feature = "walkdir")]
mod wd;
#[cfg(feature = "walkdir")]
pub use wd::*;

/// Extensions acceptable by solc compiler.
pub const SOLC_EXTENSIONS: &[&str] = &["sol", "yul"];

/// Support for configuring the EVM version
/// <https://blog.soliditylang.org/2018/03/08/solidity-0.4.21-release-announcement/>
pub const BYZANTIUM_SOLC: Version = Version::new(0, 4, 21);

/// Bug fix for configuring the EVM version with Constantinople
/// <https://blog.soliditylang.org/2018/03/08/solidity-0.4.21-release-announcement/>
pub const CONSTANTINOPLE_SOLC: Version = Version::new(0, 4, 22);

/// Petersburg support
/// <https://blog.soliditylang.org/2019/03/05/solidity-0.5.5-release-announcement/>
pub const PETERSBURG_SOLC: Version = Version::new(0, 5, 5);

/// Istanbul support
/// <https://blog.soliditylang.org/2019/12/09/solidity-0.5.14-release-announcement/>
pub const ISTANBUL_SOLC: Version = Version::new(0, 5, 14);

/// Berlin support
/// <https://blog.soliditylang.org/2021/06/10/solidity-0.8.5-release-announcement/>
pub const BERLIN_SOLC: Version = Version::new(0, 8, 5);

/// London support
/// <https://blog.soliditylang.org/2021/08/11/solidity-0.8.7-release-announcement/>
pub const LONDON_SOLC: Version = Version::new(0, 8, 7);

/// Paris support
/// <https://blog.soliditylang.org/2023/02/01/solidity-0.8.18-release-announcement/>
pub const PARIS_SOLC: Version = Version::new(0, 8, 18);

/// Shanghai support
/// <https://blog.soliditylang.org/2023/05/10/solidity-0.8.20-release-announcement/>
pub const SHANGHAI_SOLC: Version = Version::new(0, 8, 20);

/// Cancun support
/// <https://soliditylang.org/blog/2024/01/26/solidity-0.8.24-release-announcement/>
pub const CANCUN_SOLC: Version = Version::new(0, 8, 24);

/// Prague support
/// <https://soliditylang.org/blog/2024/09/04/solidity-0.8.27-release-announcement>
pub const PRAGUE_SOLC: Version = Version::new(0, 8, 27);

// `--base-path` was introduced in 0.6.9 <https://github.com/ethereum/solidity/releases/tag/v0.6.9>
pub static SUPPORTS_BASE_PATH: Lazy<VersionReq> =
    Lazy::new(|| VersionReq::parse(">=0.6.9").unwrap());

// `--include-path` was introduced in 0.8.8 <https://github.com/ethereum/solidity/releases/tag/v0.8.8>
pub static SUPPORTS_INCLUDE_PATH: Lazy<VersionReq> =
    Lazy::new(|| VersionReq::parse(">=0.8.8").unwrap());

/// Move a range by a specified offset
pub fn range_by_offset(range: &Range<usize>, offset: isize) -> Range<usize> {
    Range {
        start: offset.saturating_add(range.start as isize) as usize,
        end: offset.saturating_add(range.end as isize) as usize,
    }
}

/// Returns the source name for the given source path, the ancestors of the root path.
///
/// `/Users/project/sources/contract.sol` -> `sources/contracts.sol`
pub fn source_name<'a>(source: &'a Path, root: &Path) -> &'a Path {
    strip_prefix(source, root)
}

/// Strips `root` from `source` and returns the relative path.
pub fn strip_prefix<'a>(source: &'a Path, root: &Path) -> &'a Path {
    source.strip_prefix(root).unwrap_or(source)
}

/// Strips `root` from `source` and returns the relative path.
pub fn strip_prefix_owned(source: PathBuf, root: &Path) -> PathBuf {
    source.strip_prefix(root).map(Path::to_path_buf).unwrap_or(source)
}

/// Attempts to determine if the given source is a local, relative import.
pub fn is_local_source_name(libs: &[impl AsRef<Path>], source: impl AsRef<Path>) -> bool {
    resolve_library(libs, source.as_ref()).is_none()
}

/// Canonicalize the path, platform-agnostic.
///
/// On windows this will ensure the path only consists of `/` separators.
pub fn canonicalize(path: impl AsRef<Path>) -> Result<PathBuf, SolcIoError> {
    let path = path.as_ref();
    let res = dunce::canonicalize(path);
    #[cfg(windows)]
    let res = res.map(|p| {
        use path_slash::PathBufExt;
        PathBuf::from(p.to_slash_lossy().as_ref())
    });
    res.map_err(|err| SolcIoError::new(err, path))
}

/// Returns a normalized Solidity file path for the given import path based on the specified
/// directory.
///
/// This function resolves `./` and `../`, but, unlike [`canonicalize`], it does not resolve
/// symbolic links.
///
/// The function returns an error if the normalized path does not exist in the file system.
///
/// See also: <https://docs.soliditylang.org/en/v0.8.23/path-resolution.html>
pub fn normalize_solidity_import_path(
    directory: &Path,
    import_path: &Path,
) -> Result<PathBuf, SolcIoError> {
    let original = directory.join(import_path);
    let cleaned = clean_solidity_path(&original);

    // this is to align the behavior with `canonicalize`
    let normalized = dunce::simplified(&cleaned);
    #[cfg(windows)]
    let normalized = {
        use path_slash::PathExt;
        PathBuf::from(normalized.to_slash_lossy().as_ref())
    };
    #[cfg(not(windows))]
    let normalized = PathBuf::from(normalized);

    // checks if the path exists without reading its content and obtains an io error if it doesn't.
    let _ = normalized.metadata().map_err(|err| SolcIoError::new(err, original))?;
    Ok(normalized)
}

// This function lexically cleans the given path.
//
// It performs the following transformations for the path:
//
// * Resolves references (current directories (`.`) and parent (`..`) directories).
// * Reduces repeated separators to a single separator (e.g., from `//` to `/`).
//
// This transformation is lexical, not involving the file system, which means it does not account
// for symlinks. This approach has a caveat. For example, consider a filesystem-accessible path
// `a/b/../c.sol` passed to this function. It returns `a/c.sol`. However, if `b` is a symlink,
// `a/c.sol` might not be accessible in the filesystem in some environments. Despite this, it's
// unlikely that this will pose a problem for our intended use.
//
// # How it works
//
// The function splits the given path into components, where each component roughly corresponds to a
// string between separators. It then iterates over these components (starting from the leftmost
// part of the path) to reconstruct the path. The following steps are applied to each component:
//
// * If the component is a current directory, it's removed.
// * If the component is a parent directory, the following rules are applied:
//     * If the preceding component is a normal, then both the preceding normal component and the
//       parent directory component are removed. (Examples of normal components include `a` and `b`
//       in `a/b`.)
//     * Otherwise (if there is no preceding component, or if the preceding component is a parent,
//       root, or prefix), it remains untouched.
// * Otherwise, the component remains untouched.
//
// Finally, the processed components are reassembled into a path.
fn clean_solidity_path(original_path: &Path) -> PathBuf {
    let mut new_path = Vec::new();

    for component in original_path.components() {
        match component {
            Component::Prefix(..) | Component::RootDir | Component::Normal(..) => {
                new_path.push(component);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(Component::Normal(..)) = new_path.last() {
                    new_path.pop();
                } else {
                    new_path.push(component);
                }
            }
        }
    }

    new_path.iter().collect()
}

/// Returns the same path config but with canonicalized paths.
///
/// This will take care of potential symbolic linked directories.
/// For example, the tempdir library is creating directories hosted under `/var/`, which in OS X
/// is a symbolic link to `/private/var/`. So if when we try to resolve imports and a path is
/// rooted in a symbolic directory we might end up with different paths for the same file, like
/// `private/var/.../Dapp.sol` and `/var/.../Dapp.sol`
///
/// This canonicalizes all the paths but does not treat non existing dirs as an error
pub fn canonicalized(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    canonicalize(&path).unwrap_or(path)
}

/// Returns the path to the library if the source path is in fact determined to be a library path,
/// and it exists.
/// Note: this does not handle relative imports or remappings.
pub fn resolve_library(libs: &[impl AsRef<Path>], source: impl AsRef<Path>) -> Option<PathBuf> {
    let source = source.as_ref();
    let comp = source.components().next()?;
    match comp {
        Component::Normal(first_dir) => {
            // attempt to verify that the root component of this source exists under a library
            // folder
            for lib in libs {
                let lib = lib.as_ref();
                let contract = lib.join(source);
                if contract.exists() {
                    // contract exists in <lib>/<source>
                    return Some(contract);
                }
                // check for <lib>/<first_dir>/src/name.sol
                let contract = lib
                    .join(first_dir)
                    .join("src")
                    .join(source.strip_prefix(first_dir).expect("is first component"));
                if contract.exists() {
                    return Some(contract);
                }
            }
            None
        }
        Component::RootDir => Some(source.into()),
        _ => None,
    }
}

/// Tries to find an absolute import like `src/interfaces/IConfig.sol` in `cwd`, moving up the path
/// until the `root` is reached.
///
/// If an existing file under `root` is found, this returns the path up to the `import` path and the
/// normalized `import` path itself:
///
/// For example for following layout:
///
/// ```text
/// <root>/mydependency/
/// ├── src (`cwd`)
/// │   ├── interfaces
/// │   │   ├── IConfig.sol
/// ```
/// and `import` as `src/interfaces/IConfig.sol` and `cwd` as `src` this will return
/// (`<root>/mydependency/`, `<root>/mydependency/src/interfaces/IConfig.sol`)
pub fn resolve_absolute_library(
    root: &Path,
    cwd: &Path,
    import: &Path,
) -> Option<(PathBuf, PathBuf)> {
    let mut parent = cwd.parent()?;
    while parent != root {
        if let Ok(import) = normalize_solidity_import_path(parent, import) {
            return Some((parent.to_path_buf(), import));
        }
        parent = parent.parent()?;
    }
    None
}

/// Returns the 36 char (deprecated) fully qualified name placeholder
///
/// If the name is longer than 36 char, then the name gets truncated,
/// If the name is shorter than 36 char, then the name is filled with trailing `_`
pub fn library_fully_qualified_placeholder(name: &str) -> String {
    name.chars().chain(std::iter::repeat('_')).take(36).collect()
}

/// Returns the library hash placeholder as `$hex(library_hash(name))$`
pub fn library_hash_placeholder(name: impl AsRef<[u8]>) -> String {
    let mut s = String::with_capacity(34 + 2);
    s.push('$');
    s.push_str(hex::Buffer::<17, false>::new().format(&library_hash(name)));
    s.push('$');
    s
}

/// Returns the library placeholder for the given name
/// The placeholder is a 34 character prefix of the hex encoding of the keccak256 hash of the fully
/// qualified library name.
///
/// See also <https://docs.soliditylang.org/en/develop/using-the-compiler.html#library-linking>
pub fn library_hash(name: impl AsRef<[u8]>) -> [u8; 17] {
    let hash = keccak256(name);
    hash[..17].try_into().unwrap()
}

/// Find the common ancestor, if any, between the given paths
///
/// # Examples
///
/// ```
/// use foundry_compilers_core::utils::common_ancestor_all;
/// use std::path::{Path, PathBuf};
///
/// let baz = Path::new("/foo/bar/baz");
/// let bar = Path::new("/foo/bar/bar");
/// let foo = Path::new("/foo/bar/foo");
/// let common = common_ancestor_all([baz, bar, foo]).unwrap();
/// assert_eq!(common, Path::new("/foo/bar").to_path_buf());
/// ```
pub fn common_ancestor_all<I, P>(paths: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut iter = paths.into_iter();
    let mut ret = iter.next()?.as_ref().to_path_buf();
    for path in iter {
        if let Some(r) = common_ancestor(&ret, path.as_ref()) {
            ret = r;
        } else {
            return None;
        }
    }
    Some(ret)
}

/// Finds the common ancestor of both paths
///
/// # Examples
///
/// ```
/// use foundry_compilers_core::utils::common_ancestor;
/// use std::path::{Path, PathBuf};
///
/// let foo = Path::new("/foo/bar/foo");
/// let bar = Path::new("/foo/bar/bar");
/// let ancestor = common_ancestor(foo, bar).unwrap();
/// assert_eq!(ancestor, Path::new("/foo/bar"));
/// ```
pub fn common_ancestor(a: &Path, b: &Path) -> Option<PathBuf> {
    let a = a.components();
    let b = b.components();
    let mut ret = PathBuf::new();
    let mut found = false;
    for (c1, c2) in a.zip(b) {
        if c1 == c2 {
            ret.push(c1);
            found = true;
        } else {
            break;
        }
    }
    if found {
        Some(ret)
    } else {
        None
    }
}

/// Returns the right subpath in a dir
///
/// Returns `<root>/<fave>` if it exists or `<root>/<alt>` does not exist,
/// Returns `<root>/<alt>` if it exists and `<root>/<fave>` does not exist.
pub fn find_fave_or_alt_path(root: &Path, fave: &str, alt: &str) -> PathBuf {
    let p = root.join(fave);
    if !p.exists() {
        let alt = root.join(alt);
        if alt.exists() {
            return alt;
        }
    }
    p
}

cfg_if! {
    if #[cfg(any(feature = "async", feature = "svm-solc"))] {
        use tokio::runtime::{Handle, Runtime};

        #[derive(Debug)]
        pub enum RuntimeOrHandle {
            Runtime(Runtime),
            Handle(Handle),
        }

        impl Default for RuntimeOrHandle {
            fn default() -> Self {
                Self::new()
            }
        }

        impl RuntimeOrHandle {
            pub fn new() -> Self {
                match Handle::try_current() {
                    Ok(handle) => Self::Handle(handle),
                    Err(_) => Self::Runtime(Runtime::new().expect("Failed to start runtime")),
                }
            }

            pub fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
                match &self {
                    Self::Runtime(runtime) => runtime.block_on(f),
                    Self::Handle(handle) => tokio::task::block_in_place(|| handle.block_on(f)),
                }
            }
        }
    }
}

/// Creates a new named tempdir.
#[cfg(any(test, feature = "project-util", feature = "test-utils"))]
pub fn tempdir(name: &str) -> Result<tempfile::TempDir, SolcIoError> {
    tempfile::Builder::new().prefix(name).tempdir().map_err(|err| SolcIoError::new(err, name))
}

/// Reads the json file and deserialize it into the provided type.
pub fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T, SolcError> {
    // See: https://github.com/serde-rs/json/issues/160
    let s = fs::read_to_string(path).map_err(|err| SolcError::io(err, path))?;
    serde_json::from_str(&s).map_err(Into::into)
}

/// Writes serializes the provided value to JSON and writes it to a file.
pub fn write_json_file<T: Serialize>(
    value: &T,
    path: &Path,
    capacity: usize,
) -> Result<(), SolcError> {
    let file = fs::File::create(path).map_err(|err| SolcError::io(err, path))?;
    let mut writer = std::io::BufWriter::with_capacity(capacity, file);
    serde_json::to_writer(&mut writer, value)?;
    writer.flush().map_err(|e| SolcError::io(e, path))
}

/// Creates the parent directory of the `file` and all its ancestors if it does not exist.
///
/// See [`fs::create_dir_all()`].
pub fn create_parent_dir_all(file: &Path) -> Result<(), SolcError> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            SolcError::msg(format!(
                "Failed to create artifact parent folder \"{}\": {}",
                parent.display(),
                err
            ))
        })?;
    }
    Ok(())
}

#[cfg(any(test, feature = "test-utils"))]
// <https://doc.rust-lang.org/rust-by-example/std_misc/fs.html>
pub fn touch(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(path) {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(any(test, feature = "test-utils"))]
pub fn mkdir_or_touch(tmp: &std::path::Path, paths: &[&str]) {
    for path in paths {
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(tmp.join(parent)).unwrap();
        }
        if path.ends_with(".sol") {
            let path = tmp.join(path);
            touch(&path).unwrap();
        } else {
            let path: PathBuf = tmp.join(path);
            std::fs::create_dir_all(path).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    pub use super::*;
    pub use std::fs::{create_dir_all, File};

    #[test]
    fn can_create_parent_dirs_with_ext() {
        let tmp_dir = tempdir("out").unwrap();
        let path = tmp_dir.path().join("IsolationModeMagic.sol/IsolationModeMagic.json");
        create_parent_dir_all(&path).unwrap();
        assert!(path.parent().unwrap().is_dir());
    }

    #[test]
    fn can_create_parent_dirs_versioned() {
        let tmp_dir = tempdir("out").unwrap();
        let path = tmp_dir.path().join("IVersioned.sol/IVersioned.0.8.16.json");
        create_parent_dir_all(&path).unwrap();
        assert!(path.parent().unwrap().is_dir());
        let path = tmp_dir.path().join("IVersioned.sol/IVersioned.json");
        create_parent_dir_all(&path).unwrap();
        assert!(path.parent().unwrap().is_dir());
    }

    #[test]
    fn can_determine_local_paths() {
        assert!(is_local_source_name(&[""], "./local/contract.sol"));
        assert!(is_local_source_name(&[""], "../local/contract.sol"));
        assert!(!is_local_source_name(&[""], "/ds-test/test.sol"));

        let tmp_dir = tempdir("contracts").unwrap();
        let dir = tmp_dir.path().join("ds-test");
        create_dir_all(&dir).unwrap();
        File::create(dir.join("test.sol")).unwrap();

        assert!(!is_local_source_name(&[tmp_dir.path()], "ds-test/test.sol"));
    }

    #[test]
    fn can_normalize_solidity_import_path() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path();

        // File structure:
        //
        // `dir_path`
        // └── src (`cwd`)
        //     ├── Token.sol
        //     └── common
        //         └── Burnable.sol

        fs::create_dir_all(dir_path.join("src/common")).unwrap();
        fs::write(dir_path.join("src/Token.sol"), "").unwrap();
        fs::write(dir_path.join("src/common/Burnable.sol"), "").unwrap();

        // assume that the import path is specified in Token.sol
        let cwd = dir_path.join("src");

        assert_eq!(
            normalize_solidity_import_path(&cwd, "./common/Burnable.sol".as_ref()).unwrap(),
            dir_path.join("src/common/Burnable.sol"),
        );

        assert!(normalize_solidity_import_path(&cwd, "./common/Pausable.sol".as_ref()).is_err());
    }

    // This test is exclusive to unix because creating a symlink is a privileged action on Windows.
    // https://doc.rust-lang.org/std/os/windows/fs/fn.symlink_dir.html#limitations
    #[test]
    #[cfg(unix)]
    fn can_normalize_solidity_import_path_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path();

        // File structure:
        //
        // `dir_path`
        // ├── dependency
        // │   └── Math.sol
        // └── project
        //     ├── node_modules
        //     │   └── dependency -> symlink to actual 'dependency' directory
        //     └── src (`cwd`)
        //         └── Token.sol

        fs::create_dir_all(dir_path.join("project/src")).unwrap();
        fs::write(dir_path.join("project/src/Token.sol"), "").unwrap();
        fs::create_dir(dir_path.join("project/node_modules")).unwrap();

        fs::create_dir(dir_path.join("dependency")).unwrap();
        fs::write(dir_path.join("dependency/Math.sol"), "").unwrap();

        std::os::unix::fs::symlink(
            dir_path.join("dependency"),
            dir_path.join("project/node_modules/dependency"),
        )
        .unwrap();

        // assume that the import path is specified in Token.sol
        let cwd = dir_path.join("project/src");

        assert_eq!(
            normalize_solidity_import_path(&cwd, "../node_modules/dependency/Math.sol".as_ref())
                .unwrap(),
            dir_path.join("project/node_modules/dependency/Math.sol"),
        );
    }

    #[test]
    fn can_clean_solidity_path() {
        let clean_solidity_path = |s: &str| clean_solidity_path(s.as_ref());
        assert_eq!(clean_solidity_path("a"), PathBuf::from("a"));
        assert_eq!(clean_solidity_path("./a"), PathBuf::from("a"));
        assert_eq!(clean_solidity_path("../a"), PathBuf::from("../a"));
        assert_eq!(clean_solidity_path("/a/"), PathBuf::from("/a"));
        assert_eq!(clean_solidity_path("//a"), PathBuf::from("/a"));
        assert_eq!(clean_solidity_path("a/b"), PathBuf::from("a/b"));
        assert_eq!(clean_solidity_path("a//b"), PathBuf::from("a/b"));
        assert_eq!(clean_solidity_path("/a/b"), PathBuf::from("/a/b"));
        assert_eq!(clean_solidity_path("a/./b"), PathBuf::from("a/b"));
        assert_eq!(clean_solidity_path("a/././b"), PathBuf::from("a/b"));
        assert_eq!(clean_solidity_path("/a/../b"), PathBuf::from("/b"));
        assert_eq!(clean_solidity_path("a/./../b/."), PathBuf::from("b"));
        assert_eq!(clean_solidity_path("a/b/c"), PathBuf::from("a/b/c"));
        assert_eq!(clean_solidity_path("a/b/../c"), PathBuf::from("a/c"));
        assert_eq!(clean_solidity_path("a/b/../../c"), PathBuf::from("c"));
        assert_eq!(clean_solidity_path("a/b/../../../c"), PathBuf::from("../c"));
        assert_eq!(
            clean_solidity_path("a/../b/../../c/./Token.sol"),
            PathBuf::from("../c/Token.sol")
        );
    }

    #[test]
    fn can_find_ancestor() {
        let a = Path::new("/foo/bar/bar/test.txt");
        let b = Path::new("/foo/bar/foo/example/constract.sol");
        let expected = Path::new("/foo/bar");
        assert_eq!(common_ancestor(a, b).unwrap(), expected.to_path_buf())
    }

    #[test]
    fn no_common_ancestor_path() {
        let a = Path::new("/foo/bar");
        let b = Path::new("./bar/foo");
        assert!(common_ancestor(a, b).is_none());
    }

    #[test]
    fn can_find_all_ancestor() {
        let a = Path::new("/foo/bar/foo/example.txt");
        let b = Path::new("/foo/bar/foo/test.txt");
        let c = Path::new("/foo/bar/bar/foo/bar");
        let expected = Path::new("/foo/bar");
        let paths = vec![a, b, c];
        assert_eq!(common_ancestor_all(paths).unwrap(), expected.to_path_buf())
    }
}
