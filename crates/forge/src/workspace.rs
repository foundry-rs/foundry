//! Shared utilities for creating isolated project workspaces.
//!
//! Used by both mutation testing and brutalization to copy a project
//! to a temporary directory for safe source-level modifications.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use eyre::Result;
use foundry_config::Config;

/// Check if a path is safe for use as a relative path within a workspace.
/// Rejects absolute paths, parent directory components (..), and other unsafe patterns.
pub fn is_safe_relative_path(p: &Path) -> bool {
    !p.is_absolute()
        && p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Validates that `rel` is a safe relative path. Returns an error mentioning `label` and `orig`
/// if the path contains `..`, is absolute, or otherwise escapes the project root.
pub fn ensure_safe_relative_path(rel: &Path, label: &str, orig: &Path) -> Result<()> {
    if !is_safe_relative_path(rel) {
        eyre::bail!("requires {label} directory under project root, got: {}", orig.display());
    }
    Ok(())
}

/// Compute relative path of `path` under `root`, or return the path unchanged if not under root.
pub fn relative_to_root(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).map(|p| p.to_path_buf()).unwrap_or_else(|_| path.to_path_buf())
}

/// Copy essential project files to a temp workspace.
///
/// Copies src and test directories, symlinks library directories (read-only),
/// and copies config files (foundry.toml, remappings.txt).
pub fn copy_project(config: &Config, temp_dir: &Path) -> Result<()> {
    let src_rel = relative_to_root(&config.root, &config.src);
    ensure_safe_relative_path(&src_rel, "src", &config.src)?;

    let test_rel = relative_to_root(&config.root, &config.test);
    ensure_safe_relative_path(&test_rel, "test", &config.test)?;

    copy_dir_recursive(&config.src, &temp_dir.join(&src_rel))?;

    if config.test != config.src {
        copy_dir_recursive(&config.test, &temp_dir.join(&test_rel))?;
    }

    for lib_path in &config.libs {
        if lib_path.exists() {
            let lib_rel = relative_to_root(&config.root, lib_path);
            ensure_safe_relative_path(&lib_rel, "lib", lib_path)?;
            let target = temp_dir.join(&lib_rel);

            if !target.exists() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                if symlink_dir(lib_path, &target).is_err() {
                    copy_dir_recursive(lib_path, &target)?;
                }
            }

            symlink_nested_libs(lib_path, &target)?;
        }
    }

    for dep_dir in ["node_modules", "dependencies"] {
        let dep_path = config.root.join(dep_dir);
        if dep_path.exists() && dep_path.is_dir() {
            let target = temp_dir.join(dep_dir);
            if !target.exists() && symlink_dir(&dep_path, &target).is_err() {
                copy_dir_recursive(&dep_path, &target)?;
            }
        }
    }

    let foundry_toml = config.root.join("foundry.toml");
    if foundry_toml.exists() {
        fs::copy(&foundry_toml, temp_dir.join("foundry.toml"))?;
    }

    let remappings = config.root.join("remappings.txt");
    if remappings.exists() {
        fs::copy(&remappings, temp_dir.join("remappings.txt"))?;
    }

    Ok(())
}

/// Create a symlink to a directory (cross-platform).
pub fn symlink_dir(src: &Path, dst: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst)?;
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(src, dst)?;
    }
    Ok(())
}

/// Recursively symlink nested lib directories within a library.
fn symlink_nested_libs(lib_src: &Path, lib_dst: &Path) -> Result<()> {
    let nested_lib_dirs: Vec<PathBuf> =
        if let Ok(config) = Config::load_with_root_and_fallback(lib_src) {
            config.libs
        } else {
            vec![PathBuf::from("lib")]
        };

    for nested_lib_dir in nested_lib_dirs {
        let nested_lib = lib_src.join(&nested_lib_dir);
        if !nested_lib.exists() || !nested_lib.is_dir() {
            continue;
        }
        process_nested_lib_dir(&nested_lib, lib_dst, &nested_lib_dir)?;
    }

    Ok(())
}

fn process_nested_lib_dir(nested_lib: &Path, lib_dst: &Path, lib_rel: &Path) -> Result<()> {
    if !nested_lib.exists() || !nested_lib.is_dir() {
        return Ok(());
    }

    let entries = match fs::read_dir(nested_lib) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        let entry_name = entry.file_name();
        let nested_dst = lib_dst.join(lib_rel).join(&entry_name);

        if !nested_dst.exists() {
            if let Some(parent) = nested_dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = symlink_dir(&entry_path, &nested_dst);
        }

        symlink_nested_libs(&entry_path, &nested_dst)?;
    }

    Ok(())
}

