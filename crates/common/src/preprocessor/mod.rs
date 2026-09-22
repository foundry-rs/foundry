use crate::{errors::convert_solar_errors, fs::normalize_path};
use foundry_compilers::{
    Compiler, ProjectPathsConfig, SourceParser, apply_updates,
    artifacts::SolcLanguage,
    error::Result,
    multi::{MultiCompiler, MultiCompilerInput, MultiCompilerLanguage},
    project::{NativeDependencyState, Preprocessor, PreprocessorState},
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

/// Preprocessor that replaces static bytecode linking in tests and scripts (`new Contract`) with
/// dynamic linkage through (`Vm.create*`).
///
/// This allows for more efficient caching when iterating on tests.
///
/// See <https://github.com/foundry-rs/foundry/pull/10010>.
#[derive(Debug)]
pub struct DynamicTestLinkingPreprocessor;

impl Preprocessor<SolcCompiler> for DynamicTestLinkingPreprocessor {
    fn cache_version(&self) -> u64 {
        2
    }

    fn preprocess(
        &self,
        solc: &SolcCompiler,
        input: &mut SolcVersionedInput,
        paths: &ProjectPathsConfig<SolcLanguage>,
        mocks: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        let source_units = input.input.sources.keys().cloned().collect::<Vec<_>>();
        self.preprocess_with_dependencies(
            solc,
            input,
            paths,
            mocks,
            &mut PreprocessorState::untracked(),
            &source_units,
        )
    }

    #[instrument(name = "DynamicTestLinkingPreprocessor::preprocess", skip_all)]
    fn preprocess_with_dependencies(
        &self,
        _solc: &SolcCompiler,
        input: &mut SolcVersionedInput,
        paths: &ProjectPathsConfig<SolcLanguage>,
        mocks: &mut HashSet<PathBuf>,
        preprocessor_state: &mut PreprocessorState,
        source_units: &[PathBuf],
    ) -> Result<()> {
        // Skip if we are not preprocessing any tests or scripts. Avoids unnecessary AST parsing.
        if !input.input.sources.iter().any(|(path, _)| paths.is_test_or_script(path)) {
            trace!("no tests or scripts to preprocess");
            return Ok(());
        }

        let original_sources = input.input.sources.clone();
        let mut parser_paths = paths.clone();
        parser_paths.include_paths.extend(input.cli_settings.include_paths.iter().cloned());
        let mut compiler =
            foundry_compilers::resolver::parse::SolParser::new(parser_paths.with_language_ref())
                .into_compiler();
        let result = compiler.enter_mut(|compiler| -> solar::interface::Result {
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
            // Collect tests and scripts dependencies and identify mock contracts.
            // Script paths are passed separately so salted new-expressions are left untouched
            // (Foundry's broadcast redirects native CREATE2 through the deterministic factory,
            // but vm.deployCode runs at a deeper depth and bypasses that redirect).
            let deps = PreprocessorDependencies::new(
                gcx,
                &preprocessed_paths,
                &script_paths,
                paths,
                source_units,
                mocks,
                preprocessor_state,
            );
            // Collect data of source contracts referenced in tests and scripts.
            let data = collect_preprocessor_data(
                gcx,
                &deps.referenced_contracts,
                &paths.root,
                source_units,
            );

            // Extend existing sources with preprocessor deploy helper sources.
            sources.extend(create_deploy_helpers(&data));

            // Generate and apply preprocessor source updates.
            apply_updates(sources, remove_bytecode_dependencies(gcx, &deps, &data));

            Ok(())
        });

        let diagnostics = convert_solar_errors(compiler.dcx());
        if result.is_err() || diagnostics.is_err() {
            if let Err(err) = diagnostics {
                warn!(%err, "dynamic test linking analysis failed; using native bytecode");
            } else {
                warn!("dynamic test linking analysis failed; using native bytecode");
            }
            input.input.sources = original_sources;
            mark_conservative(paths, &input.input.sources, preprocessor_state);
        }

        Ok(())
    }
}

impl Preprocessor<MultiCompiler> for DynamicTestLinkingPreprocessor {
    fn cache_version(&self) -> u64 {
        2
    }

    fn preprocess(
        &self,
        compiler: &MultiCompiler,
        input: &mut <MultiCompiler as Compiler>::Input,
        paths: &ProjectPathsConfig<MultiCompilerLanguage>,
        mocks: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        let source_units = match input {
            MultiCompilerInput::Solc(input) => {
                input.input.sources.keys().cloned().collect::<Vec<_>>()
            }
            _ => Vec::new(),
        };
        self.preprocess_with_dependencies(
            compiler,
            input,
            paths,
            mocks,
            &mut PreprocessorState::untracked(),
            &source_units,
        )
    }

    fn preprocess_with_dependencies(
        &self,
        compiler: &MultiCompiler,
        input: &mut <MultiCompiler as Compiler>::Input,
        paths: &ProjectPathsConfig<MultiCompilerLanguage>,
        mocks: &mut HashSet<PathBuf>,
        preprocessor_state: &mut PreprocessorState,
        source_units: &[PathBuf],
    ) -> Result<()> {
        // Preprocess only Solc compilers.
        let MultiCompilerInput::Solc(input) = input else { return Ok(()) };

        let Some(solc) = &compiler.solc else { return Ok(()) };

        let paths = paths.clone().with_language::<SolcLanguage>();
        <Self as Preprocessor<SolcCompiler>>::preprocess_with_dependencies(
            self,
            solc,
            input,
            &paths,
            mocks,
            preprocessor_state,
            source_units,
        )
    }
}

/// Falls back to native bytecode and invalidates affected files after any project source change.
fn mark_conservative(
    paths: &ProjectPathsConfig<SolcLanguage>,
    sources: &foundry_compilers::artifacts::Sources,
    preprocessor_state: &mut PreprocessorState,
) {
    for path in sources.keys().filter(|path| paths.is_test_or_script(path)) {
        let path = normalize_path(&paths.root.join(path));
        preprocessor_state.update(path, Some(NativeDependencyState::Conservative));
    }
}

/// Returns the range of the given span in the source map.
#[track_caller]
fn span_to_range(source_map: &SourceMap, span: Span) -> Range<usize> {
    source_map.span_to_range(span).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_compilers::{CompilerInput, artifacts::Source, solc::SolcSettings};
    use semver::Version;

    fn input() -> (tempfile::TempDir, ProjectPathsConfig<SolcLanguage>, SolcVersionedInput) {
        let root = tempfile::tempdir().unwrap();
        let paths = ProjectPathsConfig::builder().root(root.path()).build().unwrap();
        let sources = [
            ("src/Dep.sol", "contract Dep {}"),
            ("test/Mock.sol", "import '../src/Dep.sol'; contract Mock is Dep {}"),
            (
                "test/Deploy.sol",
                "import '../src/Dep.sol'; contract Deploy { function deploy() public returns (Dep) { return new Dep(); } function native() public pure returns (bytes memory) { return type(Dep).creationCode; } }",
            ),
        ]
        .into_iter()
        .map(|(path, content)| (PathBuf::from(path), Source::new(content)))
        .collect();
        let input = SolcVersionedInput::build(
            sources,
            SolcSettings::default(),
            SolcLanguage::Solidity,
            Version::new(0, 8, 30),
        );
        (root, paths, input)
    }

    fn assert_preprocessed(
        paths: &ProjectPathsConfig<SolcLanguage>,
        input: &SolcVersionedInput,
        mocks: &HashSet<PathBuf>,
    ) {
        let source = &input.input.sources[&PathBuf::from("test/Deploy.sol")].content;
        assert!(!source.contains("return new Dep();"), "eligible deployment was not rewritten");
        assert!(source.contains("type(Dep).creationCode"), "native dependency was lost");
        assert!(mocks.contains(&paths.root.join("test/Mock.sol")));
    }

    #[test]
    fn direct_solc_preprocess_tracks_mocks_without_cache_context() {
        let (_root, paths, mut input) = input();
        let mut mocks = HashSet::new();
        <DynamicTestLinkingPreprocessor as Preprocessor<SolcCompiler>>::preprocess(
            &DynamicTestLinkingPreprocessor,
            &SolcCompiler::default(),
            &mut input,
            &paths,
            &mut mocks,
        )
        .unwrap();
        assert_preprocessed(&paths, &input, &mocks);
    }

    #[test]
    fn direct_multi_preprocess_tracks_mocks_without_cache_context() {
        let (_root, paths, input) = input();
        let mut input = MultiCompilerInput::Solc(Box::new(input));
        let mut mocks = HashSet::new();
        <DynamicTestLinkingPreprocessor as Preprocessor<MultiCompiler>>::preprocess(
            &DynamicTestLinkingPreprocessor,
            &MultiCompiler { solc: Some(SolcCompiler::default()), vyper: None },
            &mut input,
            paths.with_language_ref(),
            &mut mocks,
        )
        .unwrap();
        let MultiCompilerInput::Solc(input) = input else { unreachable!() };
        assert_preprocessed(&paths, &input, &mocks);
    }
}
