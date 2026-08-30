use crate::{
    hir_ext, render,
    utils::{Deployment, git_source_url, read_deployments},
    vocs,
};
use eyre::Result;
use foundry_compilers::{compilers::solc::SOLC_EXTENSIONS, utils::source_files_iter};
use foundry_config::{DocConfig, filter::expand_globs};
use rayon::prelude::*;
use solar::{config::CompilerStage, sema::Compiler};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, PathBuf},
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
            .filter(|p| !ignored.contains(p) && !ignored.contains(&self.root.join(p)))
            .map(|p| (p, false))
            .collect();

        if self.include_libraries {
            for lib_dir in &self.libraries {
                let lib_sources = source_files_iter(lib_dir, SOLC_EXTENSIONS)
                    .filter(|p| !ignored.contains(p) && !ignored.contains(&self.root.join(p)))
                    .map(|p| (p, true));
                sources.extend(lib_sources);
            }
        }

        sources.sort_by(|(a, _), (b, _)| a.cmp(b));
        let sources_count = sources.len();

        let repo = self.config.repository.clone();
        let commit = self.commit.clone();
        let deployments_cfg = self.deployments.clone();
        let root = self.root.clone();

        let all_pages = compiler.enter_mut(|compiler| -> eyre::Result<Vec<PathBuf>> {
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

            let name_to_page = hir_ext::build_name_to_page(gcx, &root, &allowed_sources);

            // Render each source in parallel.
            // Each entry is `(rendered_pages, panicked_user_source)`. A panicked
            // non-library source is recorded so we can fail the build at the end.
            type RenderResult = (Option<Vec<(PathBuf, String)>>, Option<PathBuf>);
            let results: Vec<RenderResult> = sources
                .par_iter()
                .map(|(path, from_library)| -> RenderResult {
                    let abs_path = if path.is_absolute() { path.clone() } else { root.join(path) };

                    let Some((_, ast_source)) = gcx.get_ast_source(&abs_path) else {
                        if !from_library {
                            warn!("AST source not found for {}", abs_path.display());
                        }
                        return (None, None);
                    };
                    let Some(ast) = &ast_source.ast else {
                        if !from_library {
                            warn!("AST missing for {}", abs_path.display());
                        }
                        return (None, None);
                    };

                    // For sources outside the project root (e.g. library deps that live under a
                    // different prefix), synthesise a safe relative path so that
                    // `pages_dir.join(rel_out_path)` can never escape the docs tree.
                    let rel_path = if let Ok(p) = abs_path.strip_prefix(&root) {
                        p.to_path_buf()
                    } else {
                        let comps: Vec<_> = abs_path.components().collect();
                        let start = comps.len().saturating_sub(3);
                        let tail: PathBuf = comps[start..].iter().collect();
                        PathBuf::from("lib").join(tail)
                    };

                    // Git source link (skipped on library files).
                    let git_url = if *from_library {
                        None
                    } else {
                        repo.as_deref().and_then(|r| {
                            git_source_url(r, commit.as_deref().unwrap_or("HEAD"), &root, &abs_path)
                        })
                    };

                    // Deployments for this source's contracts.
                    let deployments_map: HashMap<String, Vec<Deployment>> = match &deployments_cfg {
                        Some(dir_opt) => {
                            let entries = read_deployments(&root, dir_opt.as_deref(), &rel_path);
                            // All deployments belong to the contract sharing the
                            // file stem (legacy behaviour).
                            if entries.is_empty() {
                                HashMap::new()
                            } else if let Some(stem) = rel_path.file_stem().and_then(|s| s.to_str())
                            {
                                let mut m = HashMap::new();
                                m.insert(stem.to_string(), entries);
                                m
                            } else {
                                HashMap::new()
                            }
                        }
                        None => HashMap::new(),
                    };

                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        render::source(
                            ast,
                            &ast_source.file,
                            gcx.sess.source_map(),
                            &rel_path,
                            &abs_path,
                            &root,
                            gcx,
                            &name_to_page,
                            git_url.as_deref(),
                            &deployments_map,
                        )
                    }));
                    match result {
                        Ok(pages) => (Some(pages), None),
                        Err(_) => {
                            // Ignore failures from library files; surface user errors.
                            if *from_library {
                                debug!("rendering failed for library file {}", abs_path.display());
                                (None, None)
                            } else {
                                error!("rendering panicked for {}", abs_path.display());
                                (None, Some(abs_path))
                            }
                        }
                    }
                })
                .collect();

            // Split rendered pages from panicked user sources.
            let mut failed: Vec<PathBuf> = Vec::new();
            let mut all_rel: Vec<PathBuf> = Vec::new();
            for (pages, panicked) in results {
                if let Some(p) = panicked {
                    failed.push(p);
                }
                if let Some(page_list) = pages {
                    for (rel_out_path, content) in page_list {
                        // Reject any output path that would escape the docs tree.
                        if rel_out_path.is_absolute()
                            || rel_out_path.components().any(|c| {
                                matches!(
                                    c,
                                    Component::ParentDir
                                        | Component::RootDir
                                        | Component::Prefix(_)
                                )
                            })
                        {
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
            }
            all_rel.sort();

            // Fail the build if any non-library source panicked during render.
            if !failed.is_empty() {
                let list = failed
                    .iter()
                    .map(|p| format!("  - {}", p.display()))
                    .collect::<Vec<_>>()
                    .join("\n");
                eyre::bail!(
                    "forge doc: rendering panicked for {} source file(s):\n{list}",
                    failed.len()
                );
            }

            // Prune stale `.mdx` pages using a manifest of previously generated
            // files. This covers every generated subtree (including library pages
            // outside `src/`), while never touching user-authored pages that were
            // never listed in the manifest.
            let manifest_path = pages_dir.join(".forge-doc-manifest");
            let prev_generated: HashSet<PathBuf> = if manifest_path.exists() {
                fs::read_to_string(&manifest_path)
                    .unwrap_or_default()
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(PathBuf::from)
                    .collect()
            } else {
                // No manifest: do not prune. The manifest is the ownership boundary for
                // generated pages; without it, user-authored pages are indistinguishable.
                HashSet::new()
            };
            let new_generated: HashSet<PathBuf> = all_rel.iter().cloned().collect();
            for stale in prev_generated.difference(&new_generated) {
                let safe = !stale.is_absolute()
                    && !stale.components().any(|c| {
                        matches!(
                            c,
                            Component::ParentDir | Component::Prefix(_) | Component::RootDir
                        )
                    });
                if !safe {
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
                fs::write(&manifest_path, manifest_lines.join("\n") + "\n")?;
            }

            Ok(all_rel)
        })?;
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
