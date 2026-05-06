use crate::{
    hir_ext, render,
    utils::{Deployment, git_source_url, read_deployments},
    vocs,
};
use eyre::Result;
use foundry_compilers::{compilers::solc::SOLC_EXTENSIONS, utils::source_files_iter};
use foundry_config::{DocConfig, filter::expand_globs};
use rayon::prelude::*;
use solar::sema::Compiler;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
};

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
    /// Optional commit hash (HEAD) used when building Git Source links.
    pub commit: Option<String>,
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
    pub fn build(self, compiler: &mut Compiler) -> Result<()> {
        let out = self.out_dir();
        let pages_dir = out.join("src").join("pages");

        let ignored = expand_globs(&self.root, self.config.ignore.iter()).unwrap_or_default();

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

        let repo = self.config.repository.clone();
        let commit = self.commit.clone();
        let deployments_cfg = self.deployments.clone();
        let root = self.root.clone();

        let all_pages = compiler.enter_mut(|compiler| -> eyre::Result<Vec<PathBuf>> {
            if compiler.gcx().stage() < Some(solar::config::CompilerStage::Lowering) {
                let _ = compiler.lower_asts();
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

                    let rel_path =
                        abs_path.strip_prefix(&root).unwrap_or(path.as_path()).to_owned();

                    // Git source link (skipped on library files).
                    let git_url = if *from_library {
                        None
                    } else {
                        repo.as_deref().and_then(|r| {
                            git_source_url(
                                r,
                                commit.as_deref().unwrap_or("master"),
                                &root,
                                &abs_path,
                            )
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

            Ok(all_rel)
        })?;

        // Generate vocs site scaffolding.
        vocs::write_site_files(&out, &self.config, &all_pages, &self.root)?;
        info!("wrote vocs site files to {}", out.display());

        Ok(())
    }
}
