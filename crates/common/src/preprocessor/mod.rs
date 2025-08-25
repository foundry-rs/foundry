use foundry_compilers::{
    Compiler, Language, ProjectPathsConfig, apply_updates,
    artifacts::SolcLanguage,
    error::Result,
    multi::{MultiCompiler, MultiCompilerInput, MultiCompilerLanguage},
    project::Preprocessor,
    solc::{SolcCompiler, SolcVersionedInput},
};
use solar_parse::{
    ast::Span,
    interface::{Session, SourceMap},
};
use solar_sema::{ParsingContext, thread_local::ThreadLocal};
use std::{collections::HashSet, ops::Range, path::PathBuf};

mod data;
use data::{collect_preprocessor_data, create_deploy_helpers};

mod deps;
use deps::{PreprocessorDependencies, remove_bytecode_dependencies};

/// Returns the range of the given span in the source map.
#[track_caller]
fn span_to_range(source_map: &SourceMap, span: Span) -> Range<usize> {
    source_map.span_to_source(span).unwrap().1
}

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
            trace!("no tests or sources to preprocess");
            return Ok(());
        }

        let sess = solar_session_from_solc(input);
        let _ = sess.enter_parallel(|| -> solar_parse::interface::Result {
            // Set up the parsing context with the project paths.
            let mut parsing_context = solar_pcx_from_solc_no_sources(&sess, input, paths);

            // Add the sources into the context.
            // Include all sources in the source map so as to not re-load them from disk, but only
            // parse and preprocess tests and scripts.
            let mut preprocessed_paths = vec![];
            let sources = &mut input.input.sources;
            for (path, source) in sources.iter() {
                if let Ok(src_file) =
                    sess.source_map().new_source_file(path.clone(), source.content.as_str())
                    && paths.is_test_or_script(path)
                {
                    parsing_context.add_file(src_file);
                    preprocessed_paths.push(path.clone());
                }
            }

            // Parse and preprocess.
            let hir_arena = ThreadLocal::new();
            if let Some(gcx) = parsing_context.parse_and_lower(&hir_arena)? {
                let hir = &gcx.get().hir;
                // Collect tests and scripts dependencies and identify mock contracts.
                let deps = PreprocessorDependencies::new(
                    &sess,
                    hir,
                    &preprocessed_paths,
                    &paths.paths_relative().sources,
                    &paths.root,
                    mocks,
                );
                // Collect data of source contracts referenced in tests and scripts.
                let data = collect_preprocessor_data(&sess, hir, &deps.referenced_contracts);

                // Extend existing sources with preprocessor deploy helper sources.
                sources.extend(create_deploy_helpers(&data));

                // Generate and apply preprocessor source updates.
                apply_updates(sources, remove_bytecode_dependencies(hir, &deps, &data));
            }

            Ok(())
        });

        // Warn if any diagnostics emitted during content parsing.
        if let Err(err) = sess.emitted_errors().unwrap() {
            warn!("failed preprocessing {err}");
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

fn solar_session_from_solc(solc: &SolcVersionedInput) -> Session {
    use solar_parse::interface::config;

    Session::builder()
        .with_buffer_emitter(Default::default())
        .opts(config::Opts {
            language: match solc.input.language {
                SolcLanguage::Solidity => config::Language::Solidity,
                SolcLanguage::Yul => config::Language::Yul,
                _ => unimplemented!(),
            },

            // TODO: ...
            /*
            evm_version: solc.input.settings.evm_version,
            */
            ..Default::default()
        })
        .build()
}

fn solar_pcx_from_solc_no_sources<'sess>(
    sess: &'sess Session,
    solc: &SolcVersionedInput,
    paths: &ProjectPathsConfig<impl Language>,
) -> ParsingContext<'sess> {
    let mut pcx = ParsingContext::new(sess);
    pcx.file_resolver.set_current_dir(solc.cli_settings.base_path.as_ref().unwrap_or(&paths.root));
    for remapping in &paths.remappings {
        pcx.file_resolver.add_import_remapping(solar_sema::interface::config::ImportRemapping {
            context: remapping.context.clone().unwrap_or_default(),
            prefix: remapping.name.clone(),
            path: remapping.path.clone(),
        });
    }
    pcx.file_resolver.add_include_paths(solc.cli_settings.include_paths.iter().cloned());
    pcx
}
