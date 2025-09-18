use super::eip712::Resolver;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{opts::BuildOpts, utils::LoadConfig};
use foundry_common::{TYPE_BINDING_PREFIX, errors::convert_solar_errors, fs};
use foundry_compilers::{
    Graph, Project,
    artifacts::{Source, Sources},
    multi::{MultiCompilerLanguage, MultiCompilerParser},
    project::ProjectCompiler,
    solc::SolcLanguage,
};
use foundry_config::Config;
use itertools::Itertools;
use path_slash::PathExt;
use rayon::prelude::*;
use solar::parse::{
    ast::{self, FunctionKind, Span, VarMut, visit::Visit},
    interface::Session,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    ops::ControlFlow,
    path::{Path, PathBuf},
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
        let config = self.load_config()?;
        let project = config.solar_project()?;
        let target_path = config.root.join(self.out.as_ref().unwrap_or(&config.bind_json.out));

        // Read and preprocess sources.
        let sources = project.paths.read_input_files()?;
        let graph = Graph::<MultiCompilerParser>::resolve_sources(&project.paths, sources)?;

        // We only generate bindings for a single Solidity version to avoid conflicts.
        let (_, mut sources, _) = graph
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

        // Preprocess sources to handle potentially invalid bindings.
        self.preprocess_sources(&mut sources)?;

        // Insert empty bindings file.
        sources.insert(target_path.clone(), Source::new(JSON_BINDINGS_PLACEHOLDER));

        // Find structs and generate bindings.
        let structs_to_write = self.find_and_resolve_structs(&config, &project, sources)?;

        // Write bindings.
        self.write_bindings(&structs_to_write, &target_path)?;

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
    fn preprocess_sources(&self, sources: &mut Sources) -> Result<()> {
        let mut compiler =
            solar::sema::Compiler::new(Session::builder().with_stderr_emitter().build());
        let _ = compiler.enter_mut(|compiler| -> solar::interface::Result<()> {
            let mut pcx = compiler.parse();
            pcx.set_resolve_imports(false);
            for (path, source) in &*sources {
                if let Ok(source) =
                    pcx.sess.source_map().new_source_file(path.clone(), source.content.as_str())
                {
                    pcx.add_file(source);
                }
            }
            pcx.parse();

            let gcx = compiler.gcx();
            sources.par_iter_mut().try_for_each(|(path, source)| {
                let mut content = Arc::unwrap_or_clone(std::mem::take(&mut source.content));
                let mut visitor = PreprocessorVisitor::new();
                let ast = gcx.get_ast_source(path).unwrap().1.ast.as_ref().unwrap();
                let _ = visitor.visit_source_unit(ast);
                visitor.update(gcx.sess, &mut content);
                source.content = Arc::new(content);
                Ok(())
            })
        });
        convert_solar_errors(compiler.dcx())
    }

    /// Find structs, resolve conflicts, and prepare them for writing
    fn find_and_resolve_structs(
        &self,
        config: &Config,
        project: &Project,
        sources: Sources,
    ) -> Result<Vec<StructToWrite>> {
        let include = &config.bind_json.include;
        let exclude = &config.bind_json.exclude;
        let root = &config.root;

        let mut output = ProjectCompiler::with_sources(project, sources)?.compile()?;
        if output.has_compiler_errors() {
            eyre::bail!("{output}");
        }
        let compiler = output.parser_mut().solc_mut().compiler_mut();

        let mut structs_to_write = Vec::new();

        compiler.enter_mut(|compiler| {
            let exclude = |path: &Path| {
                if !include.is_empty() {
                    if !include.iter().any(|matcher| matcher.is_match(path)) {
                        return true;
                    }
                } else {
                    // Exclude library files by default
                    if project.paths.has_library_ancestor(path) {
                        return true;
                    }
                }

                if exclude.iter().any(|matcher| matcher.is_match(path)) {
                    return true;
                }

                false
            };

            // Resolve.
            if compiler.gcx().stage() < Some(solar::config::CompilerStage::Lowering) {
                let _ = compiler.lower_asts();
            }
            let gcx = compiler.gcx();
            let hir = &gcx.hir;
            let resolver = Resolver::new(gcx);
            for id in resolver.struct_ids() {
                if let Some(schema) = resolver.resolve_struct_eip712(id)
                    && let def = hir.strukt(id)
                    && let source = hir.source(def.source)
                    && let solar::interface::source_map::FileName::Real(path) = &source.file.name
                    && !exclude(path)
                {
                    structs_to_write.push(StructToWrite {
                        name: def.name.as_str().into(),
                        contract_name: def.contract.map(|id| hir.contract(id).name.as_str().into()),
                        path: path.strip_prefix(root).unwrap_or(path).to_path_buf(),
                        schema,
                        // will be filled later
                        import_alias: None,
                        name_in_fns: String::new(),
                    });
                }
            }
        });

        convert_solar_errors(compiler.dcx())?;

        // Resolve import aliases and function names.
        self.resolve_conflicts(&mut structs_to_write);

        Ok(structs_to_write)
    }

    /// We manage 2 namespaces for JSON bindings:
    ///   - Namespace of imported items. This includes imports of contracts containing structs and
    ///     structs defined at the file level.
    ///   - Namespace of struct names used in function names and schema_* variables.
    ///
    /// Both of those might contain conflicts, so we need to resolve them.
    fn resolve_conflicts(&self, structs_to_write: &mut [StructToWrite]) {
        // firstly, we resolve imported names conflicts
        // construct mapping name -> paths from which items with such name are imported
        let mut names_to_paths = BTreeMap::new();

        for s in structs_to_write.iter() {
            names_to_paths
                .entry(s.struct_or_contract_name())
                .or_insert_with(BTreeSet::new)
                .insert(s.path.as_path());
        }

        // now resolve aliases for names which need them and construct mapping (name, file) -> alias
        let mut aliases = BTreeMap::new();

        for (name, paths) in names_to_paths {
            if paths.len() <= 1 {
                continue; // no alias needed
            }

            for (i, path) in paths.into_iter().enumerate() {
                aliases
                    .entry(name.to_string())
                    .or_insert_with(BTreeMap::new)
                    .insert(path.to_path_buf(), format!("{name}_{i}"));
            }
        }

        for s in structs_to_write.iter_mut() {
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
    }

    /// Write the final bindings file
    fn write_bindings(&self, structs_to_write: &[StructToWrite], target_path: &Path) -> Result<()> {
        let mut result = String::new();

        // Write imports
        let mut grouped_imports = BTreeMap::new();
        for struct_to_write in structs_to_write {
            let item = struct_to_write.import_item();
            grouped_imports
                .entry(struct_to_write.path.as_path())
                .or_insert_with(BTreeSet::new)
                .insert(item);
        }

        result.push_str(
            "\
// Automatically @generated by `forge bind-json`.

pragma solidity >=0.6.2 <0.9.0;
pragma experimental ABIEncoderV2;

",
        );

        for (path, names) in grouped_imports {
            writeln!(
                result,
                "import {{{}}} from \"{}\";",
                names.iter().join(", "),
                path.to_slash_lossy()
            )?;
        }

        // Write VM interface
        // Writes minimal VM interface to not depend on forge-std version
        result.push_str(r#"
interface Vm {
    function parseJsonTypeArray(string calldata json, string calldata key, string calldata typeDescription) external pure returns (bytes memory);
    function parseJsonType(string calldata json, string calldata typeDescription) external pure returns (bytes memory);
    function parseJsonType(string calldata json, string calldata key, string calldata typeDescription) external pure returns (bytes memory);
    function serializeJsonType(string calldata typeDescription, bytes memory value) external pure returns (string memory json);
    function serializeJsonType(string calldata objectKey, string calldata valueKey, string calldata typeDescription, bytes memory value) external returns (string memory json);
}
        "#);

        // Write library
        result.push_str(
            r#"
library JsonBindings {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

"#,
        );

        // write schema constants
        for struct_to_write in structs_to_write {
            writeln!(
                result,
                "    {}{} = \"{}\";",
                TYPE_BINDING_PREFIX, struct_to_write.name_in_fns, struct_to_write.schema
            )?;
        }

        // write serialization functions
        for struct_to_write in structs_to_write {
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

        // Write to file
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target_path, &result)?;

        sh_println!("Bindings written to {}", target_path.display())?;

        Ok(())
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

// TODO(dani): AST mut visitor?
impl<'ast> Visit<'ast> for PreprocessorVisitor {
    type BreakValue = solar::interface::data_structures::Never;

    fn visit_item_function(
        &mut self,
        func: &'ast ast::ItemFunction<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        // Replace function bodies with a noop statement.
        if let Some(block) = &func.body
            && !block.is_empty()
        {
            let span = block.first().unwrap().span.to(block.last().unwrap().span);
            let new_body = match func.kind {
                FunctionKind::Modifier => "_;",
                _ => "revert();",
            };
            self.updates.push((span, new_body));
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
    /// StructName or ParentName.StructName
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
