use crate::Lockfile;
use clap::{Parser, ValueHint};
use comfy_table::{
    Cell, Color, Row, Table, modifiers::UTF8_ROUND_CORNERS, presets::ASCII_MARKDOWN,
};
use eyre::Result;
use foundry_cli::utils::{Git, LoadConfig};
use foundry_common::shell;
use foundry_config::{Config, impl_figment_convert_basic};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// CLI arguments for `forge dependencies`.
#[derive(Clone, Debug, Parser)]
pub struct DependenciesArgs {
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,
}

impl_figment_convert_basic!(DependenciesArgs);

/// A single installed dependency, either a Git submodule or a Soldeer package.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyInfo {
    /// The dependency name (submodule directory name, or Soldeer package name).
    pub name: String,
    /// The dependency manager that installed it.
    pub source: &'static str,
    /// A human readable version/revision identifier.
    pub version: String,
    /// Path to the dependency, relative to the project root.
    pub path: String,
    /// The upstream URL, if known.
    pub url: Option<String>,
}

impl DependenciesArgs {
    pub fn run(self) -> Result<()> {
        let config = self.load_config()?;

        let mut dependencies = submodule_dependencies(&config)?;
        dependencies.extend(soldeer_dependencies(&config)?);
        dependencies.sort_by(|a, b| a.name.cmp(&b.name));

        if shell::is_json() {
            sh_println!("{}", serde_json::to_string_pretty(&dependencies)?)?;
            return Ok(());
        }

        if dependencies.is_empty() {
            sh_println!("No dependencies found")?;
            return Ok(());
        }

        let mut table = Table::new();
        if shell::is_markdown() {
            table.load_preset(ASCII_MARKDOWN);
        } else {
            table.apply_modifier(UTF8_ROUND_CORNERS);
        }
        table.set_header(vec![
            Cell::new("Name").fg(Color::Cyan),
            Cell::new("Source").fg(Color::Cyan),
            Cell::new("Version").fg(Color::Cyan),
            Cell::new("Path").fg(Color::Cyan),
        ]);
        for dep in &dependencies {
            let mut row = Row::new();
            row.add_cell(Cell::new(&dep.name));
            row.add_cell(Cell::new(dep.source));
            row.add_cell(Cell::new(&dep.version));
            row.add_cell(Cell::new(&dep.path));
            table.add_row(row);
        }
        sh_println!("{table}")?;

        Ok(())
    }
}

