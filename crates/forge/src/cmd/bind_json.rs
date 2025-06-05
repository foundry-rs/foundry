use super::eip712::Resolver;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{
    opts::{solar_pcx_from_solc_project, BuildOpts},
    utils::LoadConfig,
};
use foundry_common::{fs, TYPE_BINDING_PREFIX};
use foundry_compilers::{
    artifacts::{Source, Sources},
    multi::{MultiCompilerLanguage, MultiCompilerParsedSource},
    solc::{SolcLanguage, SolcVersionedInput},
    CompilerInput, Graph, Project,
};
use foundry_config::Config;
use itertools::Itertools;
use path_slash::PathExt;
use rayon::prelude::*;
use semver::Version;
use solar_parse::{
    ast::{self, interface::source_map::FileName, visit::Visit, Arena, FunctionKind, Span, VarMut},
    interface::Session,
    Parser as SolarParser,
};
use solar_sema::thread_local::ThreadLocal;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt::{self, Write},
    ops::ControlFlow,
    path::PathBuf,
    sync::Arc,
};

foundry_config::impl_figment_convert!(BindJsonArgs, build);

const JSON_BINDINGS_PLACEHOLDER: &str = "library JsonBindings {}";

/// CLI arguments for `forge bind-json`.
#[derive(Clone, Debug, Parser)]
pub struct BindJsonArgs {
    /// The path to write bindings to.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub out: Option<PathBuf>,

    #[command(flatten)]
    build: BuildOpts,
}

impl BindJsonArgs {
    pub fn run(self) -> Result<()> {
        self.preprocess()?.find_structs()?.resolve_imports_and_aliases().write()?;

        Ok(())
    }

    /// In cases when user moves/renames/deletes structs, compiler will start failing because
    /// generated bindings will be referencing non-existing structs or importing non-existing
    /// files.
    ///
    /// Because of that, we need a little bit of preprocessing to make sure that bindings will still
    /// be valid.
    ///
    /// The strategy is:
    /// 1. Replace bindings file with an empty one to get rid of potentially invalid imports.
    /// 2. Remove all function bodies to get rid of `serialize`/`deserialize` invocations.
    /// 3. Remove all `immutable` attributes to avoid errors because of erased constructors
    ///    initializing them.
    ///
    /// After that we'll still have enough information for bindings but compilation should succeed
    /// in most of the cases.
    fn preprocess(self) -> Result<PreprocessedState> {
        let config = self.load_config()?;
        let project = config.ephemeral_project()?;

        let target_path = config.root.join(self.out.as_ref().unwrap_or(&config.bind_json.out));

        let sources = project.paths.read_input_files()?;
        let graph = Graph::<MultiCompilerParsedSource>::resolve_sources(&project.paths, sources)?;

        // We only generate bindings for a single Solidity version to avoid conflicts.
        let (version, mut sources, _) = graph
            // resolve graph into mapping language -> version -> sources
            .into_sources_by_version(&project)?
            .sources
            .into_iter()
            // we are only interested in Solidity sources
            .find(|(lang, _)| *lang == MultiCompilerLanguage::Solc(SolcLanguage::Solidity))
            .ok_or_else(|| eyre::eyre!("no Solidity sources"))?
            .1
            .into_iter()
            // For now, we are always picking the latest version.
            .max_by(|(v1, _, _), (v2, _, _)| v1.cmp(v2))
            .unwrap();

        let sess = Session::builder().with_stderr_emitter().build();
        let result = sess.enter_parallel(|| -> solar_parse::interface::Result<()> {
            sources.0.par_iter_mut().try_for_each(|(path, source)| {
                let mut content = Arc::try_unwrap(std::mem::take(&mut source.content)).unwrap();

                let arena = Arena::new();
                let mut parser = SolarParser::from_source_code(
                    &sess,
                    &arena,
                    FileName::Real(path.clone()),
                    content.to_string(),
                )?;
                let ast = parser.parse_file().map_err(|e| e.emit())?;

                let mut visitor = PreprocessorVisitor::new();
                let _ = visitor.visit_source_unit(&ast);
                visitor.update(&sess, &mut content);

                source.content = Arc::new(content);
                Ok(())
            })
        });
        eyre::ensure!(result.is_ok(), "failed parsing");

        // Insert empty bindings file.
        sources.insert(target_path.clone(), Source::new(JSON_BINDINGS_PLACEHOLDER));

        Ok(PreprocessedState { version, sources, target_path, project, config })
    }
}

