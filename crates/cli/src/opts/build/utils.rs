use crate::{opts::BuildOpts, utils::LoadConfig};

use foundry_compilers::{
    artifacts::{Source, Sources},
    solc::{SolcLanguage, SolcVersionedInput},
    CompilerInput,
};
use solar_sema::{interface::Session, ParsingContext};
use std::path::PathBuf;

/// Builds a Solar [`ParsingContext`] from [`BuildOpts`].
///
/// Configures include paths, remappings and registers all in-memory sources so
/// that solar can operate without touching disk.
pub fn solar_pcx_from_build_opts<'sess>(
    sess: &'sess Session,
    build: BuildOpts,
    target_paths: Vec<PathBuf>,
) -> eyre::Result<ParsingContext<'sess>> {
    // Process build options
    let config = build.load_config()?;
    let project = config.ephemeral_project()?;

    // TODO: ask if taking 0.8.0 as default is fine, or if i should return an error and force
    // users to either configure the version in the toml or pass the `--use version`
    // flag
    let version = match config.solc_version() {
        Some(version) => version,
        None => "0.8.0".parse()?,
    };

    let mut sources = Sources::new();
    for t in target_paths.into_iter() {
        let target_path = dunce::canonicalize(t)?;
        let source = Source::read(&target_path)?;
        sources.insert(target_path, source);
    }

    let solc = SolcVersionedInput::build(
        sources,
        config.solc_settings()?,
        SolcLanguage::Solidity,
        version,
    );

    // Configure the parsing context with the paths, remappings and sources
    let mut pcx = ParsingContext::new(sess);

    pcx.file_resolver
        .set_current_dir(solc.cli_settings.base_path.as_ref().unwrap_or(&project.paths.root));
    for remapping in project.paths.remappings.into_iter() {
        pcx.file_resolver.add_import_remapping(solar_sema::interface::config::ImportRemapping {
            context: remapping.context.unwrap_or_default(),
            prefix: remapping.name,
            path: remapping.path.clone(),
        });
    }
    pcx.file_resolver.add_include_paths(solc.cli_settings.include_paths.iter().cloned());

    for (path, source) in &solc.input.sources {
        if let Ok(src_file) =
            sess.source_map().new_source_file(path.clone(), source.content.as_str())
        {
            pcx.add_file(src_file);
        }
    }

    Ok(pcx)
}
