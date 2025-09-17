use super::eip712::Resolver;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{
    opts::{BuildOpts, configure_pcx_from_solc},
    utils::LoadConfig,
};
use foundry_common::{TYPE_BINDING_PREFIX, compile::ProjectCompiler, fs};
use foundry_compilers::{
    CompilerInput, Graph, Project,
    artifacts::{Source, Sources},
    multi::{MultiCompilerLanguage, MultiCompilerParser},
    solc::{SolcLanguage, SolcVersionedInput},
};
use foundry_config::Config;
use itertools::Itertools;
use path_slash::PathExt;
use rayon::prelude::*;
use semver::Version;
use solar::parse::{
    Parser as SolarParser,
    ast::{self, Arena, FunctionKind, Span, VarMut, interface::source_map::FileName, visit::Visit},
    interface::Session,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
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
        let target_path = config.root.join(self.out.as_ref().unwrap_or(&config.bind_json.out));
        std::fs::write(target_path, JSON_BINDINGS_PLACEHOLDER)?;

        let project = config.solar_project()?;
        let mut output = ProjectCompiler::new().compile(&project)?;

        // Read and preprocess sources to handle potentially invalid bindings.
        let mut sources = self.preprocess_sources(&mut output)?;

        // Insert empty bindings file.
        sources.insert(target_path.clone(), Source::new(JSON_BINDINGS_PLACEHOLDER));

        // Find structs and generate bindings.
        let structs_to_write =
            self.find_and_resolve_structs(&config, &project, &mut output, &target_path)?;

        // Write bindings.
        self.write_bindings(&structs_to_write, &target_path)?;

        Ok(())
    }

    /// Find structs, resolve conflicts, and prepare them for writing
    fn find_and_resolve_structs(
        &self,
        config: &Config,
        project: &Project,
        version: Version,
        sources: Sources,
        _target_path: &Path,
    ) -> Result<Vec<StructToWrite>> {
        let settings = config.solc_settings()?;
        let include = &config.bind_json.include;
        let exclude = &config.bind_json.exclude;
        let root = &config.root;

        let input = SolcVersionedInput::build(sources, settings, SolcLanguage::Solidity, version);

        let mut sess = Session::builder().with_stderr_emitter().build();
        sess.dcx.set_flags_mut(|flags| flags.track_diagnostics = false);
        let mut compiler = solar::sema::Compiler::new(sess);

        let mut structs_to_write = Vec::new();

        compiler.enter_mut(|compiler| -> Result<()> {
            // Set up the parsing context with the project paths, without adding the source files
            let mut pcx = compiler.parse();
            configure_pcx_from_solc(&mut pcx, project, &input, false);

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

                if let Ok(src_file) = compiler
                    .sess()
                    .source_map()
                    .new_source_file(path.clone(), source.content.as_str())
                {
                    target_files.insert(src_file.clone());
                    pcx.add_file(src_file);
                }
            }

            // Parse and resolve
            pcx.parse();
            let Ok(ControlFlow::Continue(())) = compiler.lower_asts() else { return Ok(()) };
            let gcx = compiler.gcx();
            let hir = &gcx.hir;
            let resolver = Resolver::new(gcx);
            for id in resolver.struct_ids() {
                if let Some(schema) = resolver.resolve_struct_eip712(id)
                    && let def = hir.strukt(id)
                    && let source = hir.source(def.source)
                    && target_files.contains(&source.file)
                    && let FileName::Real(path) = &source.file.name
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
            Ok(())
        })?;

        eyre::ensure!(compiler.sess().dcx.has_errors().is_ok(), "errors occurred");

        // Resolve import aliases and function names
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
    fn write_bindings(
        &self,
        structs_to_write: &[StructToWrite],
        target_path: &PathBuf,
    ) -> Result<()> {
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

        result.push_str("// Automatically generated by forge bind-json.\n\npragma solidity >=0.6.2 <0.9.0;\npragma experimental ABIEncoderV2;\n\n");

        for (path, names) in grouped_imports {
            writeln!(
                &mut result,
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
                &mut result,
                "    {}{} = \"{}\";",
                TYPE_BINDING_PREFIX, struct_to_write.name_in_fns, struct_to_write.schema
            )?;
        }

        // write serialization functions
        for struct_to_write in structs_to_write {
            write!(
                &mut result,
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
