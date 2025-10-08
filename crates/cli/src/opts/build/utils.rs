use eyre::Result;
use foundry_compilers::{
    CompilerInput, Graph, Project, ProjectCompileOutput, ProjectPathsConfig,
    artifacts::{Source, Sources},
    multi::{MultiCompilerLanguage, MultiCompilerParser},
    solc::{SOLC_EXTENSIONS, SolcLanguage, SolcVersionedInput},
};
use foundry_config::{Config, semver::Version};
use rayon::prelude::*;
use solar::sema::ParsingContext;
use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
};

/// Configures a [`ParsingContext`] from [`Config`].
///
/// - Configures include paths, remappings
/// - Source files are added if `add_source_file` is set
/// - If no `project` is provided, it will spin up a new ephemeral project.
/// - If no `target_paths` are provided, all project files are processed.
/// - Only processes the subset of sources with the most up-to-date Solidity version.
pub fn configure_pcx(
    pcx: &mut ParsingContext<'_>,
    config: &Config,
    project: Option<&Project>,
    target_paths: Option<&[PathBuf]>,
) -> Result<()> {
    // Process build options
    let project = match project {
        Some(project) => project,
        None => &config.ephemeral_project()?,
    };

    let sources = match target_paths {
        // If target files are provided, only process those sources
        Some(targets) => {
            let mut sources = Sources::new();
            for t in targets {
                let path = dunce::canonicalize(t)?;
                let source = Source::read(&path)?;
                sources.insert(path, source);
            }
            sources
        }
        // Otherwise, process all project files
        None => project.paths.read_input_files()?,
    };

    // Only process sources with latest Solidity version to avoid conflicts.
    let graph = Graph::<MultiCompilerParser>::resolve_sources(&project.paths, sources)?;
    let (version, sources, _) = graph
        // Resolve graph into mapping language -> version -> sources
        .into_sources_by_version(project)?
        .sources
        .into_iter()
        // Only interested in Solidity sources
        .find(|(lang, _)| *lang == MultiCompilerLanguage::Solc(SolcLanguage::Solidity))
        .ok_or_else(|| eyre::eyre!("no Solidity sources"))?
        .1
        .into_iter()
        // Always pick the latest version
        .max_by(|(v1, _, _), (v2, _, _)| v1.cmp(v2))
        .unwrap();

    let solc = SolcVersionedInput::build(
        sources,
        config.solc_settings()?,
        SolcLanguage::Solidity,
        version,
    );

    configure_pcx_from_solc(pcx, &project.paths, &solc, true);

    Ok(())
}

/// Configures a [`ParsingContext`] from a [`ProjectCompileOutput`] and [`SolcVersionedInput`].
///
/// # Note:
/// uses `output.graph().source_files()` and `output.artifact_ids()` rather than `output.sources()`
/// because sources aren't populated when build is skipped when there are no changes in the source
/// code. <https://github.com/foundry-rs/foundry/issues/12018>
pub fn configure_pcx_from_compile_output(
    pcx: &mut ParsingContext<'_>,
    config: &Config,
    output: &ProjectCompileOutput,
    target_paths: Option<&[PathBuf]>,
) -> Result<()> {
    let is_solidity_file = |path: &Path| -> bool {
        path.extension().and_then(|s| s.to_str()).is_some_and(|ext| SOLC_EXTENSIONS.contains(&ext))
    };

    // Collect source path targets
    let mut source_paths: HashSet<PathBuf> = if let Some(targets) = target_paths
        && !targets.is_empty()
    {
        let mut source_paths = HashSet::new();
        let mut queue: VecDeque<PathBuf> = targets
            .iter()
            .filter_map(|path| {
                is_solidity_file(path).then(|| dunce::canonicalize(path).ok()).flatten()
            })
            .collect();

        while let Some(path) = queue.pop_front() {
            if source_paths.insert(path.clone()) {
                for import in output.graph().imports(path.as_path()) {
                    queue.push_back(import.to_path_buf());
                }
            }
        }

        source_paths
    } else {
        output
            .graph()
            .source_files()
            .filter_map(|idx| {
                let path = output.graph().node_path(idx).to_path_buf();
                is_solidity_file(&path).then_some(path)
            })
            .collect()
    };

    // Read all sources and find the latest version.
    let (version, sources) = {
        let (mut max_version, mut sources) = (Version::new(0, 0, 0), Sources::new());
        for (id, _) in output.artifact_ids() {
            if let Ok(path) = dunce::canonicalize(&id.source)
                && source_paths.remove(&path)
            {
                if id.version > max_version {
                    max_version = id.version;
                };

                let source = Source::read(&path)?;
                sources.insert(path, source);
            }
        }

        (max_version, sources)
    };

    let solc = SolcVersionedInput::build(
        sources,
        config.solc_settings()?,
        SolcLanguage::Solidity,
        version,
    );

    configure_pcx_from_solc(pcx, &config.project_paths(), &solc, true);
    Ok(())
}

/// Configures a [`ParsingContext`] from [`ProjectPathsConfig`] and [`SolcVersionedInput`].
///
/// - Configures include paths, remappings.
/// - Source files are added if `add_source_file` is set
pub fn configure_pcx_from_solc(
    pcx: &mut ParsingContext<'_>,
    project_paths: &ProjectPathsConfig,
    vinput: &SolcVersionedInput,
    add_source_files: bool,
) {
    configure_pcx_from_solc_cli(pcx, project_paths, &vinput.cli_settings);
    if add_source_files {
        let sources = vinput
            .input
            .sources
            .par_iter()
            .filter_map(|(path, source)| {
                pcx.sess.source_map().new_source_file(path.clone(), source.content.as_str()).ok()
            })
            .collect::<Vec<_>>();
        pcx.add_files(sources);
    }
}

fn configure_pcx_from_solc_cli(
    pcx: &mut ParsingContext<'_>,
    project_paths: &ProjectPathsConfig,
    cli_settings: &foundry_compilers::solc::CliSettings,
) {
    pcx.file_resolver
        .set_current_dir(cli_settings.base_path.as_ref().unwrap_or(&project_paths.root));
    for remapping in &project_paths.remappings {
        pcx.file_resolver.add_import_remapping(solar::sema::interface::config::ImportRemapping {
            context: remapping.context.clone().unwrap_or_default(),
            prefix: remapping.name.clone(),
            path: remapping.path.clone(),
        });
    }
    pcx.file_resolver.add_include_paths(cli_settings.include_paths.iter().cloned());
}
