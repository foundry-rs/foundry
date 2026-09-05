use crate::errors::convert_solar_errors;
use foundry_compilers::{
    Compiler, ProjectPathsConfig, SourceParser, apply_updates,
    artifacts::SolcLanguage,
    cache::CompilerCache,
    error::Result,
    multi::{MultiCompiler, MultiCompilerInput, MultiCompilerLanguage, MultiCompilerSettings},
    project::Preprocessor,
    solc::{SolcCompiler, SolcSettings, SolcVersionedInput},
};
use solar::parse::{ast::Span, interface::SourceMap};
use std::{
    collections::HashSet,
    ops::{ControlFlow, Range},
    path::PathBuf,
};

mod data;
use data::{collect_preprocessor_data, create_deploy_helpers};

mod deps;
use deps::{PreprocessorDependencies, remove_bytecode_dependencies};

/// Preprocessor that replaces static bytecode linking in tests and scripts (`new Contract`) with
/// dynamic linkage through (`Vm.create*`).
///
/// This allows for more efficient caching when iterating on tests.
///
/// See <https://github.com/foundry-rs/foundry/pull/10010>.
#[derive(Debug)]
pub struct DynamicTestLinkingPreprocessor;

impl Preprocessor<SolcCompiler> for DynamicTestLinkingPreprocessor {
    #[instrument(name = "DynamicTestLinkingPreprocessor::preprocess", skip_all)]
    fn preprocess(
        &self,
        _solc: &SolcCompiler,
        input: &mut SolcVersionedInput,
        paths: &ProjectPathsConfig<SolcLanguage>,
        mocks: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        // Skip if we are not preprocessing any tests or scripts. Avoids unnecessary AST parsing.
        if !input.input.sources.iter().any(|(path, _)| paths.is_test_or_script(path)) {
            trace!("no tests or scripts to preprocess");
            return Ok(());
        }

        let mut compiler =
            foundry_compilers::resolver::parse::SolParser::new(paths.with_language_ref())
                .into_compiler();
        let _ = compiler.enter_mut(|compiler| -> solar::interface::Result {
            let mut pcx = compiler.parse();

            // Add the sources into the context.
            // Include all sources in the source map so as to not re-load them from disk, but only
            // parse and preprocess tests and scripts.
            let mut preprocessed_paths = vec![];
            let mut script_paths = HashSet::new();
            let sources = &mut input.input.sources;
            for (path, source) in sources.iter() {
                if let Ok(src_file) = compiler
                    .sess()
                    .source_map()
                    .new_source_file(path.clone(), source.content.as_str())
                    && paths.is_test_or_script(path)
                {
                    pcx.add_file(src_file);
                    if paths.is_script(path) {
                        script_paths.insert(path.clone());
                    }
                    preprocessed_paths.push(path.clone());
                }
            }

            // Parse and preprocess.
            pcx.parse();
            let ControlFlow::Continue(()) = compiler.lower_asts()? else { return Ok(()) };
            let gcx = compiler.gcx();
            let mut source_units = sources.keys().cloned().collect::<Vec<_>>();
            // Cache data is optional, including on the first compilation. Avoid the cache
            // reader diagnostics when probing for either supported settings format.
            let cache_files = crate::fs::read_to_string(&paths.cache).ok().and_then(|cache| {
                serde_json::from_str::<CompilerCache<MultiCompilerSettings>>(&cache)
                    .map(|cache| cache.files)
                    .or_else(|_| {
                        serde_json::from_str::<CompilerCache<SolcSettings>>(&cache)
                            .map(|cache| cache.files)
                    })
                    .ok()
            });
            if let Some(files) = cache_files {
                source_units.extend(
                    files
                        .into_keys()
                        .map(|path| path.strip_prefix(&paths.root).unwrap_or(&path).to_path_buf()),
                );
            }
            source_units.sort_unstable();
            source_units.dedup();
            // Collect tests and scripts dependencies and identify mock contracts.
            // Script paths are passed separately so salted new-expressions are left untouched
            // (Foundry's broadcast redirects native CREATE2 through the deterministic factory,
            // but vm.deployCode runs at a deeper depth and bypasses that redirect).
            let deps = PreprocessorDependencies::new(
                gcx,
                &preprocessed_paths,
                &script_paths,
                paths,
                &source_units,
                mocks,
            );
            // Collect data of source contracts referenced in tests and scripts.
            let data = collect_preprocessor_data(gcx, &deps.referenced_contracts, &paths.root);

            // Extend existing sources with preprocessor deploy helper sources.
            sources.extend(create_deploy_helpers(&data));

            // Generate and apply preprocessor source updates.
            apply_updates(sources, remove_bytecode_dependencies(gcx, &deps, &data));

            Ok(())
        });

        // Warn if any diagnostics emitted during content parsing.
        if let Err(err) = convert_solar_errors(compiler.dcx()) {
            warn!(%err, "failed preprocessing");
        }

        Ok(())
    }
}

impl Preprocessor<MultiCompiler> for DynamicTestLinkingPreprocessor {
    fn preprocess(
        &self,
        compiler: &MultiCompiler,
        input: &mut <MultiCompiler as Compiler>::Input,
        paths: &ProjectPathsConfig<MultiCompilerLanguage>,
        mocks: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        // Preprocess only Solc compilers.
        let MultiCompilerInput::Solc(input) = input else { return Ok(()) };

        let Some(solc) = &compiler.solc else { return Ok(()) };

        let paths = paths.clone().with_language::<SolcLanguage>();
        self.preprocess(solc, input, &paths, mocks)
    }
}

/// Returns the range of the given span in the source map.
#[track_caller]
fn span_to_range(source_map: &SourceMap, span: Span) -> Range<usize> {
    source_map.span_to_range(span).unwrap()
}