struct PreprocessorVisitor {
    updates: Vec<(Span, &'static str)>,
}

impl PreprocessorVisitor {
    fn new() -> Self {
        Self { updates: Vec::new() }
    }

    fn update(mut self, sess: &Session, content: &mut String) {
        if self.updates.is_empty() {
            return;
        }

        let sf = sess.source_map().lookup_source_file(self.updates[0].0.lo());
        let base = sf.start_pos.0;

        self.updates.sort_by_key(|(span, _)| span.lo());
        let mut shift = 0_i64;
        for (span, new) in self.updates {
            let lo = span.lo() - base;
            let hi = span.hi() - base;
            let start = ((lo.0 as i64) - shift) as usize;
            let end = ((hi.0 as i64) - shift) as usize;

            content.replace_range(start..end, new);
            shift += (end - start) as i64;
            shift -= new.len() as i64;
        }
    }
}

impl<'ast> Visit<'ast> for PreprocessorVisitor {
    type BreakValue = solar_parse::interface::data_structures::Never;

    fn visit_item_function(
        &mut self,
        func: &'ast ast::ItemFunction<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        // Replace function bodies with a noop statement.
        if let Some(block) = &func.body {
            if !block.is_empty() {
                let span = block.first().unwrap().span.to(block.last().unwrap().span);
                let new_body = match func.kind {
                    FunctionKind::Modifier => "_;",
                    _ => "revert();",
                };
                self.updates.push((span, new_body));
            }
        }

        self.walk_item_function(func)
    }

    fn visit_variable_definition(
        &mut self,
        var: &'ast ast::VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        // Remove `immutable` attributes.
        if let Some(VarMut::Immutable) = var.mutability {
            self.updates.push((var.span, ""));
        }

        self.walk_variable_definition(var)
    }
}

/// A single struct definition for which we need to generate bindings.
#[derive(Debug, Clone)]
struct StructToWrite {
    /// Name of the struct definition.
    name: String,
    /// Name of the contract containing the struct definition. None if the struct is defined at the
    /// file level.
    contract_name: Option<String>,
    /// Import alias for the contract or struct, depending on whether the struct is imported
    /// directly, or via a contract.
    import_alias: Option<String>,
    /// Path to the file containing the struct definition.
    path: PathBuf,
    /// EIP712 schema for the struct.
    schema: String,
    /// Name of the struct definition used in function names and schema_* variables.
    name_in_fns: String,
}

impl StructToWrite {
    /// Returns the name of the imported item. If struct is defined at the file level, returns the
    /// struct name, otherwise returns the parent contract name.
    fn struct_or_contract_name(&self) -> &str {
        self.contract_name.as_deref().unwrap_or(&self.name)
    }

    /// Same as [StructToWrite::struct_or_contract_name] but with alias applied.
    fn struct_or_contract_name_with_alias(&self) -> &str {
        self.import_alias.as_deref().unwrap_or(self.struct_or_contract_name())
    }

    /// Path which can be used to reference this struct in input/output parameters. Either
    /// StructName or ParantName.StructName
    fn full_path(&self) -> String {
        if self.contract_name.is_some() {
            format!("{}.{}", self.struct_or_contract_name_with_alias(), self.name)
        } else {
            self.struct_or_contract_name_with_alias().to_string()
        }
    }