/// Lists Git submodule dependencies under the project's install lib dir.
///
/// `foundry.lock` (see [`Lockfile`]) is consulted, when present, to report the pinned tag/branch
/// instead of just the raw revision the submodule is currently checked out at. This never writes
/// to `foundry.lock` - listing is read-only.
fn submodule_dependencies(config: &Config) -> Result<Vec<DependencyInfo>> {
    let git = Git::from_config(config);
    let Ok(git_root) = Git::root_of(&config.root) else {
        // Not a Git repository - no submodules possible.
        return Ok(Vec::new());
    };
    // Unlike the not-a-repo case above, a failure here is a real problem worth surfacing loudly:
    // `git submodule status` fails its *entire* output (not just the offending line) if any
    // submodule anywhere in the repo is merge-conflicted (`U<all-zeros> path`) - `Submodule`'s
    // status-line regex only recognizes ` `/`+`/`-` prefixes, not `U`. Silently falling back to
    // an empty list here would print "No dependencies found" while the repo is mid-conflict,
    // which is exactly the kind of "looks empty but isn't" result this command exists to avoid.
    // Trade-off worth knowing: this means one conflicted submodule anywhere in a large repo
    // blocks `forge dependencies` entirely, even for an unrelated project subdirectory - fixing
    // that needs `Submodule`'s status parsing to tolerate individual bad lines, which is shared
    // by every other caller of `Git::submodules()` and out of scope here.
    let submodules = git.submodules()?;

    let mut lockfile = Lockfile::new(&config.root);
    if lockfile.exists() {
        lockfile.read()?;
    }

    // `config.root` isn't guaranteed to be the canonical spelling of the project root - on
    // macOS, `/tmp/project` (what a user or test harness typically passes) and
    // `/private/tmp/project` (what `Config::canonic_at` resolves `install_lib_dir` and friends
    // to) name the same directory but compare unequal as strings. Canonicalize before any
    // `strip_prefix`/path-arithmetic, matching `Lockfile::check`'s own handling of this.
    let project_root = dunce::canonicalize(&config.root).unwrap_or_else(|_| config.root.clone());

    // `Git`'s commands all run with `config.root` as their working directory (see
    // `Git::from_config`), and `git submodule status` prints paths relative to cwd - so
    // `submodule.path()` below is already relative to the project root, nested-monorepo layout
    // or not. No rebasing needed for the filter, the `foundry.lock` lookup (its keys are written
    // the same way, by a `Git` instance rooted the same way - see `install.rs`/`update.rs`), or
    // the displayed path.
    let install_lib_dir = config.install_lib_dir();
    let lib = install_lib_dir.strip_prefix(&project_root).unwrap_or(install_lib_dir);

    // `.gitmodules` always keys its `path` field relative to the Git repository root, regardless
    // of invocation cwd - so this is the one place that still needs a Git-root-relative path.
    let project_prefix = project_root.strip_prefix(&git_root).unwrap_or(Path::new(""));

    let mut out = Vec::new();
    for submodule in &submodules {
        let path = submodule.path();
        if !path.starts_with(lib) {
            continue;
        }
        // `git submodule status`'s leading `-`/`+` status marker (which would say "not
        // initialized") is stripped away by `Submodule::from_str`'s parsing, so a registered but
        // never-checked-out submodule - e.g. after a `git clone` without `--recursive`, or a
        // manual `rm -rf lib/foo` without `git submodule deinit` - still shows up here with its
        // last-known rev. Match git's own "is this submodule populated" check
        // (`<path>/.git` existing) rather than a bare directory-existence check, since a
        // never-initialized gitlink still leaves behind an empty directory.
        if !project_root.join(path).join(".git").exists() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };

        // A `.gitmodules` section name isn't required to match its `path` field (e.g. `git
        // submodule add --name openzeppelin <url> lib/openzeppelin`), and `git config
        // submodule.<key>.url` is keyed by the section name - resolve it by path instead of
        // assuming the two are the same string.
        let url = git.submodule_url_for_path(&git_root, &project_prefix.join(path)).ok().flatten();

        // Prefer `foundry.lock`'s pinned tag/branch, but only when it still matches what's
        // actually checked out - if the submodule was manually moved to a different commit
        // (`git submodule status`'s `+` marker), showing the stale locked version would
        // misrepresent what's really on disk.
        let version = lockfile
            .get(path)
            // `git submodule status` always reports the full 40-char SHA, but a `DepIdentifier::Rev`
            // entry (from `forge install dep@<rev>`) stores whatever string the user typed
            // verbatim, abbreviated or not - `Tag`/`Branch` entries are already normalized to a
            // full SHA via `git.get_rev()`, so a bare `==` silently rejects an accurate,
            // still-matching abbreviated `Rev` pin. A short hash is only ever the checkout's own
            // SHA prefixed with itself, never another commit's, so `starts_with` is exact for
            // full-length revs and correct for abbreviated ones.
            .filter(|dep| !dep.rev().is_empty() && submodule.rev().starts_with(dep.rev()))
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("rev={}", submodule.rev()));

        out.push(DependencyInfo {
            name: name.to_string(),
            source: "submodule",
            version,
            path: path.display().to_string(),
            url,
        });
    }

    Ok(out)
}

/// Lists Soldeer-managed dependencies recorded in `soldeer.lock`.
fn soldeer_dependencies(config: &Config) -> Result<Vec<DependencyInfo>> {
    let soldeer_lock_path = config.root.join(soldeer_core::lock::SOLDEER_LOCK);
    if !soldeer_lock_path.exists() {
        return Ok(Vec::new());
    }

    // `read_lockfile` returns `Ok` with empty entries for malformed files, consistent with how
    // `forge build`'s Soldeer integrity check already treats it.
    let Ok(lockfile) = soldeer_core::lock::read_lockfile(&soldeer_lock_path) else {
        return Ok(Vec::new());
    };

    let deps_dir = config.root.join("dependencies");

    let mut out = Vec::new();
    for entry in &lockfile.entries {
        let install_path = entry.install_path(&deps_dir);
        // A fresh clone (before `forge soldeer install`) or a manually deleted package still has
        // a `soldeer.lock` entry - only report what's actually present on disk.
        if !install_path.exists() {
            continue;
        }
        let path = install_path.strip_prefix(&config.root).unwrap_or(&install_path);

        let url = match entry {
            soldeer_core::lock::LockEntry::Git(dep) => Some(dep.git.clone()),
            soldeer_core::lock::LockEntry::Http(dep) => Some(dep.url.clone()),
            _ => None,
        };

        out.push(DependencyInfo {
            name: entry.name().to_string(),
            source: "soldeer",
            version: entry.version().to_string(),
            path: path.display().to_string(),
            url,
        });
    }

    Ok(out)
}
