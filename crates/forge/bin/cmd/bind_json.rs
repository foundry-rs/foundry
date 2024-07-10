use super::eip712::Resolver;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{opts::CoreBuildArgs, utils::LoadConfig};
use foundry_common::{compile::with_compilation_reporter, fs};
use foundry_compilers::{
    artifacts::{
        output_selection::OutputSelection, ContractDefinitionPart, Source, SourceUnit,
        SourceUnitPart, Sources,
    },
    multi::{MultiCompilerLanguage, MultiCompilerParsedSource},
    project::ProjectCompiler,
    solc::SolcLanguage,
    CompilerSettings, Graph, Project,
};
use foundry_config::Config;
use itertools::Itertools;
use rayon::prelude::*;
use solang_parser::pt as solang_ast;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fmt::Write,
    path::PathBuf,
    sync::Arc,
};

foundry_config::impl_figment_convert!(BindJsonArgs, opts);

/// CLI arguments for `forge bind-json`.
#[derive(Clone, Debug, Parser)]
pub struct BindJsonArgs {
    /// The path to write bindings to.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub out: Option<PathBuf>,

    #[command(flatten)]
    opts: CoreBuildArgs,
}

impl BindJsonArgs {
    pub fn run(self) -> Result<()> {
        self.preprocess()?.compile()?.find_structs()?.resolve_imports_and_aliases().write()?;

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
        let config = self.try_load_config_emit_warnings()?;
        let project = config.create_project(false, true)?;

        let target_path = config.root.0.join(self.out.as_ref().unwrap_or(&config.bind_json.out));

        let sources = project.paths.read_input_files()?;
        let graph = Graph::<MultiCompilerParsedSource>::resolve_sources(&project.paths, sources)?;

        // We only generate bindings for a single Solidity version to avoid conflicts.
        let mut sources = graph
            // resolve graph into mapping language -> version -> sources
            .into_sources_by_version(project.offline, &project.locked_versions, &project.compiler)?
            .0
            .into_iter()
            // we are only interested in Solidity sources
            .find(|(lang, _)| *lang == MultiCompilerLanguage::Solc(SolcLanguage::Solidity))
            .ok_or_else(|| eyre::eyre!("no Solidity sources"))?
            .1
            .into_iter()
            // For now, we are always picking the latest version.
            .max_by(|(v1, _), (v2, _)| v1.cmp(v2))
            .unwrap()
            .1;

        // Insert empty bindings file
        sources.insert(target_path.clone(), Source::new("library JsonBindings {}"));

        let sources = Sources(
            sources
                .0
                .into_par_iter()
                .map(|(path, source)| {
                    let mut locs_to_update = Vec::new();
                    let mut content = Arc::unwrap_or_clone(source.content);
                    let (parsed, _) = solang_parser::parse(&content, 0)
                        .map_err(|errors| eyre::eyre!("Parser failed: {errors:?}"))?;

                    // All function definitions in the file
                    let mut functions = Vec::new();

                    for part in &parsed.0 {
                        if let solang_ast::SourceUnitPart::FunctionDefinition(def) = part {
                            functions.push(def);
                        }
                        if let solang_ast::SourceUnitPart::ContractDefinition(contract) = part {
                            for part in &contract.parts {
                                match part {
                                    solang_ast::ContractPart::FunctionDefinition(def) => {
                                        functions.push(def);
                                    }
                                    // Remove `immutable` attributes
                                    solang_ast::ContractPart::VariableDefinition(def) => {
                                        for attr in &def.attrs {
                                            if let solang_ast::VariableAttribute::Immutable(loc) =
                                                attr
                                            {
                                                locs_to_update.push((
                                                    loc.start(),
                                                    loc.end(),
                                                    String::new(),
                                                ));
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        };
                    }

                    for def in functions {
                        // If there's no body block, keep the function as is
                        let Some(solang_ast::Statement::Block { loc, .. }) = def.body else {
                            continue;
                        };
                        let new_body = match def.ty {
                            solang_ast::FunctionTy::Modifier => "{ _; }",
                            _ => "{ revert(); }",
                        };
                        let start = loc.start();
                        let end = loc.end();
                        locs_to_update.push((start, end + 1, new_body.to_string()));
                    }

                    locs_to_update.sort_by_key(|(start, _, _)| *start);

                    let mut shift = 0_i64;

                    for (start, end, new) in locs_to_update {
                        let start = ((start as i64) - shift) as usize;
                        let end = ((end as i64) - shift) as usize;

                        content.replace_range(start..end, new.as_str());
                        shift += (end - start) as i64;
                        shift -= new.len() as i64;
                    }

                    Ok((path, Source::new(content)))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        );

        Ok(PreprocessedState { sources, target_path, project, config })
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
    /// Returns the name of the imported item. If struct is definied at the file level, returns the
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

#[derive(Debug)]
struct PreprocessedState {
    sources: Sources,
    target_path: PathBuf,
    project: Project,
    config: Config,
}

impl PreprocessedState {
    fn compile(self) -> Result<CompiledState> {
        let Self { sources, target_path, mut project, config } = self;

        project.settings.update_output_selection(|selection| {
            *selection = OutputSelection::ast_output_selection();
        });

        let output = with_compilation_reporter(false, || {
            ProjectCompiler::with_sources(&project, sources)?.compile()
        })?;

        if output.has_compiler_errors() {
            eyre::bail!("{output}");
        }

        // Collect ASTs by getting them from sources and converting into strongly typed
        // `SourceUnit`s. Also strips root from paths.
        let asts = output
            .into_output()
            .sources
            .into_iter()
            .filter_map(|(path, mut sources)| Some((path, sources.swap_remove(0).source_file.ast?)))
            .map(|(path, ast)| {
                Ok((
                    path.strip_prefix(project.root()).unwrap_or(&path).to_path_buf(),
                    serde_json::from_str::<SourceUnit>(&serde_json::to_string(&ast)?)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        Ok(CompiledState { asts, target_path, config, project })
    }
}

#[derive(Debug, Clone)]
struct CompiledState {
    asts: BTreeMap<PathBuf, SourceUnit>,
    target_path: PathBuf,
    config: Config,
    project: Project,
}

impl CompiledState {
    fn find_structs(self) -> Result<StructsState> {
        let Self { asts, target_path, config, project } = self;

        // construct mapping (file, id) -> (struct definition, optional parent contract name)
        let structs = asts
            .iter()
            .flat_map(|(path, ast)| {
                let mut structs = Vec::new();
                // we walk AST directly instead of using visitors because we need to distinguish
                // between file-level and contract-level struct definitions
                for node in &ast.nodes {
                    match node {
                        SourceUnitPart::StructDefinition(def) => {
                            structs.push((def, None));
                        }
                        SourceUnitPart::ContractDefinition(contract) => {
                            for node in &contract.nodes {
                                if let ContractDefinitionPart::StructDefinition(def) = node {
                                    structs.push((def, Some(contract.name.clone())));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                structs.into_iter().map(|(def, parent)| ((path.as_path(), def.id), (def, parent)))
            })
            .collect::<BTreeMap<_, _>>();

        // Resolver for EIP712 schemas
        let resolver = Resolver::new(&asts);

        let mut structs_to_write = Vec::new();

        let include = config.bind_json.include;
        let exclude = config.bind_json.exclude;

        for ((path, id), (def, contract_name)) in structs {
            // For some structs there's no schema (e.g. if they contain a mapping), so we just skip
            // those.
            let Some(schema) = resolver.resolve_struct_eip712(id, &mut Default::default(), true)?
            else {
                continue
            };

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

            structs_to_write.push(StructToWrite {
                name: def.name.clone(),
                contract_name,
                path: path.to_path_buf(),
                schema,

                // will be filled later
                import_alias: None,
                name_in_fns: String::new(),
            })
        }

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

        println!("Bindings written to {}", self.target_path.display());

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
                path.display()
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
                "    string constant schema_{} = \"{}\";",
                struct_to_write.name_in_fns, struct_to_write.schema
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
