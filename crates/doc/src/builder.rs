use crate::{
    hir_ext, render,
    utils::{git_source_url, is_safe_output_path, read_deployments, relative_source_path},
    vocs,
};
use eyre::Result;
use foundry_compilers::{compilers::solc::SOLC_EXTENSIONS, utils::source_files_iter};
use foundry_config::{
    DocConfig,
    filter::{expand_globs, is_ignored_path},
};
use rayon::prelude::*;
use solar::{config::CompilerStage, sema::Compiler};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Summary stats produced by [`DocBuilder::build`], surfaced to the user as
/// progress feedback by `forge doc`.
#[derive(Debug, Default, Clone)]
pub struct BuildStats {
    /// Number of Solidity sources considered for rendering.
    pub sources: usize,
    /// Number of MDX pages written to disk.
    pub pages: usize,
    /// Total time spent generating MDX pages and pruning stale ones.
    pub render_elapsed: Duration,
    /// Total time spent writing the vocs site scaffold.
    pub site_elapsed: Duration,
}

/// Build Solidity documentation for a project from natspec comments using [`solar`].
#[derive(Debug)]
pub struct DocBuilder {
    /// Project root.
    pub root: PathBuf,
    /// Path to Solidity source files.
    pub sources: PathBuf,
    /// Paths to external libraries.
    pub libraries: Vec<PathBuf>,
    /// Whether to also document files coming from external libraries.
    pub include_libraries: bool,
    /// Optional Git commit, tag, or branch used when building Git Source links.
    pub commit: Option<String>,
    /// Optional current branch name; used as the `<branch>` segment of vocs
    /// editLink URLs (which require an actual branch, not a commit/`HEAD`).
    pub branch: Option<String>,
    /// Optional path to the deployments directory (relative to `root`).
    /// `Some(None)` enables the preprocessor with the default `deployments`
    /// path; `Some(Some(p))` overrides; `None` disables it entirely.
    pub deployments: Option<Option<PathBuf>>,
    /// Documentation configuration.
    pub config: DocConfig,
}

impl DocBuilder {
    /// Create a new builder.
    pub fn new(
        root: PathBuf,
        sources: PathBuf,
        libraries: Vec<PathBuf>,
        include_libraries: bool,
    ) -> Self {
        Self {
            root,
            sources,
            libraries,
            include_libraries,
            commit: None,
            branch: None,
            deployments: None,
            config: DocConfig::default(),
        }
    }

    /// Resolve the absolute output directory.
    fn out_dir(&self) -> PathBuf {
        if self.config.out.is_absolute() {
            self.config.out.clone()
        } else {
            self.root.join(&self.config.out)
        }
    }

    /// Run the documentation pipeline.
    pub fn build(self, compiler: &mut Compiler) -> Result<BuildStats> {
        let out = self.out_dir();
        let pages_dir = out.join("src").join("pages");
        let render_started = Instant::now();

        let ignored = expand_globs(&self.root, self.config.ignore.iter()).unwrap_or_else(|e| {
            warn!("doc.ignore: failed to expand globs: {e}");
            Default::default()
        });

        let mut sources: Vec<(PathBuf, bool)> = source_files_iter(&self.sources, SOLC_EXTENSIONS)
            .filter(|p| !is_ignored_path(p, &ignored, &self.root))
            .map(|p| (p, false))
            .collect();

        if self.include_libraries {
            for lib_dir in &self.libraries {
                let lib_sources = source_files_iter(lib_dir, SOLC_EXTENSIONS)
                    .filter(|p| !is_ignored_path(p, &ignored, &self.root))
                    .map(|p| (p, true));
                sources.extend(lib_sources);
            }
        }

        sources.sort_by(|(a, _), (b, _)| a.cmp(b));
        let sources_count = sources.len();

        let repo = self.config.repository.as_deref();
        let commit = self.commit.as_deref();
        let deployments_cfg = &self.deployments;
        let root = &self.root;

        let results = compiler.enter_mut(|compiler| -> Result<Vec<RenderResult>> {
            if compiler.gcx().stage() < Some(CompilerStage::Lowering)
                && compiler.lower_asts().is_err()
            {
                // Diagnostics are already emitted via the solar session.
                eyre::bail!("forge doc: HIR lowering failed; see diagnostics above");
            }

            let gcx = compiler.gcx();

            // Restrict cross-reference resolution to files we'll actually emit pages for.
            let allowed_sources: HashSet<PathBuf> = sources
                .iter()
                .map(|(p, _)| if p.is_absolute() { p.clone() } else { root.join(p) })
                .collect();

            let name_to_page = hir_ext::build_name_to_page(gcx, root, &allowed_sources);

            // Render each source in parallel.
            // Record panicked user sources so we can fail after writing successful pages.
            let results: Vec<RenderResult> = sources
                .par_iter()
                .map(|(path, from_library)| -> RenderResult {
                    let abs_path = if path.is_absolute() { path.clone() } else { root.join(path) };

                    let Some((_, ast_source)) = gcx.get_ast_source(&abs_path) else {
                        if !from_library {
                            warn!("AST source not found for {}", abs_path.display());
                        }
                        return Ok(Vec::new());
                    };
                    let Some(ast) = &ast_source.ast else {
                        if !from_library {
                            warn!("AST missing for {}", abs_path.display());
                        }
                        return Ok(Vec::new());
                    };

                    // For sources outside the project root (e.g. library deps that live under a
                    // different prefix), synthesise a safe relative path so that
                    // `pages_dir.join(rel_out_path)` can never escape the docs tree.
                    let rel_path = relative_source_path(root, &abs_path);

                    // Git source link (skipped on library files).
                    let git_url = if *from_library {
                        None
                    } else {
                        repo.and_then(|r| {
                            git_source_url(r, commit.unwrap_or("HEAD"), root, &abs_path)
                        })
                    };

                    // Deployments for this source's contracts.
                    // The legacy lookup is per file stem, so this source has one deployment list.
                    let deployments = deployments_cfg.as_ref().map_or_else(Vec::new, |dir| {
                        read_deployments(root, dir.as_deref(), &rel_path)
                    });

                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        render::source(
                            ast,
                            &ast_source.file,
                            &rel_path,
                            &abs_path,
                            gcx,
                            &name_to_page,
                            git_url.as_deref(),
                            &deployments,
                        )
                    }));
                    match result {
                        Ok(pages) => Ok(pages),
                        Err(_) => {
                            // Ignore failures from library files; surface user errors.
                            if *from_library {
                                debug!("rendering failed for library file {}", abs_path.display());
                                Ok(Vec::new())
                            } else {
                                error!("rendering panicked for {}", abs_path.display());
                                Err(abs_path)
                            }
                        }
                    }
                })
                .collect();

