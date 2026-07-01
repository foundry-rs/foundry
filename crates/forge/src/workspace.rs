//! Shared utilities for creating isolated project workspaces.
//!
//! Used by both mutation testing and brutalization to copy a project
//! to a temporary directory for safe source-level modifications.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use eyre::Result;
use foundry_compilers::artifacts::remappings::{RelativeRemapping, Remapping};
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

/// Build a config for a copied temp workspace from an already materialized config.
///
/// This preserves CLI/env overrides and runtime normalization while rebasing
/// project-local paths from the original root to `temp_path`.
pub fn rebase_config_paths(config: &Config, temp_path: &Path) -> Config {
    let mut temp_config = config.clone();
    temp_config.root = temp_path.to_path_buf();
    temp_config.src = rebase_project_path(&config.root, temp_path, &config.src);
    temp_config.test = rebase_project_path(&config.root, temp_path, &config.test);
    temp_config.script = rebase_project_path(&config.root, temp_path, &config.script);
    temp_config.out = rebase_project_path(&config.root, temp_path, &config.out);
    temp_config.cache_path = rebase_project_path(&config.root, temp_path, &config.cache_path);
    temp_config.snapshots = rebase_project_path(&config.root, temp_path, &config.snapshots);
    temp_config.broadcast = rebase_project_path(&config.root, temp_path, &config.broadcast);
    temp_config.mutation_dir = rebase_project_path(&config.root, temp_path, &config.mutation_dir);
    temp_config.test_failures_file =
        rebase_project_path(&config.root, temp_path, &config.test_failures_file);
    temp_config.build_info_path = config
        .build_info_path
        .as_ref()
        .map(|path| rebase_project_path(&config.root, temp_path, path));
    temp_config.libs =
        config.libs.iter().map(|lib| rebase_project_path(&config.root, temp_path, lib)).collect();
    temp_config.remappings = config
        .remappings
        .iter()
        .map(|remapping| rebase_remapping(&config.root, temp_path, remapping))
        .collect();
    temp_config.include_paths = config
        .include_paths
        .iter()
        .map(|path| rebase_project_path(&config.root, temp_path, path))
        .collect();
    temp_config.allow_paths = config
        .allow_paths
        .iter()
        .map(|path| rebase_project_path(&config.root, temp_path, path))
        .collect();
    temp_config.ignored_error_codes_from = config
        .ignored_error_codes_from
        .iter()
        .map(|(path, codes)| (rebase_project_path(&config.root, temp_path, path), codes.clone()))
        .collect();
    temp_config.ignored_file_paths = config
        .ignored_file_paths
        .iter()
        .map(|path| rebase_project_path(&config.root, temp_path, path))
        .collect();

    if let Some(path) = &config.fuzz.failure_persist_dir {
        temp_config.fuzz.failure_persist_dir =
            Some(rebase_project_path(&config.root, temp_path, path));
    }
    if let Some(path) = &config.invariant.failure_persist_dir {
        temp_config.invariant.failure_persist_dir =
            Some(rebase_project_path(&config.root, temp_path, path));
    }
    for permission in &mut temp_config.fs_permissions.permissions {
        permission.path = rebase_project_path(&config.root, temp_path, &permission.path);
    }
    if let Some(model_checker) = &mut temp_config.model_checker {
        model_checker.contracts = std::mem::take(&mut model_checker.contracts)
            .into_iter()
            .map(|(path, contracts)| {
                let path = rebase_project_path(&config.root, temp_path, Path::new(&path));
                (path.display().to_string(), contracts)
            })
            .collect();
    }

    temp_config
}

fn rebase_project_path(root: &Path, temp_path: &Path, path: &Path) -> PathBuf {
    let rel = relative_to_root(root, path);
    if rel.is_absolute() { path.to_path_buf() } else { temp_path.join(rel) }
}

fn rebase_remapping(
    root: &Path,
    temp_path: &Path,
    remapping: &RelativeRemapping,
) -> RelativeRemapping {
    let mut remapping: Remapping = remapping.clone().into();
    remapping.path =
        rebase_project_path(root, temp_path, Path::new(&remapping.path)).display().to_string();
    if let Some(context) = &mut remapping.context {
        *context = rebase_project_path(root, temp_path, Path::new(context)).display().to_string();
    }
    RelativeRemapping::new(remapping, temp_path)
}

