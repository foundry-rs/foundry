use crate::{opts::BuildOpts, utils::LoadConfig};

use eyre::Result;
use foundry_compilers::{
    CompilerInput, Graph, Project,
    artifacts::{Source, Sources},
    multi::{MultiCompilerLanguage, MultiCompilerParsedSource},
    solc::{SolcLanguage, SolcVersionedInput},
};
use solar_sema::{ParsingContext, interface::Session};
use std::path::PathBuf;

/// Builds a Solar [`solar_sema::ParsingContext`] from [`BuildOpts`].
///
/// * Configures include paths, remappings and registers all in-memory sources so that solar can
///   operate without touching disk.
/// * If no `target_paths` are provided, all project files are processed.
/// * Only processes the subset of sources with the most up-to-date Solitidy version.
pub fn solar_pcx_from_build_opts<'sess>(
    sess: &'sess Session,
    build: BuildOpts,
    target_paths: Option<Vec<PathBuf>>,
) -> Result<ParsingContext<'sess>> {
    // Process build options
    let config = build.load_config()?;
    let project = config.ephemeral_project()?;

    let sources = match target_paths {
        // If target files are provided, only process those sources
        Some(targets) => {
            let mut sources = Sources::new();
            for t in targets.into_iter() {
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
    let graph = Graph::<MultiCompilerParsedSource>::resolve_sources(&project.paths, sources)?;
    let (version, sources, _) = graph
        // resolve graph into mapping language -> version -> sources
        .into_sources_by_version(&project)?
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

    Ok(solar_pcx_from_solc_project(sess, &project, &solc, true))
}

/// Builds a Solar [`solar_sema::ParsingContext`] from a  [`foundry_compilers::Project`] and a
/// [`SolcVersionedInput`].
///
/// * Configures include paths, remappings.
/// * Source files can be manually added if the param `add_source_file` is set to `false`.
pub fn solar_pcx_from_solc_project<'sess>(
    sess: &'sess Session,
    project: &Project,
    solc: &SolcVersionedInput,
    add_source_files: bool,
) -> ParsingContext<'sess> {
    // Configure the parsing context with the paths, remappings and sources
    let mut pcx = ParsingContext::new(sess);

    pcx.file_resolver
        .set_current_dir(solc.cli_settings.base_path.as_ref().unwrap_or(&project.paths.root));
    for remapping in &project.paths.remappings {
        pcx.file_resolver.add_import_remapping(solar_sema::interface::config::ImportRemapping {
            context: remapping.context.clone().unwrap_or_default(),
            prefix: remapping.name.clone(),
            path: remapping.path.clone(),
        });
    }
    pcx.file_resolver.add_include_paths(solc.cli_settings.include_paths.iter().cloned());

    if add_source_files {
        for (path, source) in &solc.input.sources {
            if let Ok(src_file) =
                sess.source_map().new_source_file(path.clone(), source.content.as_str())
            {
                pcx.add_file(src_file);
            }
        }
    }

    pcx
}