            Ok(results)
        })?;
        let all_pages = write_pages(results, &pages_dir)?;
        let render_elapsed = render_started.elapsed();

        // Generate vocs site scaffolding.
        let site_started = Instant::now();
        vocs::write_site_files(
            &out,
            &self.config,
            &all_pages,
            &self.root,
            &self.sources,
            self.branch.as_deref(),
            self.commit.as_deref(),
        )?;
        info!("wrote vocs site files to {}", out.display());
        let site_elapsed = site_started.elapsed();

        Ok(BuildStats {
            sources: sources_count,
            pages: all_pages.len(),
            render_elapsed,
            site_elapsed,
        })
    }
}

/// Successful pages or the path of a user source whose renderer panicked.
type RenderResult = std::result::Result<Vec<(PathBuf, String)>, PathBuf>;

/// Write successful pages, but do not change ownership or prune if a user source failed.
fn write_pages(results: Vec<RenderResult>, pages_dir: &Path) -> Result<Vec<PathBuf>> {
    // Split rendered pages from panicked user sources.
    let mut failed: Vec<PathBuf> = Vec::new();
    let mut all_rel: Vec<PathBuf> = Vec::new();
    for result in results {
        let page_list = match result {
            Ok(pages) => pages,
            Err(path) => {
                failed.push(path);
                continue;
            }
        };
        for (rel_out_path, content) in page_list {
            // Reject any output path that would escape the docs tree.
            if !is_safe_output_path(&rel_out_path) {
                warn!("skipping unsafe output path: {}", rel_out_path.display());
                continue;
            }
            let abs_out = pages_dir.join(&rel_out_path);
            if let Some(parent) = abs_out.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&abs_out, content)?;
            info!("wrote {}", abs_out.display());
            all_rel.push(rel_out_path);
        }
    }
    all_rel.sort();

    // Fail the build if any non-library source panicked during render.
    if !failed.is_empty() {
        let list =
            failed.iter().map(|p| format!("  - {}", p.display())).collect::<Vec<_>>().join("\n");
        eyre::bail!("forge doc: rendering panicked for {} source file(s):\n{list}", failed.len());
    }
    update_manifest(pages_dir, &all_rel)?;
    Ok(all_rel)
}

/// The manifest is the ownership boundary: only previously generated pages can be pruned.
fn update_manifest(pages_dir: &Path, all_rel: &[PathBuf]) -> Result<()> {
    // Prune stale `.mdx` pages using a manifest of previously generated
    // files. This covers every generated subtree (including library pages
    // outside `src/`), while never touching user-authored pages that were
    // never listed in the manifest.
    let manifest_path = pages_dir.join(".forge-doc-manifest");
    // Missing or unreadable manifests grant no ownership over existing pages.
    let prev_generated = fs::read_to_string(&manifest_path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect::<HashSet<_>>();
    let new_generated: HashSet<PathBuf> = all_rel.iter().cloned().collect();
    for stale in prev_generated.difference(&new_generated) {
        if !is_safe_output_path(stale) {
            warn!("forge doc: ignoring unsafe manifest entry '{}'", stale.display());
            continue;
        }
        let stale_abs = pages_dir.join(stale);
        if stale_abs.is_file() {
            debug!("pruning stale page {}", stale_abs.display());
            let _ = fs::remove_file(&stale_abs);
        }
    }
    // Write new manifest.
    {
        let mut manifest_lines: Vec<String> =
            all_rel.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        manifest_lines.sort();
        fs::create_dir_all(pages_dir)?;
        fs::write(&manifest_path, manifest_lines.join("\n") + "\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_failure_keeps_previous_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let pages = dir.path();
        let manifest = pages.join(".forge-doc-manifest");
        fs::write(&manifest, "old.mdx\n").unwrap();
        fs::write(pages.join("old.mdx"), "old").unwrap();
        let results = vec![
            Ok(vec![(PathBuf::from("new.mdx"), "new".to_string())]),
            Err(PathBuf::from("Broken.sol")),
            Ok(vec![(PathBuf::from("after.mdx"), "after".to_string())]),
        ];

        let error = write_pages(results, pages).unwrap_err();
        assert_eq!(
            error.to_string(),
            "forge doc: rendering panicked for 1 source file(s):\n  - Broken.sol"
        );
        assert_eq!(fs::read_to_string(pages.join("new.mdx")).unwrap(), "new");
        assert_eq!(fs::read_to_string(pages.join("after.mdx")).unwrap(), "after");
        assert_eq!(fs::read_to_string(pages.join("old.mdx")).unwrap(), "old");
        assert_eq!(fs::read_to_string(manifest).unwrap(), "old.mdx\n");
    }
}
