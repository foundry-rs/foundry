use eyre::Result;
use foundry_compilers::{
    CompilerInput, Graph, Project,
    artifacts::{Source, Sources},
    multi::{MultiCompilerLanguage, MultiCompilerParser},
    solc::{SolcLanguage, SolcVersionedInput},
};
use foundry_config::Config;
use solar_sema::ParsingContext;
use std::path::PathBuf;

/// Configures a Solar [`solar_sema::ParsingContext`] from [`BuildOpts`].
///
/// * Configures include paths, remappings and registers all in-memory sources so that solar can
///   operate without touching disk.
/// * If no `project` is provided, it will spin up a new ephemeral project.
/// * If no `target_paths` are provided, all project files are processed.
/// * Only processes the subset of sources with the most up-to-date Solitidy version.
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
        // resolve graph into mapping language -> version -> sources
        .into_sources_by_version(project)?
        .sources
        .into_iter()
        // only interested in Solidity sources
        .find(|(lang, _)| *lang == MultiCompilerLanguage::Solc(SolcLanguage::Solidity))
        .ok_or_else(|| eyre::eyre!("no Solidity sources"))?
        .1
        .into_iter()
        // always pick the latest version
        .max_by(|(v1, _, _), (v2, _, _)| v1.cmp(v2))
        .unwrap();

    let solc = SolcVersionedInput::build(
        sources,
        config.solc_settings()?,
        SolcLanguage::Solidity,
        version,
    );

    configure_pcx_from_solc(pcx, project, &solc, true);

    Ok(())
}

/// Configures a Solar [`solar_sema::ParsingContext`] from a [`foundry_compilers::Project`] and a
/// [`SolcVersionedInput`].
///
/// * Configures include paths, remappings.
/// * Source files can be manually added if the param `add_source_file` is set to `false`.
pub fn configure_pcx_from_solc(
    pcx: &mut ParsingContext<'_>,
    project: &Project,
    vinput: &SolcVersionedInput,
    add_source_files: bool,
) {
    configure_pcx_from_solc_cli(pcx, project, &vinput.cli_settings);
    if add_source_files {
        for (path, source) in &vinput.input.sources {
            if let Ok(src_file) =
                pcx.sess.source_map().new_source_file(path.clone(), source.content.as_str())
            {
                pcx.add_file(src_file);
            }
        }
    }
}

fn configure_pcx_from_solc_cli(
    pcx: &mut ParsingContext<'_>,
    project: &Project,
    cli_settings: &foundry_compilers::solc::CliSettings,
) {
    pcx.file_resolver
        .set_current_dir(cli_settings.base_path.as_ref().unwrap_or(&project.paths.root));
    for remapping in &project.paths.remappings {
        pcx.file_resolver.add_import_remapping(solar_sema::interface::config::ImportRemapping {
            context: remapping.context.clone().unwrap_or_default(),
            prefix: remapping.name.clone(),
            path: remapping.path.clone(),
        });
    }
    pcx.file_resolver.add_include_paths(cli_settings.include_paths.iter().cloned());
}