    fn import_item(&self) -> String {
        if let Some(alias) = &self.import_alias {
            format!("{} as {}", self.struct_or_contract_name(), alias)
        } else {
            self.struct_or_contract_name().to_string()
        }
    }
}

struct PreprocessedState {
    version: Version,
    sources: Sources,
    target_path: PathBuf,
    project: Project,
    config: Config,
}

impl PreprocessedState {
    fn find_structs(self) -> Result<StructsState> {
        let mut structs_to_write = Vec::new();
        let Self { version, sources, target_path, config, project } = self;

        let settings = config.solc_settings()?;
        let include = config.bind_json.include;
        let exclude = config.bind_json.exclude;
        let root = config.root;

        let input = SolcVersionedInput::build(sources, settings, SolcLanguage::Solidity, version);

        let mut sess = Session::builder().with_stderr_emitter().build();
        sess.dcx = sess.dcx.set_flags(|flags| flags.track_diagnostics = false);

        let result = sess.enter_parallel(|| -> Result<()> {
            // Set up the parsing context with the project paths, without adding the source files
            let mut parsing_context = solar_pcx_from_solc_project(&sess, &project, &input, false);

            let mut target_files = HashSet::new();
            for (path, source) in &input.input.sources {
                if !include.is_empty() {
                    if !include.iter().any(|matcher| matcher.is_match(path)) {
                        continue;
                    }
                } else {
                    // Exclude library files by default
                    if project.paths.has_library_ancestor(path) {
                        continue;
                    }
                }

                if exclude.iter().any(|matcher| matcher.is_match(path)) {
                    continue;
                }

                if let Ok(src_file) =
                    sess.source_map().new_source_file(path.clone(), source.content.as_str())
                {
                    target_files.insert(src_file.stable_id);
                    parsing_context.add_file(src_file);
                }
            }

            // Parse and resolve
            let hir_arena = ThreadLocal::new();
            if let Ok(Some(gcx)) = parsing_context.parse_and_lower(&hir_arena) {
                let hir = &gcx.get().hir;
                let resolver = Resolver::new(gcx);
                for id in &resolver.struct_ids() {
                    if let Some(schema) = resolver.resolve_struct_eip712(*id) {
                        let def = hir.strukt(*id);
                        let source = hir.source(def.source);

                        if !target_files.contains(&source.file.stable_id) {
                            continue;
                        }

                        if let FileName::Real(ref path) = source.file.name {
                            structs_to_write.push(StructToWrite {
                                name: def.name.as_str().into(),
                                contract_name: def
                                    .contract
                                    .map(|id| hir.contract(id).name.as_str().into()),
                                path: path
                                    .strip_prefix(&root)
                                    .unwrap_or_else(|_| path)
                                    .to_path_buf(),
                                schema,

                                // will be filled later
                                import_alias: None,
                                name_in_fns: String::new(),
                            });
                        }
                    }
                }
            }
            Ok(())
        });

        eyre::ensure!(result.is_ok() && sess.dcx.has_errors().is_ok(), "failed parsing");

        Ok(StructsState { structs_to_write, target_path })
    }
}

#[derive(Debug)]
struct StructsState {
    structs_to_write: Vec<StructToWrite>,
    target_path: PathBuf,
}

impl StructsState {
    /// We manage 2 namespsaces for JSON bindings:
    ///   - Namespace of imported items. This includes imports of contracts containing structs and
    ///     structs defined at the file level.
    ///   - Namespace of struct names used in function names and schema_* variables.
    ///
    /// Both of those might contain conflicts, so we need to resolve them.
    fn resolve_imports_and_aliases(self) -> ResolvedState {
        let Self { mut structs_to_write, target_path } = self;

        // firstly, we resolve imported names conflicts
        // construct mapping name -> paths from which items with such name are imported
        let mut names_to_paths = BTreeMap::new();

        for s in &structs_to_write {
            names_to_paths
                .entry(s.struct_or_contract_name())
                .or_insert_with(BTreeSet::new)
                .insert(s.path.as_path());
        }

        // now resolve aliases for names which need them and construct mapping (name, file) -> alias
        let mut aliases = BTreeMap::new();

        for (name, paths) in names_to_paths {
            if paths.len() <= 1 {
                // no alias needed
                continue
            }

            for (i, path) in paths.into_iter().enumerate() {
                aliases
                    .entry(name.to_string())
                    .or_insert_with(BTreeMap::new)
                    .insert(path.to_path_buf(), format!("{name}_{i}"));
            }
        }

        for s in &mut structs_to_write {
            let name = s.struct_or_contract_name();
            if aliases.contains_key(name) {
                s.import_alias = Some(aliases[name][&s.path].clone());
            }
        }

        // Each struct needs a name by which we are referencing it in function names (e.g.
        // deserializeFoo) Those might also have conflicts, so we manage a separate
        // namespace for them
        let mut name_to_structs_indexes = BTreeMap::new();

        for (idx, s) in structs_to_write.iter().enumerate() {
            name_to_structs_indexes.entry(&s.name).or_insert_with(Vec::new).push(idx);
        }

        // Keeps `Some` for structs that will be referenced by name other than their definition
        // name.
        let mut fn_names = vec![None; structs_to_write.len()];

        for (name, indexes) in name_to_structs_indexes {
            if indexes.len() > 1 {
                for (i, idx) in indexes.into_iter().enumerate() {
                    fn_names[idx] = Some(format!("{name}_{i}"));
                }
            }
        }

        for (s, fn_name) in structs_to_write.iter_mut().zip(fn_names.into_iter()) {
            s.name_in_fns = fn_name.unwrap_or(s.name.clone());
        }

        ResolvedState { structs_to_write, target_path }
    }
}

struct ResolvedState {
    structs_to_write: Vec<StructToWrite>,
    target_path: PathBuf,
}

impl ResolvedState {
    fn write(self) -> Result<String> {
        let mut result = String::new();
        self.write_imports(&mut result)?;
        self.write_vm(&mut result);
        self.write_library(&mut result)?;

        if let Some(parent) = self.target_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.target_path, &result)?;

