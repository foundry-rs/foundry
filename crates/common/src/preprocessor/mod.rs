use crate::{errors::convert_solar_errors, fs::normalize_path};
use foundry_compilers::{
    Compiler, ProjectPathsConfig, SourceParser, apply_updates,
    artifacts::{EvmVersion, SolcLanguage},
    error::Result,
    multi::{MultiCompiler, MultiCompilerInput, MultiCompilerLanguage},
    project::{
        NativeDependencies, NativeDependencyState, Preprocessor, PreprocessorState,
        merge_native_dependencies,
    },
    resolver::{
        Graph,
        parse::{SolData, SolParser},
    },
    solc::{SolcCompiler, SolcVersionedInput},
};
use solar::parse::{ast::Span, interface::SourceMap};
use std::{
    collections::{BTreeSet, HashSet},
    ops::{ControlFlow, Range},
    path::PathBuf,
};

mod data;
use data::{collect_preprocessor_data, create_deploy_helpers};

pub use data::is_deploy_helper_path;

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
        8
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
            compiler.dcx().has_errors()?;
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
            let reason = diagnostics.err().map(|err| err.to_string());
            if let Some(err) = &reason {
                warn!(%err, "dynamic test linking analysis failed; using native bytecode");
            } else {
                warn!("dynamic test linking analysis failed; using native bytecode");
            }
            input.input.sources = original_sources;
            let dependencies = native_import_dependencies(&parser_paths, &input.input.sources);
            let affected = dependencies.len();
            for (path, state) in dependencies {
                if preprocessor_state.update(path.clone(), Some(state)) {
                    mocks.remove(&path);
                }
            }
            if affected > 0 {
                let reason = reason
                    .as_deref()
                    .and_then(solar_error_summary)
                    .unwrap_or_else(|| "Solar analysis failed".to_string());
                let _ = sh_warn!("dynamic test linking disabled for {affected} files: {reason}");
            }
        }

        Ok(())
    }
}

