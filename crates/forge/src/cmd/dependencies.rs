use crate::Lockfile;
use clap::{Parser, ValueHint};
use comfy_table::{
    Cell, Color, Row, Table, modifiers::UTF8_ROUND_CORNERS, presets::ASCII_MARKDOWN,
};
use eyre::{Context, Result};
use foundry_cli::utils::{Git, LoadConfig, SubmoduleCheckoutStatus};
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
        // Long URLs would otherwise stretch every row to the widest one.
        table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
        table.set_header(vec![
            Cell::new("Name").fg(Color::Cyan),
            Cell::new("Source").fg(Color::Cyan),
            Cell::new("Version").fg(Color::Cyan),
            Cell::new("Path").fg(Color::Cyan),
            Cell::new("URL").fg(Color::Cyan),
        ]);
        for dep in &dependencies {
            let mut row = Row::new();
            row.add_cell(Cell::new(&dep.name));
            row.add_cell(Cell::new(dep.source));
            row.add_cell(Cell::new(&dep.version));
            row.add_cell(Cell::new(&dep.path));
            row.add_cell(Cell::new(dep.url.as_deref().unwrap_or("-")));
            table.add_row(row);
        }
        sh_println!("{table}")?;

        Ok(())
    }
}

/// Strips HTTP(S) userinfo (`user:pass@` / `token@`) from a dependency URL before it's ever
/// displayed or serialized.
///
/// `forge install`/`git config submodule.<name>.url` both accept credentials embedded directly in
/// the URL (`https://ghp_xxx@github.com/org/private-repo.git`, common for a private-repo token or
/// CI credential), and Soldeer Git/HTTP URLs can carry the same. Verified empirically: a submodule
/// whose effective local-config URL embeds a token was printed verbatim by `forge dependencies
/// --json` before this redaction. Leaves the URL untouched if it doesn't parse as an absolute URL
/// (e.g. SSH's `git@host:path` shorthand, which never round-trips through [`url::Url`] and has no
/// parseable userinfo component to strip) or if it has no credentials to begin with.
fn redact_url_credentials(url: String) -> String {
    let Ok(mut parsed) = url::Url::parse(&url) else { return url };
    if parsed.username().is_empty() && parsed.password().is_none() {
        return url;
    }
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.to_string()
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
    // `Git::from_config`), so `lib`, scoped as a pathspec below, is relative to the project root,
    // nested-monorepo layout or not.
    let install_lib_dir = config.install_lib_dir();
    let lib = install_lib_dir.strip_prefix(&project_root).unwrap_or(install_lib_dir);

    // `libs` explicitly supports absolute paths, and a configured lib dir can be (or contain) a
    // symlink resolving outside the Git repository entirely. `git ls-files` then exits 128 ("is
    // outside repository") - not a per-line parse failure like the other cases in this function,
    // but a structural one, and letting it propagate would fail the *whole* command, including
    // the unrelated Soldeer half. Verified empirically: `libs = ["lib"]` with `lib` symlinked to
    // a directory outside the repo reproduces exactly this exit-128 failure. Treat it the same as
    // "not a Git repository" - there simply are no Git submodules to report from here.
    let git_root_canonical = dunce::canonicalize(&git_root).unwrap_or_else(|_| git_root.clone());
    if let Ok(lib_absolute) = dunce::canonicalize(project_root.join(lib))
        && !lib_absolute.starts_with(&git_root_canonical)
    {
        return Ok(Vec::new());
    }

    // `.gitmodules` always keys its `path` field relative to the Git repository root, regardless
    // of invocation cwd - so this is the one place that still needs a Git-root-relative path.
    let project_prefix = project_root.strip_prefix(&git_root).unwrap_or(Path::new(""));

    // Parsed once and reused for every submodule below, instead of re-parsing `.gitmodules` (and
    // spawning a fresh `git config` subprocess) per submodule.
    let gitmodules_entries = git.submodule_gitmodules_entries(&git_root).unwrap_or_default();

    // NUL-delimited (`git ls-files --stage -z` under the hood) and status-tolerant per entry -
    // unlike `Git::submodules()`'s whitespace-split `git submodule status` parser, this handles a
    // submodule path containing a space correctly (that parser's path capture group is `[^\s]+`
    // and rejects the whole line), and classifies each submodule's checkout status individually
    // instead of failing the *entire* listing the moment one line looks unusual - a
    // merge-conflicted submodule used to take down the whole command this way; scoping directly
    // to `lib` here also replaces the old post-hoc `path.starts_with(lib)` filter.
    let submodules = git.submodules_in_worktree(lib, &git_root, project_prefix)?;

    let mut out = Vec::new();
    for submodule in &submodules {
        // A registered but never-checked-out submodule - e.g. after a `git clone` without
        // `--recursive`, or a manual `rm -rf lib/foo` without `git submodule deinit` - isn't
        // actually installed.
        //
        // A `MissingMapping` gitlink (present in the index, but with no corresponding
        // `.gitmodules` entry - e.g. `.gitmodules` was hand-edited or deleted without also
        // running `git submodule deinit`) can never be initialized: `git submodule update --init`
        // has nothing to clone from without a mapping. `submodules_in_worktree` reports it
        // *without* consulting the real on-disk checkout at all - its "rev" is always the index's
        // recorded sha, never the actual checkout - so on a fresh clone (where the directory is
        // genuinely empty) this used to list a phantom "installed" dependency with a fabricated
        // rev. Verified empirically: a fresh clone of a repo with a mappingless gitlink shows an
        // empty `lib/<name>` directory, yet without this check `forge dependencies` still listed
        // it as installed.
        if matches!(
            submodule.status(),
            SubmoduleCheckoutStatus::Uninitialized | SubmoduleCheckoutStatus::MissingMapping
        ) {
            continue;
        }

        let path = submodule.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };

        // A `.gitmodules` section name isn't required to match its `path` field (e.g. `git
        // submodule add --name openzeppelin <url> lib/openzeppelin`), and `git config
        // submodule.<key>.url` is keyed by the section name - resolve it by path instead of
        // assuming the two are the same string.
        let url = git
            .submodule_url_for_path(&gitmodules_entries, &project_prefix.join(path))
            .ok()
            .flatten()
            .map(redact_url_credentials);

        // A merge-conflicted submodule has no meaningful single revision to report (`git
        // submodule status` prints an all-zero placeholder SHA for it) - say so plainly rather
        // than showing that placeholder or a stale lockfile pin.
        let version = if submodule.status() == SubmoduleCheckoutStatus::Conflicted {
            "conflicted".to_string()
        } else {
            // Prefer `foundry.lock`'s pinned tag/branch, but only when it still matches what's
            // actually checked out - if the submodule was manually moved to a different commit,
            // showing the stale locked version would misrepresent what's really on disk.
            lockfile
                .get(path)
                // `git submodule status` always reports the full 40-char SHA, but a
                // `DepIdentifier::Rev` entry (from `forge install dep@<rev>`) stores whatever
                // string the user typed verbatim, abbreviated or not - `Tag`/`Branch` entries are
                // already normalized to a full SHA via `git.get_rev()`, so a bare `==` silently
                // rejects an accurate, still-matching abbreviated `Rev` pin. A short hash is only
                // ever the checkout's own SHA prefixed with itself, never another commit's, so
                // `starts_with` is exact for full-length revs and correct for abbreviated ones.
                .filter(|dep| !dep.rev().is_empty() && submodule.rev().starts_with(dep.rev()))
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("rev={}", submodule.rev()))
        };

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

    // `read_lockfile` already converts malformed TOML to `Ok` with empty entries internally
    // (`toml_edit::de::from_str(..).unwrap_or_default()`, logging a warning) - the only way it
    // returns `Err` is a genuine I/O failure reading the path (`fs::read_to_string`), e.g.
    // `soldeer.lock` being unreadable or, notably, being a *directory* rather than a file.
    // Swallowing that here made an unreadable lockfile silently look identical to "zero Soldeer
    // dependencies" instead of surfacing a real problem - propagate it with path context instead.
    let lockfile = soldeer_core::lock::read_lockfile(&soldeer_lock_path)
        .wrap_err_with(|| format!("failed to read {}", soldeer_lock_path.display()))?;

    let deps_dir = config.root.join("dependencies");

    let mut out = Vec::new();
    for entry in &lockfile.entries {
        let install_path = entry.install_path(&deps_dir);
        // A fresh clone (before `forge soldeer install`) or a manually deleted package still has
        // a `soldeer.lock` entry - only report what's actually present on disk. For a Git-backed
        // entry specifically, mere existence isn't enough: Soldeer's own integrity check
        // (`check_git_dependency`) classifies an empty or non-Git directory at the install path
        // as `Missing`, not installed - verified empirically that a bare empty directory at a
        // Git entry's install path was otherwise listed here as if it were a real checkout.
        // Requiring `.git` metadata is a lighter approximation of that same check (it doesn't
        // also verify the checked-out commit matches the locked `rev`, which needs Soldeer's own
        // async git plumbing), but it closes the reported false-positive.
        let looks_installed = match entry {
            soldeer_core::lock::LockEntry::Git(_) => install_path.join(".git").exists(),
            _ => install_path.exists(),
        };
        if !looks_installed {
            continue;
        }
        let path = install_path.strip_prefix(&config.root).unwrap_or(&install_path);

        // For a Git-backed entry, `version()` is only the *requirement* the dependency was
        // installed against (e.g. a tag/branch spec) - the actually resolved, checked-out commit
        // is `rev`, a separate field. A later `forge soldeer update` can move `rev` to a new
        // commit while leaving the requirement string unchanged, so `version` alone doesn't
        // identify what's actually on disk; append the locked rev, matching the `<label>=<value>@
        // <rev>` convention `DepIdentifier`'s `Display` impl already uses for submodule pins.
        let (url, version) = match entry {
            soldeer_core::lock::LockEntry::Git(dep) => {
                (Some(dep.git.clone()), format!("{}@{}", entry.version(), dep.rev))
            }
            soldeer_core::lock::LockEntry::Http(dep) => {
                (Some(dep.url.clone()), entry.version().to_string())
            }
            _ => (None, entry.version().to_string()),
        };

        out.push(DependencyInfo {
            name: entry.name().to_string(),
            source: "soldeer",
            version,
            path: path.display().to_string(),
            url: url.map(redact_url_credentials),
        });
    }

    Ok(out)
}