/// Recursively copy a directory, skipping symlinked directories for safety.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }

    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());

        let meta = fs::symlink_metadata(&path)?;

        if meta.file_type().is_symlink() {
            if path.is_dir() {
                continue;
            }
            fs::copy(&path, &dest_path)?;
        } else if meta.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_dir_structure(base: &Path, structure: &[&str]) {
        for path in structure {
            let full_path = base.join(path);
            if path.ends_with('/') {
                fs::create_dir_all(&full_path).unwrap();
            } else {
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&full_path, format!("// {path}")).unwrap();
            }
        }
    }

    #[test]
    fn test_symlink_dir_creates_symlink() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("source_dir");
        let dst = temp.path().join("target_link");

        fs::create_dir(&src).unwrap();
        fs::write(src.join("file.txt"), "content").unwrap();

        symlink_dir(&src, &dst).unwrap();

        assert!(dst.exists());
        assert!(dst.is_symlink());
        assert!(dst.join("file.txt").exists());
    }

    #[test]
    fn test_symlink_nested_libs_single_level() {
        let temp = TempDir::new().unwrap();

        let lib_src = temp.path().join("lib_src");
        create_test_dir_structure(
            &lib_src,
            &[
                "src/Contract.sol",
                "lib/",
                "lib/openzeppelin/contracts/token/ERC20.sol",
                "lib/solmate/src/tokens/ERC20.sol",
            ],
        );

        let lib_dst = temp.path().join("lib_dst");
        fs::create_dir(&lib_dst).unwrap();

        symlink_nested_libs(&lib_src, &lib_dst).unwrap();

        assert!(lib_dst.join("lib/openzeppelin").exists());
        assert!(lib_dst.join("lib/solmate").exists());
        assert!(lib_dst.join("lib/openzeppelin/contracts/token/ERC20.sol").exists());
        assert!(lib_dst.join("lib/solmate/src/tokens/ERC20.sol").exists());
    }

    #[test]
    fn test_symlink_nested_libs_deeply_nested() {
        let temp = TempDir::new().unwrap();

        let lib_src = temp.path().join("lib_src");
        create_test_dir_structure(
            &lib_src,
            &[
                "src/Main.sol",
                "lib/",
                "lib/dep-a/src/A.sol",
                "lib/dep-a/lib/",
                "lib/dep-a/lib/dep-b/src/B.sol",
                "lib/dep-a/lib/dep-b/lib/",
                "lib/dep-a/lib/dep-b/lib/dep-c/src/C.sol",
            ],
        );

        let lib_dst = temp.path().join("lib_dst");
        fs::create_dir(&lib_dst).unwrap();

        symlink_nested_libs(&lib_src, &lib_dst).unwrap();

        assert!(lib_dst.join("lib/dep-a").exists());
        assert!(lib_dst.join("lib/dep-a/lib/dep-b").exists());
        assert!(lib_dst.join("lib/dep-a/lib/dep-b/lib/dep-c").exists());
        assert!(lib_dst.join("lib/dep-a/lib/dep-b/lib/dep-c/src/C.sol").exists());
    }

    #[test]
    fn test_symlink_nested_libs_no_nested_lib_dir() {
        let temp = TempDir::new().unwrap();

        let lib_src = temp.path().join("lib_src");
        create_test_dir_structure(&lib_src, &["src/Contract.sol", "test/Test.sol"]);

        let lib_dst = temp.path().join("lib_dst");
        fs::create_dir(&lib_dst).unwrap();

        symlink_nested_libs(&lib_src, &lib_dst).unwrap();

        assert!(!lib_dst.join("lib").exists());
    }

    #[test]
    fn test_symlink_nested_libs_skips_existing() {
        let temp = TempDir::new().unwrap();

        let lib_src = temp.path().join("lib_src");
        create_test_dir_structure(&lib_src, &["lib/", "lib/existing/src/File.sol"]);

        let lib_dst = temp.path().join("lib_dst");
        fs::create_dir_all(lib_dst.join("lib/existing")).unwrap();
        fs::write(lib_dst.join("lib/existing/marker.txt"), "pre-existing").unwrap();

        symlink_nested_libs(&lib_src, &lib_dst).unwrap();

        assert!(lib_dst.join("lib/existing/marker.txt").exists());
    }

    #[test]
    fn test_copy_dir_recursive_basic() {
        let temp = TempDir::new().unwrap();

        let src = temp.path().join("src");
        create_test_dir_structure(
            &src,
            &["file1.sol", "subdir/file2.sol", "subdir/nested/file3.sol"],
        );

        let dst = temp.path().join("dst");
        copy_dir_recursive(&src, &dst).unwrap();

        assert!(dst.join("file1.sol").exists());
        assert!(dst.join("subdir/file2.sol").exists());
        assert!(dst.join("subdir/nested/file3.sol").exists());
    }

    #[test]
    fn test_copy_dir_recursive_skips_symlinked_dirs() {
        let temp = TempDir::new().unwrap();

        let src = temp.path().join("src");
        let external = temp.path().join("external");

        fs::create_dir_all(&external).unwrap();
        fs::write(external.join("secret.txt"), "should not be copied").unwrap();

        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("file.sol"), "content").unwrap();

        symlink_dir(&external, &src.join("external_link")).unwrap();

        let dst = temp.path().join("dst");
        copy_dir_recursive(&src, &dst).unwrap();

        assert!(dst.join("file.sol").exists());
        assert!(!dst.join("external_link").exists());
    }

    #[test]
    fn test_copy_dir_recursive_nonexistent_src() {
        let temp = TempDir::new().unwrap();

        let src = temp.path().join("nonexistent");
        let dst = temp.path().join("dst");

        copy_dir_recursive(&src, &dst).unwrap();
        assert!(!dst.exists());
    }

    #[test]
    fn test_relative_to_root_basic() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/src/contracts");

        let rel = relative_to_root(&root, &path);
        assert_eq!(rel, PathBuf::from("src/contracts"));
    }

    #[test]
    fn test_relative_to_root_same_path() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project");

        let rel = relative_to_root(&root, &path);
        assert_eq!(rel, PathBuf::from(""));
    }

    #[test]
    fn test_relative_to_root_outside_root() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/other/location");

        let rel = relative_to_root(&root, &path);
        assert_eq!(rel, path);
    }
}