/// Verify that `candidate` resolves (after following symlinks) to a path that lives
/// inside `allowed_root`. Protects against `src`/`test`/`lib`/etc. being symlinks
/// that escape the project root.
///
/// `label` and `orig` are only used for error messages.
fn ensure_within_root(
    allowed_root: &Path,
    candidate: &Path,
    label: &str,
    orig: &Path,
) -> Result<()> {
    // If the path doesn't exist yet, lexical containment is the best we can do.
    if !candidate.exists() {
        return Ok(());
    }
    let canon_root = allowed_root.canonicalize().map_err(|e| {
        eyre::eyre!("failed to canonicalize project root {}: {e}", allowed_root.display())
    })?;
    let canon_candidate = candidate.canonicalize().map_err(|e| {
        eyre::eyre!("failed to canonicalize {label} path {}: {e}", candidate.display())
    })?;
    if !canon_candidate.starts_with(&canon_root) {
        eyre::bail!(
            "{label} path {} escapes project root {} (resolved to {})",
            orig.display(),
            allowed_root.display(),
            canon_candidate.display()
        );
    }
    Ok(())
}

/// Copy essential project files to a temp workspace.
///
/// Copies src and test directories, symlinks library directories (read-only),
/// and copies config files (foundry.toml, remappings.txt).
pub fn copy_project(config: &Config, temp_dir: &Path) -> Result<()> {
    let src_rel = relative_to_root(&config.root, &config.src);
    ensure_safe_relative_path(&src_rel, "src", &config.src)?;
    ensure_within_root(&config.root, &config.src, "src", &config.src)?;

    let test_rel = relative_to_root(&config.root, &config.test);
    ensure_safe_relative_path(&test_rel, "test", &config.test)?;
    ensure_within_root(&config.root, &config.test, "test", &config.test)?;

    copy_dir_recursive(&config.src, &temp_dir.join(&src_rel))?;

    if config.test != config.src {
        copy_dir_recursive(&config.test, &temp_dir.join(&test_rel))?;
    }

    let handled_extra_roots = handled_project_roots(config)?;
    for extra_path in config.include_paths.iter().chain(config.allow_paths.iter()) {
        copy_extra_project_path(&config.root, temp_dir, extra_path, &handled_extra_roots)?;
    }

    // Copy `script/` too when present and distinct from src/test. Many real
    // projects keep helper contracts, deployment scripts, or fixtures under
    // `script/` and reference them from tests via relative imports. Without
    // this, baselines that compile fine produce a sea of `Invalid` mutants
    // for purely-environmental reasons.
    if config.script.exists() && config.script != config.src && config.script != config.test {
        let script_rel = relative_to_root(&config.root, &config.script);
        ensure_safe_relative_path(&script_rel, "script", &config.script)?;
        ensure_within_root(&config.root, &config.script, "script", &config.script)?;
        copy_dir_recursive(&config.script, &temp_dir.join(&script_rel))?;
    }

    for lib_path in &config.libs {
        if lib_path.exists() {
            let lib_rel = relative_to_root(&config.root, lib_path);
            ensure_safe_relative_path(&lib_rel, "lib", lib_path)?;
            ensure_within_root(&config.root, lib_path, "lib", lib_path)?;
            let target = temp_dir.join(&lib_rel);

            if !target.exists() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                if symlink_dir(lib_path, &target).is_err() {
                    copy_dir_recursive(lib_path, &target)?;
                }
            }

            symlink_nested_libs(lib_path, &target, 0)?;
        }
    }

    for dep_dir in ["node_modules", "dependencies"] {
        let dep_path = config.root.join(dep_dir);
        if dep_path.exists() && dep_path.is_dir() {
            // Reject if the project-root entry is a symlink that escapes the root.
            ensure_within_root(&config.root, &dep_path, dep_dir, &dep_path)?;
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

fn handled_project_roots(config: &Config) -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    push_handled_project_root(&mut roots, &config.root, &config.src, "src")?;
    push_handled_project_root(&mut roots, &config.root, &config.test, "test")?;

    if config.script.exists() && config.script != config.src && config.script != config.test {
        push_handled_project_root(&mut roots, &config.root, &config.script, "script")?;
    }

    for lib_path in &config.libs {
        if lib_path.exists() {
            push_handled_project_root(&mut roots, &config.root, lib_path, "lib")?;
        }
    }

    for dep_dir in ["node_modules", "dependencies"] {
        let dep_path = config.root.join(dep_dir);
        if dep_path.exists() && dep_path.is_dir() {
            roots.push(PathBuf::from(dep_dir));
        }
    }

    Ok(roots)
}

fn push_handled_project_root(
    roots: &mut Vec<PathBuf>,
    root: &Path,
    path: &Path,
    label: &str,
) -> Result<()> {
    let rel = relative_to_root(root, path);
    ensure_safe_relative_path(&rel, label, path)?;
    ensure_within_root(root, path, label, path)?;
    roots.push(rel);
    Ok(())
}

fn is_covered_by_handled_root(rel: &Path, handled_roots: &[PathBuf]) -> bool {
    handled_roots.iter().any(|root| !root.as_os_str().is_empty() && rel.starts_with(root))
}

fn copy_extra_project_path(
    root: &Path,
    temp_dir: &Path,
    path: &Path,
    handled_roots: &[PathBuf],
) -> Result<()> {
    let resolved = if path.is_absolute() { path.to_path_buf() } else { root.join(path) };
    let rel = relative_to_root(root, &resolved);
    ensure_safe_relative_path(&rel, "include/allow", path)?;
    ensure_within_root(root, &resolved, "include/allow", path)?;

    if is_covered_by_handled_root(&rel, handled_roots) {
        return Ok(());
    }

    if !resolved.exists() {
        return Ok(());
    }

    let target = temp_dir.join(rel);
    if resolved.is_dir() {
        copy_dir_recursive(&resolved, &target)
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&resolved, target)?;
        Ok(())
    }
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

/// Maximum recursion depth for nested lib symlinks to prevent infinite loops.
const MAX_SYMLINK_DEPTH: usize = 10;

/// Recursively symlink nested lib directories within a library.
fn symlink_nested_libs(lib_src: &Path, lib_dst: &Path, depth: usize) -> Result<()> {
    if depth >= MAX_SYMLINK_DEPTH {
        return Ok(());
    }

    let nested_lib_dirs: Vec<PathBuf> =
        if let Ok(config) = Config::load_with_root_and_fallback(lib_src) {
            config.libs
        } else {
            vec![PathBuf::from("lib")]
        };

    for nested_lib_dir in nested_lib_dirs {
        // A dependency's foundry.toml is untrusted input. Reject any nested lib
        // path that is absolute or contains `..`, then verify the resolved path
        // doesn't escape the dependency root via symlink.
        if !is_safe_relative_path(&nested_lib_dir) {
            continue;
        }
        let nested_lib = lib_src.join(&nested_lib_dir);
        if !nested_lib.exists() {
            continue;
        }
        // Use symlink_metadata so we don't follow a symlinked nested lib root.
        let Ok(meta) = fs::symlink_metadata(&nested_lib) else { continue };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        if ensure_within_root(lib_src, &nested_lib, "nested lib", &nested_lib).is_err() {
            continue;
        }
        process_nested_lib_dir(&nested_lib, lib_dst, &nested_lib_dir, depth)?;
    }

    Ok(())
}

fn process_nested_lib_dir(
    nested_lib: &Path,
    lib_dst: &Path,
    lib_rel: &Path,
    depth: usize,
) -> Result<()> {
    if !nested_lib.exists() || !nested_lib.is_dir() {
        return Ok(());
    }

    let entries = match fs::read_dir(nested_lib) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        // Use file_type() (does not follow symlinks) so a symlinked entry in a
        // dependency's lib dir cannot be silently followed and re-symlinked
        // outside the workspace.
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }

        let entry_path = entry.path();
        let entry_name = entry.file_name();
        let nested_dst = lib_dst.join(lib_rel).join(&entry_name);

        if !nested_dst.exists() {
            if let Some(parent) = nested_dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = symlink_dir(&entry_path, &nested_dst);
        }

        symlink_nested_libs(&entry_path, &nested_dst, depth + 1)?;
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
    use std::{collections::BTreeMap, str::FromStr};

    use foundry_compilers::artifacts::ModelCheckerSettings;
    use foundry_config::fs_permissions::PathPermission;
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
    fn test_rebase_config_paths_rebases_materialized_project_paths() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("project");
        let workspace = temp.path().join("workspace");
        let external = temp.path().join("external");

        let mut contracts = BTreeMap::new();
        contracts.insert(root.join("src/Target.sol").display().to_string(), vec!["Target".into()]);
        contracts
            .insert(external.join("External.sol").display().to_string(), vec!["External".into()]);

        let config = Config {
            root: root.clone(),
            src: root.join("contracts"),
            test: root.join("checks"),
            script: root.join("deploy"),
            out: root.join("custom-out"),
            cache_path: root.join("custom-cache"),
            snapshots: root.join("custom-snapshots"),
            broadcast: root.join("custom-broadcast"),
            mutation_dir: root.join("custom-cache/mutation"),
            test_failures_file: root.join("custom-cache/test-failures"),
            build_info_path: Some(root.join("custom-build-info")),
            libs: vec![root.join("vendor"), external.join("lib")],
            include_paths: vec![root.join("shared")],
            allow_paths: vec![root.join("fixtures"), external.join("fixtures")],
            ignored_error_codes_from: vec![
                (
                    root.join("contracts"),
                    vec![foundry_config::SolidityErrorCode::UnusedLocalVariable],
                ),
                (
                    external.join("contracts"),
                    vec![foundry_config::SolidityErrorCode::UnusedFunctionParameter],
                ),
            ],
            ignored_file_paths: vec![
                root.join("contracts/Ignored.sol"),
                external.join("Ignored.sol"),
            ],
            remappings: vec![
                Remapping::from_str(&format!("@src/={}/", root.join("src").display()))
                    .unwrap()
                    .into(),
                Remapping::from_str(&format!("@ext/={}/", external.join("src").display()))
                    .unwrap()
                    .into(),
            ],
            fs_permissions: foundry_config::FsPermissions::new([
                PathPermission::read(root.join("fixtures")),
                PathPermission::read(external.join("fixtures")),
            ]),
            model_checker: Some(ModelCheckerSettings {
                contracts,
                engine: None,
                timeout: None,
                targets: None,
                invariants: None,
                show_unproved: None,
                div_mod_with_slacks: None,
                solvers: None,
                show_unsupported: None,
                show_proved_safe: None,
            }),
            ..Default::default()
        };

        let temp_config = rebase_config_paths(&config, &workspace);

        assert_eq!(temp_config.root, workspace);
        assert_eq!(temp_config.src, workspace.join("contracts"));
        assert_eq!(temp_config.test, workspace.join("checks"));
        assert_eq!(temp_config.script, workspace.join("deploy"));
        assert_eq!(temp_config.out, workspace.join("custom-out"));
        assert_eq!(temp_config.cache_path, workspace.join("custom-cache"));
        assert_eq!(temp_config.snapshots, workspace.join("custom-snapshots"));
        assert_eq!(temp_config.broadcast, workspace.join("custom-broadcast"));
        assert_eq!(temp_config.mutation_dir, workspace.join("custom-cache/mutation"));
        assert_eq!(temp_config.test_failures_file, workspace.join("custom-cache/test-failures"));
        assert_eq!(temp_config.build_info_path, Some(workspace.join("custom-build-info")));
        assert_eq!(temp_config.libs, vec![workspace.join("vendor"), external.join("lib")]);
        assert_eq!(temp_config.include_paths, vec![workspace.join("shared")]);
        assert_eq!(
            temp_config.allow_paths,
            vec![workspace.join("fixtures"), external.join("fixtures")]
        );
        assert_eq!(
            temp_config.ignored_error_codes_from,
            vec![
                (
                    workspace.join("contracts"),
                    vec![foundry_config::SolidityErrorCode::UnusedLocalVariable]
                ),
                (
                    external.join("contracts"),
                    vec![foundry_config::SolidityErrorCode::UnusedFunctionParameter]
                )
            ]
        );
        assert_eq!(
            temp_config.ignored_file_paths,
            vec![workspace.join("contracts/Ignored.sol"), external.join("Ignored.sol")]
        );

        let remappings =
            temp_config.remappings.into_iter().map(Remapping::from).collect::<Vec<_>>();
        assert_eq!(remappings[0].path, format!("{}/", workspace.join("src").display()));
        assert_eq!(remappings[1].path, format!("{}/", external.join("src").display()));

        assert_eq!(temp_config.fs_permissions.permissions[0].path, workspace.join("fixtures"));
        assert_eq!(temp_config.fs_permissions.permissions[1].path, external.join("fixtures"));

        let contracts = temp_config.model_checker.unwrap().contracts;
        assert!(contracts.contains_key(&workspace.join("src/Target.sol").display().to_string()));
        assert!(contracts.contains_key(&external.join("External.sol").display().to_string()));
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

        symlink_nested_libs(&lib_src, &lib_dst, 0).unwrap();

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

        symlink_nested_libs(&lib_src, &lib_dst, 0).unwrap();

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

        symlink_nested_libs(&lib_src, &lib_dst, 0).unwrap();

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

        symlink_nested_libs(&lib_src, &lib_dst, 0).unwrap();

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
    fn test_copy_project_copies_include_paths_under_root() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("project");
        let out = temp.path().join("workspace");
        create_test_dir_structure(
            &root,
            &["src/Counter.sol", "test/Counter.t.sol", "include/Shared.sol"],
        );

        let config = Config {
            root: root.clone(),
            src: root.join("src"),
            test: root.join("test"),
            script: root.join("script"),
            include_paths: vec![root.join("include")],
            ..Default::default()
        };

        copy_project(&config, &out).unwrap();

        assert!(out.join("include/Shared.sol").exists());
    }

    #[test]
    fn test_copy_project_skips_include_paths_covered_by_libs() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("project");
        let out = temp.path().join("workspace");
        create_test_dir_structure(
            &root,
            &["src/Counter.sol", "test/Counter.t.sol", "lib/foo/Foo.sol", "lib/bar/Bar.sol"],
        );

        let config = Config {
            root: root.clone(),
            src: root.join("src"),
            test: root.join("test"),
            script: root.join("script"),
            libs: vec![root.join("lib")],
            include_paths: vec![root.join("lib/foo")],
            ..Default::default()
        };

        copy_project(&config, &out).unwrap();

        assert!(out.join("lib/foo/Foo.sol").exists());
        assert!(out.join("lib/bar/Bar.sol").exists());
    }

    #[test]
    fn test_copy_project_rejects_external_include_paths() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("project");
        let outside = temp.path().join("outside");
        let out = temp.path().join("workspace");
        create_test_dir_structure(&root, &["src/Counter.sol", "test/Counter.t.sol"]);
        create_test_dir_structure(&outside, &["Shared.sol"]);

        let config = Config {
            root: root.clone(),
            src: root.join("src"),
            test: root.join("test"),
            script: root.join("script"),
            include_paths: vec![outside],
            ..Default::default()
        };

        let err = copy_project(&config, &out).unwrap_err();

        assert!(
            err.to_string().contains("requires include/allow directory under project root"),
            "unexpected error: {err}"
        );
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

    #[test]
    fn test_ensure_within_root_rejects_symlink_escape() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("project");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "shhh").unwrap();

        // src is a symlink that points outside the project root.
        let src = root.join("src");
        symlink_dir(&outside, &src).unwrap();

        let err = ensure_within_root(&root, &src, "src", &src).unwrap_err();
        assert!(err.to_string().contains("escapes project root"), "unexpected error: {err}");
    }

    #[test]
    fn test_ensure_within_root_accepts_in_root_symlink() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("project");
        let real_src = root.join("real_src");
        fs::create_dir_all(&real_src).unwrap();

        // src -> real_src is fine: stays inside the project root.
        let src_link = root.join("src");
        symlink_dir(&real_src, &src_link).unwrap();

        ensure_within_root(&root, &src_link, "src", &src_link).unwrap();
    }

    #[test]
    fn test_symlink_nested_libs_rejects_traversal_in_dependency_config() {
        let temp = TempDir::new().unwrap();

        // Pretend lib_src is a malicious dependency whose foundry.toml says
        // libs = ["../../escape"]. We can't easily write foundry.toml here, so
        // exercise the lexical guard directly via is_safe_relative_path: any
        // path containing `..` must be rejected before being joined with
        // `lib_src`.
        let malicious: PathBuf = PathBuf::from("../../escape");
        assert!(!is_safe_relative_path(&malicious));

        // Sanity check: a benign relative path is still accepted.
        let benign: PathBuf = PathBuf::from("lib");
        assert!(is_safe_relative_path(&benign));

        // And the function returns Ok when there is nothing to do.
        let lib_src = temp.path().join("lib_src");
        let lib_dst = temp.path().join("lib_dst");
        fs::create_dir_all(&lib_src).unwrap();
        fs::create_dir_all(&lib_dst).unwrap();
        symlink_nested_libs(&lib_src, &lib_dst, 0).unwrap();
    }

    #[test]
    fn test_process_nested_lib_dir_skips_symlinks() {
        let temp = TempDir::new().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir_all(outside.join("secret_pkg/src")).unwrap();
        fs::write(outside.join("secret_pkg/src/Secret.sol"), "secret").unwrap();

        let lib_src = temp.path().join("lib_src");
        let nested = lib_src.join("lib");
        fs::create_dir_all(&nested).unwrap();
        // A dep that is a symlink pointing outside the lib root.
        symlink_dir(&outside.join("secret_pkg"), &nested.join("evil")).unwrap();

        let lib_dst = temp.path().join("lib_dst");
        fs::create_dir_all(&lib_dst).unwrap();

        process_nested_lib_dir(&nested, &lib_dst, Path::new("lib"), 0).unwrap();

        // The symlinked entry must not have been followed into the destination.
        assert!(!lib_dst.join("lib/evil").exists(), "symlinked dep was followed");
    }
}
