use clap::{Parser, ValueHint};
use eyre::{Ok, OptionExt, Result};
use foundry_cli::{opts::CoreBuildArgs, utils::LoadConfig};
use foundry_common::compile::ProjectCompiler;
use foundry_compilers::{
    artifacts::{
        output_selection::OutputSelection,
        visitor::{Visitor, Walk},
        ContractDefinition, EnumDefinition, SourceUnit, StructDefinition, TypeDescriptions,
        TypeName,
    },
    CompilerSettings,
};
use std::{collections::BTreeMap, path::PathBuf};

foundry_config::impl_figment_convert!(Eip712Args, opts);

/// CLI arguments for `forge eip712`.
#[derive(Clone, Debug, Parser)]
pub struct Eip712Args {
    /// The path to the file from which to read struct definitions.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub target_path: PathBuf,

    #[command(flatten)]
    opts: CoreBuildArgs,
}

impl Eip712Args {
    pub fn run(self) -> Result<()> {
        let config = self.try_load_config_emit_warnings()?;
        let mut project = config.create_project(false, true)?;
        let target_path = dunce::canonicalize(self.target_path)?;
        project.settings.update_output_selection(|selection| {
            *selection = OutputSelection::ast_output_selection();
        });

        let output = ProjectCompiler::new().files([target_path.clone()]).compile(&project)?;

        // Collect ASTs by getting them from sources and converting into strongly typed
        // `SourceUnit`s.
        let asts = output
            .into_output()
            .sources
            .into_iter()
            .filter_map(|(path, mut sources)| Some((path, sources.swap_remove(0).source_file.ast?)))
            .map(|(path, ast)| {
                Ok((path, serde_json::from_str::<SourceUnit>(&serde_json::to_string(&ast)?)?))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        let resolver = Resolver::new(&asts);

        let target_ast = asts
            .get(&target_path)
            .ok_or_else(|| eyre::eyre!("Could not find AST for target file {target_path:?}"))?;

        let structs_in_target = {
            let mut collector = StructCollector::default();
            target_ast.walk(&mut collector);
            collector.0
        };

        for (id, _) in structs_in_target {
            if let Some(resolved) =
                resolver.resolve_struct_eip712(id, &mut Default::default(), true)?
            {
                println!("{resolved}");
                println!();
            }
        }

        Ok(())
    }
}

/// AST [Visitor] used for collecting struct definitions.
#[derive(Debug, Clone, Default)]
pub struct StructCollector(pub BTreeMap<usize, StructDefinition>);

impl Visitor for StructCollector {
    fn visit_struct_definition(&mut self, def: &StructDefinition) {
        self.0.insert(def.id, def.clone());
    }
}

/// Collects mapping from AST id of type definition to representation of this type for EIP-712
/// encoding.
///
/// For now, maps contract definitions to `address` and enums to `uint8`.
#[derive(Debug, Clone, Default)]
struct SimpleCustomTypesCollector(BTreeMap<usize, String>);

impl Visitor for SimpleCustomTypesCollector {
    fn visit_contract_definition(&mut self, def: &ContractDefinition) {
        self.0.insert(def.id, "address".to_string());
    }

    fn visit_enum_definition(&mut self, def: &EnumDefinition) {
        self.0.insert(def.id, "uint8".to_string());
    }
}

pub struct Resolver {
    simple_types: BTreeMap<usize, String>,
    structs: BTreeMap<usize, StructDefinition>,
}

impl Resolver {
    pub fn new(asts: &BTreeMap<PathBuf, SourceUnit>) -> Self {
        let simple_types = {
            let mut collector = SimpleCustomTypesCollector::default();
            asts.values().for_each(|ast| ast.walk(&mut collector));

            collector.0
        };

        let structs = {
            let mut collector = StructCollector::default();
            asts.values().for_each(|ast| ast.walk(&mut collector));
            collector.0
        };

        Self { simple_types, structs }
    }

    /// Converts a given struct definition into EIP-712 `encodeType` representation.
    ///
    /// Returns `None` if struct contains any fields that are not supported by EIP-712 (e.g.
    /// mappings or function pointers).
    pub fn resolve_struct_eip712(
        &self,
        id: usize,
        subtypes: &mut BTreeMap<String, usize>,
        append_subtypes: bool,
    ) -> Result<Option<String>> {
        let def = &self.structs[&id];
        let mut result = format!("{}(", def.name);

        for (idx, member) in def.members.iter().enumerate() {
            let Some(ty) = self.resolve_type(
                member.type_name.as_ref().ok_or_eyre("missing type name")?,
                subtypes,
            )?
            else {
                return Ok(None)
            };

            result.push_str(&ty);
            result.push(' ');
            result.push_str(&member.name);

            if idx < def.members.len() - 1 {
                result.push(',');
            }
        }

        result.push(')');

        if !append_subtypes {
            return Ok(Some(result))
        }

        for subtype_id in subtypes.values().copied().collect::<Vec<_>>() {
            if subtype_id == id {
                continue
            }
            let Some(encoded_subtype) = self.resolve_struct_eip712(subtype_id, subtypes, false)?
            else {
                return Ok(None)
            };
            result.push_str(&encoded_subtype);
        }

        Ok(Some(result))
    }

    /// Converts given [TypeName] into a type which can be converted to [DynSolType].
    ///
    /// Returns `None` if the type is not supported for EIP712 encoding.
    pub fn resolve_type(
        &self,
        type_name: &TypeName,
        subtypes: &mut BTreeMap<String, usize>,
    ) -> Result<Option<String>> {
        match type_name {
            TypeName::FunctionTypeName(_) | TypeName::Mapping(_) => Ok(None),
            TypeName::ElementaryTypeName(ty) => Ok(Some(ty.name.clone())),
            TypeName::ArrayTypeName(ty) => {
                let Some(inner) = self.resolve_type(&ty.base_type, subtypes)? else {
                    return Ok(None)
                };
                let len = parse_array_length(&ty.type_descriptions)?;

                Ok(Some(format!("{inner}[{}]", len.unwrap_or(""))))
            }
            TypeName::UserDefinedTypeName(ty) => {
                if let Some(name) = self.simple_types.get(&(ty.referenced_declaration as usize)) {
                    Ok(Some(name.clone()))
                } else if let Some(def) = self.structs.get(&(ty.referenced_declaration as usize)) {
                    let name =
                        // If we've already seen struct with this ID, just use assigned name.
                        if let Some((name, _)) = subtypes.iter().find(|(_, id)| **id == def.id) {
                            name.clone()
                        // Otherwise, try assigning a new name.
                        } else {
                            let mut i = 0;
                            let mut name = def.name.clone();
                            while subtypes.contains_key(&name) {
                                i += 1;
                                name = format!("{}_{i}", def.name);
                            }

                            subtypes.insert(name.clone(), def.id);
                            name
                        };

                    return Ok(Some(name))
                } else {
                    return Ok(None)
                }
            }
        }
    }
}

fn parse_array_length(type_description: &TypeDescriptions) -> Result<Option<&str>> {
    let type_string =
        type_description.type_string.as_ref().ok_or_eyre("missing typeString for array type")?;
    let Some(inside_brackets) =
        type_string.rsplit_once("[").and_then(|(_, right)| right.split("]").next())
    else {
        eyre::bail!("failed to parse array type string: {type_string}")
    };

    if inside_brackets.is_empty() {
        Ok(None)
    } else {
        Ok(Some(inside_brackets))
    }
}
