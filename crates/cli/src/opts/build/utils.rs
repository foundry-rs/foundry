use eyre::Result;
use foundry_compilers::{
    CompilerInput, Graph, Project, ProjectCompileOutput, ProjectPathsConfig,
    artifacts::{Source, Sources},
    multi::{MultiCompilerLanguage, MultiCompilerParsedSource, MultiCompilerParser},
    solc::{SolcLanguage, SolcVersionedInput},
};
use foundry_config::{Config, semver::Version};
use rayon::prelude::*;
use solar::sema::ParsingContext;
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
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

/// Configures a [`ParsingContext`] from a [`Project`] and [`SolcVersionedInput`].
///
/// - Configures include paths, remappings.
/// - Source files are added if `add_source_file` is set
pub fn configure_pcx_from_compile_output(
    pcx: &mut ParsingContext<'_>,
    config: &Config,
    output: &ProjectCompileOutput,
    target_paths: Option<&[PathBuf]>,
) -> Result<()> {
    // If targets are specified, find the max version among those files and their dependencies.
    let (version, source_paths): (Version, Vec<PathBuf>) = if let Some(targets) = target_paths {
        let mut scope = HashSet::new();
        let mut queue: VecDeque<PathBuf> = targets
            .iter()
            .filter_map(|path| {
                if let Ok(full_path) = dunce::canonicalize(path)
                    && output
                        .graph()
                        .get_parsed_source(full_path.as_path())
                        .is_some_and(|ps| matches!(ps, MultiCompilerParsedSource::Solc(..)))
                {
                    Some(full_path)
                } else {
                    None
                }
            })
            .collect();

        while let Some(path) = queue.pop_front() {
            if scope.insert(path.to_path_buf()) {
                for import in output.graph().imports(path.as_path()) {
                    queue.push_back(import.to_path_buf());
                }
            }
        }

        let version = output
            .output()
            .sources
            .sources_with_version()
            .filter_map(|(p, _, v)| {
                if let Ok(full_path) = dunce::canonicalize(p)
                    && scope.contains(&full_path)
                {
                    Some(v)
                } else {
                    None
                }
            })
            .min()
            .cloned()
            .ok_or_else(|| eyre::eyre!("no Solidity sources"))?;

        (version, scope.into_iter().collect())
    }
    // Otherwise, find the latest version among all compiled files.
    else {
        let (mut max_version, mut latest_paths) = (Version::new(0, 0, 0), Vec::new());
        for (path, _, version) in output.output().sources.sources_with_version() {
            // Only process Solidity files.
            if !output
                .graph()
                .get_parsed_source(path)
                .is_some_and(|ps| matches!(ps, MultiCompilerParsedSource::Solc(..)))
            {
                continue;
            }

            match version.cmp(&max_version) {
                // A newer version was found --> reset the list of paths
                std::cmp::Ordering::Greater => {
                    max_version = version.clone();
                    latest_paths.clear();
                    if let Ok(canonical_path) = dunce::canonicalize(path) {
                        latest_paths.push(canonical_path);
                    }
                }
                // A file with the same version was found --> add it to the list
                std::cmp::Ordering::Equal => {
                    if let Ok(canonical_path) = dunce::canonicalize(path) {
                        latest_paths.push(canonical_path);
                    }
                }
                // A file with an older version was found --> ignore
                std::cmp::Ordering::Less => {}
            }
        }

        (max_version, latest_paths)
    };

    // Read the file content for each of the determined paths.
    let mut sources = Sources::new();
    for path in source_paths.into_iter() {
        let source = Source::read(&path)?;
        sources.insert(path, source);
    }

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