        sh_println!("Bindings written to {}", self.target_path.display())?;

        Ok(result)
    }

    fn write_imports(&self, result: &mut String) -> fmt::Result {
        let mut grouped_imports = BTreeMap::new();

        for struct_to_write in &self.structs_to_write {
            let item = struct_to_write.import_item();
            grouped_imports
                .entry(struct_to_write.path.as_path())
                .or_insert_with(BTreeSet::new)
                .insert(item);
        }

        result.push_str("// Automatically generated by forge bind-json.\n\npragma solidity >=0.6.2 <0.9.0;\npragma experimental ABIEncoderV2;\n\n");

        for (path, names) in grouped_imports {
            writeln!(
                result,
                "import {{{}}} from \"{}\";",
                names.iter().join(", "),
                path.to_slash_lossy()
            )?;
        }

        Ok(())
    }

    /// Writes minimal VM interface to not depend on forge-std version
    fn write_vm(&self, result: &mut String) {
        result.push_str(r#"
interface Vm {
    function parseJsonTypeArray(string calldata json, string calldata key, string calldata typeDescription) external pure returns (bytes memory);
    function parseJsonType(string calldata json, string calldata typeDescription) external pure returns (bytes memory);
    function parseJsonType(string calldata json, string calldata key, string calldata typeDescription) external pure returns (bytes memory);
    function serializeJsonType(string calldata typeDescription, bytes memory value) external pure returns (string memory json);
    function serializeJsonType(string calldata objectKey, string calldata valueKey, string calldata typeDescription, bytes memory value) external returns (string memory json);
}
        "#);
    }

    fn write_library(&self, result: &mut String) -> fmt::Result {
        result.push_str(
            r#"
library JsonBindings {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

"#,
        );
        // write schema constants
        for struct_to_write in &self.structs_to_write {
            writeln!(
                result,
                "    {}{} = \"{}\";",
                TYPE_BINDING_PREFIX, struct_to_write.name_in_fns, struct_to_write.schema
            )?;
        }

        // write serialization functions
        for struct_to_write in &self.structs_to_write {
            write!(
                result,
                r#"
    function serialize({path} memory value) internal pure returns (string memory) {{
        return vm.serializeJsonType(schema_{name_in_fns}, abi.encode(value));
    }}

    function serialize({path} memory value, string memory objectKey, string memory valueKey) internal returns (string memory) {{
        return vm.serializeJsonType(objectKey, valueKey, schema_{name_in_fns}, abi.encode(value));
    }}

    function deserialize{name_in_fns}(string memory json) public pure returns ({path} memory) {{
        return abi.decode(vm.parseJsonType(json, schema_{name_in_fns}), ({path}));
    }}

    function deserialize{name_in_fns}(string memory json, string memory path) public pure returns ({path} memory) {{
        return abi.decode(vm.parseJsonType(json, path, schema_{name_in_fns}), ({path}));
    }}

    function deserialize{name_in_fns}Array(string memory json, string memory path) public pure returns ({path}[] memory) {{
        return abi.decode(vm.parseJsonTypeArray(json, path, schema_{name_in_fns}), ({path}[]));
    }}
"#,
                name_in_fns = struct_to_write.name_in_fns,
                path = struct_to_write.full_path()
            )?;
        }

        result.push_str("}\n");

        Ok(())
    }
}