impl Preprocessor<MultiCompiler> for DynamicTestLinkingPreprocessor {
    fn cache_version(&self) -> u64 {
        8
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

/// Tracks transitive imports of unmodified tests and scripts after analysis fails.
fn native_import_dependencies(
    paths: &ProjectPathsConfig<SolcLanguage>,
    sources: &foundry_compilers::artifacts::Sources,
) -> NativeDependencies {
    let Ok(graph) = Graph::<SolParser>::resolve_sources(paths, sources.clone()) else {
        return sources
            .keys()
            .filter(|path| paths.is_test_or_script(path))
            .map(|path| {
                (normalize_path(&paths.root.join(path)), NativeDependencyState::Conservative)
            })
            .collect();
    };

    // Keep conservative invalidation only where import extraction itself is incomplete.
    let mut incomplete = HashSet::new();
    if graph.nodes.iter().any(|node| node.data.parse_result.is_err()) {
        incomplete.extend(
            graph
                .nodes
                .iter()
                .filter(|node| SolData::parse(node.content(), node.path()).parse_result.is_err())
                .map(|node| normalize_path(node.path())),
        );
    }
    let graph_paths = graph.files().keys().cloned().collect::<HashSet<_>>();
    let (_, edges) = graph.into_sources();
    incomplete.extend(edges.unresolved_imports().iter().map(|(_, path)| normalize_path(path)));

    let mut dependencies = NativeDependencies::new();
    for path in sources.keys().filter(|path| paths.is_test_or_script(path)) {
        let graph_path = paths.root.join(path);
        let path = normalize_path(&graph_path);
        let state = if graph_paths.contains(&graph_path) {
            let imports =
                edges.imports(&graph_path).into_iter().map(normalize_path).collect::<BTreeSet<_>>();
            if incomplete.contains(&path) || incomplete.iter().any(|path| imports.contains(path)) {
                NativeDependencyState::Conservative
            } else {
                NativeDependencyState::Known(imports)
            }
        } else {
            NativeDependencyState::Conservative
        };
        merge_native_dependencies(&mut dependencies, NativeDependencies::from([(path, state)]));
    }
    dependencies
}

/// Extracts the first rendered Solar error header without ANSI codes.
fn solar_error_summary(reason: &str) -> Option<String> {
    anstream::adapter::strip_str(reason)
        .to_string()
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("error:") || line.starts_with("error["))
        .map(str::to_string)
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
    fn constructor_call_options_are_preserved() {
        let (_root, paths, mut input) = input();
        input.input.sources.insert(
            PathBuf::from("src/Dep.sol"),
            Source::new("contract Dep { constructor() payable {} }"),
        );
        input.input.sources.insert(
            PathBuf::from("test/Deploy.sol"),
            Source::new("import '../src/Dep.sol'; contract Deploy { function deploy() public { new Dep{value: 1, salt: bytes32(7)}(); } }"),
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
        assert!(!source.contains("new Dep{value:"), "deployment was not rewritten");
        assert!(source.contains("_value: 1"), "value option was not preserved");
        assert!(source.contains("_salt: bytes32(7)"), "salt option was not preserved");
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

    #[test]
    fn analysis_fallback_tracks_transitive_imports() {
        let (_root, mut paths, mut input) = input();
        paths.remappings.push("dep/=lib/dep/".parse().unwrap());
        input.input.sources = [
            ("lib/dep/Leaf.sol", "contract Leaf {}"),
            ("src/Middle.sol", "import 'dep/Leaf.sol'; contract Middle {}"),
            ("src/Unrelated.sol", "contract Unrelated {}"),
            ("test/Fallback.sol", "import '../src/Middle.sol'; contract Fallback is Missing {}"),
            ("test/Independent.sol", "contract Independent {}"),
            ("script/Deploy.sol", "import '../src/Middle.sol'; contract Deploy {}"),
        ]
        .into_iter()
        .map(|(path, source)| (PathBuf::from(path), Source::new(source)))
        .collect();
        for (path, source) in &input.input.sources {
            let path = paths.root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source.content.as_str()).unwrap();
        }
        let original = input.input.sources.clone();
        let mut mocks = HashSet::from([paths.root.join("test/Fallback.sol")]);
        <DynamicTestLinkingPreprocessor as Preprocessor<SolcCompiler>>::preprocess(
            &DynamicTestLinkingPreprocessor,
            &SolcCompiler::default(),
            &mut input,
            &paths,
            &mut mocks,
        )
        .unwrap();
        assert_eq!(input.input.sources, original);
        assert!(mocks.is_empty());
        let dependencies = native_import_dependencies(&paths, &input.input.sources);
        let imports = BTreeSet::from([
            paths.root.join("src/Middle.sol"),
            paths.root.join("lib/dep/Leaf.sol"),
        ]);
        assert_eq!(
            dependencies,
            NativeDependencies::from([
                (
                    paths.root.join("test/Fallback.sol"),
                    NativeDependencyState::Known(imports.clone())
                ),
                (paths.root.join("script/Deploy.sol"), NativeDependencyState::Known(imports)),
                (
                    paths.root.join("test/Independent.sol"),
                    NativeDependencyState::Known(BTreeSet::new())
                ),
            ])
        );
    }

    #[test]
    fn analysis_fallback_limits_incomplete_imports_to_consumers() {
        let (_root, paths, mut input) = input();
        input.input.sources = [
            ("src/AHealthy.sol", "contract AHealthy {}"),
            (
                "test/Fallback.sol",
                "import /* Solar's regex fallback cannot recover this */ '../src/AHealthy.sol'; contract Fallback { function fail() public { throw; } }",
            ),
            (
                "test/Healthy.sol",
                "import '../src/AHealthy.sol'; contract Healthy is AHealthy {}",
            ),
            ("test/Independent.sol", "contract Independent {}"),
        ]
        .into_iter()
        .map(|(path, source)| (PathBuf::from(path), Source::new(source)))
        .collect();
        for (path, source) in &input.input.sources {
            let path = paths.root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source.content.as_str()).unwrap();
        }
        let dependencies = native_import_dependencies(&paths, &input.input.sources);
        assert_eq!(
            dependencies,
            NativeDependencies::from([
                (paths.root.join("test/Fallback.sol"), NativeDependencyState::Conservative),
                (
                    paths.root.join("test/Healthy.sol"),
                    NativeDependencyState::Known(BTreeSet::from([paths
                        .root
                        .join("src/AHealthy.sol")]))
                ),
                (
                    paths.root.join("test/Independent.sol"),
                    NativeDependencyState::Known(BTreeSet::new())
                ),
            ])
        );
    }

    #[test]
    fn analysis_fallback_merges_normalized_graph_paths() {
        let (_root, mut paths, mut input) = input();
        paths.remappings.push("test/z/:src/=lib/alternate/".parse().unwrap());
        input.input.sources = [
            ("src/Dep.sol", "contract Dep {}"),
            ("lib/alternate/Dep.sol", "contract Dep {}"),
            ("test/A.sol", "import '../src/Dep.sol'; contract A is Dep {}"),
            ("test/z/../A.sol", "import '../src/Dep.sol'; contract A is Dep {}"),
        ]
        .into_iter()
        .map(|(path, source)| (PathBuf::from(path), Source::new(source)))
        .collect();
        for (path, source) in &input.input.sources {
            let path = paths.root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source.content.as_str()).unwrap();
        }
        let dependencies = native_import_dependencies(&paths, &input.input.sources);
        assert_eq!(
            dependencies,
            NativeDependencies::from([(
                paths.root.join("test/A.sol"),
                NativeDependencyState::Known(BTreeSet::from([
                    paths.root.join("src/Dep.sol"),
                    paths.root.join("lib/alternate/Dep.sol")
                ]))
            )])
        );
    }

    #[test]
    fn solar_error_summary_accepts_coded_and_colored_errors() {
        assert_eq!(
            solar_error_summary(
                "solar reported errors:\n\n\u{1b}[1;91merror\u{1b}[0m\u{1b}[1m: boom\u{1b}[0m"
            ),
            Some("error: boom".to_string())
        );
        assert_eq!(
            solar_error_summary("solar reported errors:\n\n\u{1b}[31merror[1234]: boom\u{1b}[0m"),
            Some("error[1234]: boom".to_string())
        );
        assert_eq!(
            solar_error_summary("solar reported errors:\n\nerror: boom"),
            Some("error: boom".to_string())
        );
        assert_eq!(solar_error_summary("solar reported 1 error"), None);
    }
}
