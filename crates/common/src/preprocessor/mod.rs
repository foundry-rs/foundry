use crate::errors::convert_solar_errors;
use foundry_compilers::{
    Compiler, ProjectPathsConfig, SourceParser, apply_updates,
    artifacts::SolcLanguage,
    error::Result,
    multi::{MultiCompiler, MultiCompilerInput, MultiCompilerLanguage},
    project::Preprocessor,
    solc::{SolcCompiler, SolcVersionedInput},
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

/// Returns the range of the given span in the source map.
#[track_caller]
fn span_to_range(source_map: &SourceMap, span: Span) -> Range<usize> {
    source_map.span_to_range(span).unwrap()
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

        let mut compiler =
            foundry_compilers::resolver::parse::SolParser::new(paths.with_language_ref())
                .into_compiler();
        let _ = compiler.enter_mut(|compiler| -> solar::interface::Result {
            let mut pcx = compiler.parse();

            // Add the sources into the context.
            // Include all sources in the source map so as to not re-load them from disk, but only
            // parse and preprocess tests and scripts.
            let mut preprocessed_paths = vec![];
            let sources = &mut input.input.sources;
            for (path, source) in sources.iter() {
                if let Ok(src_file) = compiler
                    .sess()
                    .source_map()
                    .new_source_file(path.clone(), source.content.as_str())
                    && paths.is_test_or_script(path)
                {
                    pcx.add_file(src_file);
                    preprocessed_paths.push(path.clone());
                }
            }

            // Parse and preprocess.
            pcx.parse();
            let ControlFlow::Continue(()) = compiler.lower_asts()? else { return Ok(()) };
            let gcx = compiler.gcx();
            // Collect tests and scripts dependencies and identify mock contracts.
            let deps = PreprocessorDependencies::new(
                gcx,
                &preprocessed_paths,
                &paths.paths_relative().sources,
                &paths.root,
                mocks,
            );
            // Collect data of source contracts referenced in tests and scripts.
            let data = collect_preprocessor_data(gcx, &deps.referenced_contracts);

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
