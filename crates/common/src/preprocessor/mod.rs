use crate::{errors::convert_solar_errors, fs::normalize_path};
use foundry_compilers::{
    Compiler, ProjectPathsConfig, SourceParser, apply_updates,
    artifacts::{EvmVersion, SolcLanguage},
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
use deps::{ConstructorContext, PreprocessorDependencies, remove_bytecode_dependencies};

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
        7
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

        let constructor_context = ConstructorContext {
            abi_coder_v2: input.version >= semver::Version::new(0, 8, 0),
            // Without an explicit target, preserve native validation rather than guessing the
            // selected compiler's default EVM version.
            supports_create2: input
                .input
                .settings
                .evm_version
                .is_some_and(|version| version >= EvmVersion::Constantinople),
        };
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
                constructor_context,
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
        7
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
                "import '../src/Dep.sol'; contract Deploy { bytes constant CODE = type(Dep).creationCode; bytes32 constant CODE_HASH = keccak256(type(Dep).creationCode); bytes32 immutable codeHash = keccak256(type(Dep).creationCode); function deploy() public returns (Dep) { return new Dep(); } function native() public pure returns (bytes memory) { return type(Dep).creationCode; } }",
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
        assert!(
            source.contains("bytes constant CODE = type(Dep).creationCode"),
            "constant creation code was rewritten"
        );
        assert!(
            source.contains("bytes32 constant CODE_HASH = keccak256(type(Dep).creationCode)"),
            "constant creation code hash was rewritten"
        );
        assert!(
            !source.contains("immutable codeHash = keccak256(type(Dep).creationCode)"),
            "constant preservation leaked into the immutable initializer"
        );
        assert!(
            source.contains("immutable codeHash = keccak256(VmContractHelper"),
            "immutable creation code was not rewritten"
        );
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

    #[test]
    fn constructor_abi_context_preserves_source_pragmas_and_version_defaults() {
        for (minor, pragma, native) in [
            (7, "", true),
            (8, "", false),
            (8, "pragma abicoder /* comment */ v1;", true),
            (7, "pragma abicoder v2;", false),
            (7, "pragma experimental ABIEncoderV2;", false),
        ] {
            let (_root, paths, mut input) = input();
            input.version = Version::new(0, minor, 6);
            input.input.sources = [
                ("src/Dep.sol", "contract Dep { constructor(uint256 n) {} }".to_string()),
                ("test/Deploy.sol", format!("{pragma} import '../src/Dep.sol'; contract Deploy {{ function deploy() public {{ new Dep(7); }} }}")),
                // A pragma in another source must not change this source's ABI context.
                ("test/Other.sol", "pragma abicoder v2; import '../src/Dep.sol'; contract Other { function deploy() public { new Dep(7); } }".to_string()),
            ].into_iter().map(|(path, source)| (PathBuf::from(path), Source::new(source))).collect();
            <DynamicTestLinkingPreprocessor as Preprocessor<SolcCompiler>>::preprocess(
                &DynamicTestLinkingPreprocessor,
                &SolcCompiler::default(),
                &mut input,
                &paths,
                &mut HashSet::new(),
            )
            .unwrap();
            let source = &input.input.sources[&PathBuf::from("test/Deploy.sol")].content;
            assert_eq!(source.contains("new Dep(7)"), native, "0.{minor}.6 {pragma}");
            let other = &input.input.sources[&PathBuf::from("test/Other.sol")].content;
            assert!(!other.contains("new Dep(7)"), "unrelated v2 source was not rewritten");
        }
    }

    #[test]
    fn constructor_salt_uses_compiler_evm_target() {
        for (evm_version, native) in [
            (None, true),
            (Some(EvmVersion::Byzantium), true),
            (Some(EvmVersion::Constantinople), false),
            (Some(EvmVersion::Prague), false),
        ] {
            let (_root, paths, mut input) = input();
            input.input.settings.evm_version = evm_version;
            input.input.sources.insert(
                PathBuf::from("test/Deploy.sol"),
                Source::new("pragma abicoder v1; import '../src/Dep.sol'; contract Deploy { function deploy() public { new Dep{salt: bytes32(0)}(); } function plain() public { new Dep(); } }"),
            );
            <DynamicTestLinkingPreprocessor as Preprocessor<SolcCompiler>>::preprocess(
                &DynamicTestLinkingPreprocessor,
                &SolcCompiler::default(),
                &mut input,
                &paths,
                &mut HashSet::new(),
            )
            .unwrap();
            let source = &input.input.sources[&PathBuf::from("test/Deploy.sol")].content;
            assert_eq!(source.contains("new Dep{salt:"), native, "{evm_version:?}");
            assert!(
                !source.contains("new Dep();"),
                "ordinary parameterless CREATE was not rewritten"
            );
        }
    }

    #[test]
    fn return_data_fallback_follows_helpers_without_disabling_unrelated_contracts() {
        for (helper, declarations, expression) in [
            (
                "function size(uint256) view returns (uint256 n) { assembly { n := returndatasize() } }",
                "import * as R from '../src/Read.sol';",
                "R.size(0)",
            ),
            (
                "function size(uint256) view returns (uint256 n) { assembly { n := returndatasize() } }",
                "import {size} from '../src/Read.sol'; using {size} for uint256;",
                "uint256(0).size()",
            ),
            (
                "function size(uint256) view returns (uint256 n) { assembly { n := returndatasize() } }",
                "import {size as readSize} from '../src/Read.sol'; using {readSize} for uint256;",
                "uint256(0).readSize()",
            ),
            (
                "library R { function size(uint256) internal view returns (uint256 n) { assembly { n := returndatasize() } } }",
                "import {R} from '../src/Read.sol'; using R for uint256;",
                "uint256(0).size()",
            ),
            (
                "library R { function size(uint256) internal view returns (uint256 n) { assembly { n := returndatasize() } } }",
                "import {R} from '../src/Read.sol';",
                "R.size(0)",
            ),
            (
                "library R { function size(uint256) internal pure returns (uint256 n) { assembly { n := returndatasize() } } }",
                "import {R} from '../src/Read.sol';",
                "(R).size(0)",
            ),
            (
                "type Word is uint256; using {size as -} for Word global; function size(Word) pure returns (Word) { uint256 n; assembly { n := returndatasize() } return Word.wrap(n); }",
                "import {Word} from '../src/Read.sol';",
                "Word.unwrap(-Word.wrap(0))",
            ),
            (
                "type Word is uint256; using {size as +} for Word global; function size(Word, Word) pure returns (Word) { uint256 n; assembly { n := returndatasize() } return Word.wrap(n); }",
                "import {Word} from '../src/Read.sol';",
                "Word.unwrap(Word.wrap(0) + Word.wrap(0))",
            ),
        ] {
            let (_root, paths, mut input) = input();
            input.input.sources.insert(PathBuf::from("src/Read.sol"), Source::new(helper));
            let observer = format!(
                "{declarations} import '../src/Dep.sol'; contract Observer {{ function observe() public {{ new Dep(); require({expression} == 0); }} }}"
            );
            let safe = "import '../src/Read.sol'; import '../src/Dep.sol'; contract Safe { function deploy() public { new Dep(); } }";
            input.input.sources.insert(PathBuf::from("test/Observer.sol"), Source::new(&observer));
            input.input.sources.insert(PathBuf::from("test/Safe.sol"), Source::new(safe));
            <DynamicTestLinkingPreprocessor as Preprocessor<SolcCompiler>>::preprocess(
                &DynamicTestLinkingPreprocessor,
                &SolcCompiler::default(),
                &mut input,
                &paths,
                &mut HashSet::new(),
            )
            .unwrap();
            assert_eq!(
                input.input.sources[&PathBuf::from("test/Observer.sol")].content.as_str(),
                observer
            );
            assert_ne!(input.input.sources[&PathBuf::from("test/Safe.sol")].content.as_str(), safe);
        }
    }

    #[test]
    fn return_data_fallback_includes_inherited_observers() {
        assert_return_data_scope_native(
            "abstract contract Base { function make() internal virtual; function observe() public { make(); uint256 n; assembly { n := returndatasize() } require(n == 0); } } contract Derived is Base { function make() internal override { new Dep(); } }",
        );
    }

    #[test]
    fn return_data_fallback_collects_using_before_initializers() {
        assert_return_data_scope_native(
            "library Reader { function size(uint256) internal view returns (uint256 n) { assembly { n := returndatasize() } } } contract Initializer { using Reader for uint256; Dep target = new Dep(); uint256 observed = uint256(0).size(); function observe() public view { require(observed == 0); } }",
        );
    }

    fn assert_return_data_scope_native(declarations: &str) {
        let (_root, paths, mut input) = input();
        let observer = format!("import '../src/Dep.sol'; {declarations}");
        let safe =
            "import '../src/Dep.sol'; contract Safe { function deploy() public { new Dep(); } }";
        input.input.sources.insert(PathBuf::from("test/Observer.sol"), Source::new(&observer));
        input.input.sources.insert(PathBuf::from("test/Safe.sol"), Source::new(safe));
        <DynamicTestLinkingPreprocessor as Preprocessor<SolcCompiler>>::preprocess(
            &DynamicTestLinkingPreprocessor,
            &SolcCompiler::default(),
            &mut input,
            &paths,
            &mut HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            input.input.sources[&PathBuf::from("test/Observer.sol")].content.as_str(),
            observer
        );
        assert_ne!(input.input.sources[&PathBuf::from("test/Safe.sol")].content.as_str(), safe);
    }

    #[test]
    fn return_data_namespace_resolution_selects_exports() {
        for (namespace, expression, native) in [
            ("Read", "R.identity(7)", false),
            ("Read", "R.size(0)", true),
            ("Export", "R.identity(7)", false),
            ("Export", "R.observe(0)", true),
            ("Nested", "R.Inner.identity(7)", false),
            ("Nested", "R.Inner.observe(0)", true),
            ("Read", "R.Reader.identity(7)", false),
            ("Read", "R.Reader.size(0)", true),
            ("Export", "R.Renamed.identity(7)", false),
            ("Export", "R.Renamed.size(0)", true),
        ] {
            let (_root, paths, mut input) = input();
            for (path, source) in [
                (
                    "src/Read.sol",
                    "function identity(uint256 n) pure returns (uint256) { return n; } function size(uint256) view returns (uint256 n) { assembly { n := returndatasize() } } library Reader { function identity(uint256 n) internal pure returns (uint256) { return n; } function size(uint256) internal view returns (uint256 n) { assembly { n := returndatasize() } } }",
                ),
                (
                    "src/Export.sol",
                    "import {size as observe, identity, Reader as Renamed} from './Read.sol';",
                ),
                ("src/Nested.sol", "import * as Inner from './Export.sol';"),
            ] {
                input.input.sources.insert(PathBuf::from(path), Source::new(source));
            }
            let source = format!(
                "import '../src/Dep.sol'; import * as R from '../src/{namespace}.sol'; contract Case {{ function run() public {{ new Dep(); {expression}; }} }}"
            );
            input.input.sources.insert(PathBuf::from("test/Case.sol"), Source::new(&source));
            <DynamicTestLinkingPreprocessor as Preprocessor<SolcCompiler>>::preprocess(
                &DynamicTestLinkingPreprocessor,
                &SolcCompiler::default(),
                &mut input,
                &paths,
                &mut HashSet::new(),
            )
            .unwrap();
            let actual = &input.input.sources[&PathBuf::from("test/Case.sol")].content;
            assert_eq!(actual.contains("new Dep();"), native, "{namespace}: {expression}");
        }
    }
}
