//! Functions, modifiers, and getters of a Solar AST, found by the offset where solc's source map
//! places their definition.
//!
//! solc's `functionDebugData` lists an internal function by the offset of its entry `JUMPDEST`,
//! and the source map puts that instruction at the start of the function's definition. The Solar
//! AST that Foundry parses for every source file holds the definition there, with the contract it
//! is in, its name, and its parameters and their types.

use super::Param;
use alloy_dyn_abi::{DynSolType, parser::Storage};
use solar::{
    ast::{
        DataLocation, ElementaryType, ExprKind, FunctionKind, ImportItems, Item, ItemKind, LitKind,
        SourceUnit, Type, TypeKind, VariableDefinition, Visibility,
        visit::Visit,
        yul::{self, StmtKind as YulStmtKind},
    },
    data_structures::Never,
    interface::{Ident, source_map::SourceMap},
};
use std::{collections::HashMap, ops::ControlFlow, sync::Arc};

/// The user-defined types of the sources by name, or `None` for a name that two types share.
pub type UserTypes = HashMap<String, Option<UserType>>;

/// A user-defined type and how it appears in function signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserType {
    /// A contract, interface, or library, which the ABI treats as an address.
    Address,
    /// An enum, which the ABI treats as `uint8`.
    Enum,
    /// A user-defined value type of an elementary type.
    Value(DynSolType),
    /// A struct, which does not have a single stack word to decode.
    Struct,
}

/// A function, modifier, public state variable getter, or Yul function.
#[derive(Clone, Debug)]
pub struct SourceFunction {
    /// The contract that defines it. Free functions have none.
    pub contract: Option<String>,
    /// The name of the function, modifier, or variable.
    pub name: String,
    /// Whether the dispatcher or the call frame itself enters it, which is then the external call
    /// that the trace shows already, such as a public function or getter.
    pub is_entry: bool,
    /// Whether it is a function of an inline assembly block. Those have no types: every value is
    /// a word.
    pub is_yul: bool,
    /// The parameters, or `None` for a getter, whose parameters the AST does not list.
    pub(super) params: Option<Vec<SourceParam>>,
    /// The return values, or `None` for a getter.
    pub(super) returns: Option<Vec<SourceParam>>,
}

/// A function that has run, with the types of its parameters and return values.
#[derive(Clone, Debug)]
pub struct FunctionInfo {
    /// `Contract::name(types)`, or `Contract::name` when a type has no canonical name. Free
    /// functions have no contract.
    pub name: String,
    /// The parameters, or `None` if they are unknown.
    pub params: Option<Vec<Param>>,
    /// The return values, or `None` if they are unknown.
    pub returns: Option<Vec<Param>>,
    /// The name that the function is declared with, which the ABI spells too.
    pub declared_name: String,
    /// `name(types)` as the ABI spells it, or `None` when a parameter type is not known.
    pub signature: Option<String>,
    /// Whether the dispatcher or the call frame itself enters it.
    pub is_entry: bool,
}

impl FunctionInfo {
    /// A function that only the name is known of.
    pub fn untyped(name: String, is_entry: bool) -> Self {
        Self {
            declared_name: name.clone(),
            name,
            params: None,
            returns: None,
            signature: None,
            is_entry,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SourceParam {
    ty: SourceType,
    storage: Option<Storage>,
    /// The number of stack words the value takes.
    slots: usize,
}

/// A type as spelled in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SourceType {
    Elementary(DynSolType),
    /// A user-defined type, by the last segment of its name.
    Custom(String),
    Array(Box<Self>, Option<usize>),
    /// An external function type, which takes an address and a selector.
    ExternalFunction,
    /// A mapping or internal function type.
    Other,
}

impl SourceFunction {
    /// Resolves the types of the parameters and return values.
    pub fn info(&self, user_types: &UserTypes) -> FunctionInfo {
        let resolve = |params: &Option<Vec<SourceParam>>| {
            params.as_ref().map(|params| {
                params
                    .iter()
                    .map(|param| Param {
                        ty: param.ty.resolve(user_types),
                        storage: param.storage,
                        slots: param.slots,
                    })
                    .collect::<Vec<_>>()
            })
        };
        let params = resolve(&self.params);
        // A parameter type without a canonical name leaves the name without types, and so do Yul
        // functions, which have none.
        let types = params.as_deref().filter(|_| !self.is_yul).and_then(|params| {
            params
                .iter()
                .map(|param| param.ty.as_ref().map(DynSolType::sol_type_name))
                .collect::<Option<Vec<_>>>()
        });
        let signature = types.map(|types| format!("{}({})", self.name, types.join(",")));
        let spelled = signature.as_ref().unwrap_or(&self.name);
        let name = match &self.contract {
            Some(contract) => format!("{contract}::{spelled}"),
            None => spelled.clone(),
        };
        FunctionInfo {
            name,
            params,
            returns: resolve(&self.returns),
            declared_name: self.name.clone(),
            signature,
            is_entry: self.is_entry,
        }
    }
}

impl SourceType {
    fn resolve(&self, user_types: &UserTypes) -> Option<DynSolType> {
        match self {
            Self::Elementary(ty) => Some(ty.clone()),
            Self::Custom(name) => match user_types.get(name)?.as_ref()? {
                UserType::Address => Some(DynSolType::Address),
                UserType::Enum => Some(DynSolType::Uint(8)),
                UserType::Value(ty) => Some(ty.clone()),
                UserType::Struct => None,
            },
            Self::Array(element, size) => {
                let element = Box::new(element.resolve(user_types)?);
                Some(match size {
                    Some(size) => DynSolType::FixedArray(element, *size),
                    None => DynSolType::Array(element),
                })
            }
            Self::ExternalFunction | Self::Other => None,
        }
    }

    /// The number of stack words a value of this type takes: calldata arrays, `bytes` and
    /// `string` take an offset and a length, and external function types an address and a selector.
    const fn slots(&self, storage: Option<Storage>) -> usize {
        match (self, storage) {
            (Self::ExternalFunction, _) => 2,
            (
                Self::Elementary(DynSolType::String | DynSolType::Bytes) | Self::Array(_, None),
                Some(Storage::Calldata),
            ) => 2,
            _ => 1,
        }
    }
}

/// Collects the functions and user-defined types of a parsed source file.
///
/// `functions` maps the byte offset of each definition in the source to the function.
pub fn collect<'ast>(
    ast: &'ast SourceUnit<'ast>,
    source_map: &SourceMap,
    functions: &mut HashMap<usize, Arc<SourceFunction>>,
    user_types: &mut Vec<(String, Option<UserType>)>,
) {
    for item in ast.items.iter() {
        match &item.kind {
            ItemKind::Contract(contract) => {
                let name = contract.name.to_string();
                user_types.push((name.clone(), Some(UserType::Address)));
                for member in contract.body.iter() {
                    collect_item(member, Some(&name), source_map, functions, user_types);
                }
            }
            ItemKind::Import(import) => {
                // An alias can name any type, so a type of that name is not known by its name.
                if let ImportItems::Aliases(aliases) = &import.items {
                    let aliases = aliases.iter().filter_map(|(_, alias)| alias.as_ref());
                    user_types.extend(aliases.map(|alias| (alias.to_string(), None)));
                }
            }
            _ => collect_item(item, None, source_map, functions, user_types),
        }
    }
}

fn collect_item<'ast>(
    item: &'ast Item<'ast>,
    contract: Option<&str>,
    source_map: &SourceMap,
    functions: &mut HashMap<usize, Arc<SourceFunction>>,
    user_types: &mut Vec<(String, Option<UserType>)>,
) {
    let function = match &item.kind {
        ItemKind::Function(function) => {
            let mut yul = YulFunctions { contract, source_map, functions };
            let _ = yul.visit_item_function(function);
            let Some(name) = &function.header.name else { return };
            let public = matches!(
                function.header.visibility(),
                Some(Visibility::Public | Visibility::External)
            );
            Some(SourceFunction {
                contract: contract.map(str::to_string),
                name: name.to_string(),
                is_entry: function.kind == FunctionKind::Function && public,
                is_yul: false,
                params: Some(params(function.header.parameters.vars)),
                returns: Some(
                    function.header.returns.as_ref().map_or_else(Vec::new, |r| params(r.vars)),
                ),
            })
        }
        ItemKind::Variable(variable)
            if contract.is_some() && variable.visibility == Some(Visibility::Public) =>
        {
            variable.name.map(|name| SourceFunction {
                contract: contract.map(str::to_string),
                name: name.to_string(),
                is_entry: true,
                is_yul: false,
                params: None,
                returns: None,
            })
        }
        ItemKind::Enum(item) => {
            user_types.push((item.name.to_string(), Some(UserType::Enum)));
            None
        }
        ItemKind::Struct(item) => {
            user_types.push((item.name.to_string(), Some(UserType::Struct)));
            None
        }
        ItemKind::Udvt(item) => {
            if let SourceType::Elementary(ty) = source_type(&item.ty) {
                user_types.push((item.name.to_string(), Some(UserType::Value(ty))));
            }
            None
        }
        _ => None,
    };
    if let Some(function) = function
        && let Ok(range) = source_map.span_to_range(item.span)
    {
        functions.insert(range.start, Arc::new(function));
    }
}

/// Collects the functions of the inline assembly blocks of a function body.
struct YulFunctions<'a> {
    contract: Option<&'a str>,
    source_map: &'a SourceMap,
    functions: &'a mut HashMap<usize, Arc<SourceFunction>>,
}

impl<'ast> Visit<'ast> for YulFunctions<'_> {
    type BreakValue = Never;

    fn visit_yul_stmt(&mut self, stmt: &'ast yul::Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        if let YulStmtKind::FunctionDef(function) = &stmt.kind
            && let Ok(range) = self.source_map.span_to_range(stmt.span)
        {
            let word = || SourceParam {
                ty: SourceType::Elementary(DynSolType::Uint(256)),
                storage: None,
                slots: 1,
            };
            let words = |names: &[Ident]| names.iter().map(|_| word()).collect();
            self.functions.insert(
                range.start,
                Arc::new(SourceFunction {
                    contract: self.contract.map(str::to_string),
                    name: function.name.to_string(),
                    is_entry: false,
                    is_yul: true,
                    params: Some(words(function.parameters)),
                    returns: Some(words(function.returns)),
                }),
            );
        }
        self.walk_yul_stmt(stmt)
    }
}

fn params(vars: &[VariableDefinition<'_>]) -> Vec<SourceParam> {
    vars.iter()
        .map(|var| {
            let ty = source_type(&var.ty);
            let storage = var.data_location.map(|location| match location {
                DataLocation::Memory => Storage::Memory,
                DataLocation::Calldata => Storage::Calldata,
                DataLocation::Storage | DataLocation::Transient => Storage::Storage,
            });
            SourceParam { slots: ty.slots(storage), ty, storage }
        })
        .collect()
}

fn source_type(ty: &Type<'_>) -> SourceType {
    match &ty.kind {
        TypeKind::Elementary(ty) => SourceType::Elementary(match *ty {
            ElementaryType::Address(_) => DynSolType::Address,
            ElementaryType::Bool => DynSolType::Bool,
            ElementaryType::String => DynSolType::String,
            ElementaryType::Bytes => DynSolType::Bytes,
            ElementaryType::Int(size) => DynSolType::Int(size.bits() as usize),
            ElementaryType::UInt(size) => DynSolType::Uint(size.bits() as usize),
            ElementaryType::FixedBytes(size) => DynSolType::FixedBytes(size.bytes() as usize),
        }),
        TypeKind::Array(array) => {
            let size = match &array.size {
                None => None,
                Some(size) => {
                    if let ExprKind::Lit(lit, _) = &size.kind
                        && let LitKind::Number(number) = &lit.kind
                        && let Ok(size) = usize::try_from(*number)
                    {
                        Some(size)
                    } else {
                        return SourceType::Other;
                    }
                }
            };
            SourceType::Array(Box::new(source_type(&array.element)), size)
        }
        TypeKind::Custom(path) => path
            .segments()
            .last()
            .map_or(SourceType::Other, |last| SourceType::Custom(last.to_string())),
        TypeKind::Function(function) if function.visibility() == Some(Visibility::External) => {
            SourceType::ExternalFunction
        }
        TypeKind::Function(_) | TypeKind::Mapping(_) => SourceType::Other,
    }
}

/// The name that solc's `functionDebugData` key spells for a function with the AST id `id`:
/// `@name_id` in legacy codegen, and `fun_name`, `modifier_name` or `getter_fun_name` in via-IR
/// codegen. The via-IR names of functions that share a name end in a number as well, and drop
/// leading underscores.
pub fn name_of_key(key: &str, id: u32) -> Option<String> {
    let name = key
        .strip_prefix('@')
        .or_else(|| key.strip_prefix("getter_fun_"))
        .or_else(|| key.strip_prefix("modifier_"))
        .or_else(|| key.strip_prefix("fun_"))?;
    // The fallback function has no name.
    let name = match name.rsplit_once('_') {
        Some((base, number)) if number.parse() == Ok(id) => base,
        _ => name,
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// Whether the name that a key spells is the name of the function `name`, up to the leading
/// underscores and the number that solc may add to a name.
pub fn names_agree(key_name: &str, name: &str) -> bool {
    let (key_name, name) = (key_name.trim_start_matches('_'), name.trim_start_matches('_'));
    key_name == name || without_number(key_name) == name
}

/// A name without the number that solc adds to a name that functions share.
pub fn without_number(name: &str) -> &str {
    match name.rsplit_once('_') {
        Some((base, number))
            if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) =>
        {
            base
        }
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_of_function_debug_data_keys() {
        assert_eq!(name_of_key("@runChain_76", 76).as_deref(), Some("runChain"));
        assert_eq!(name_of_key("fun_runChain", 76).as_deref(), Some("runChain"));
        assert_eq!(name_of_key("fun_v_264", 264).as_deref(), Some("v"));
        assert_eq!(name_of_key("fun_v", 26).as_deref(), Some("v"));
        assert_eq!(name_of_key("modifier_onlyPositive", 52).as_deref(), Some("onlyPositive"));
        assert_eq!(name_of_key("getter_fun_s", 31).as_deref(), Some("s"));
        assert_eq!(name_of_key("abi_decode_t_uint256", 1), None);
        assert_eq!(name_of_key("fun_", 90), None);
    }

    #[test]
    fn keys_agree_with_names_up_to_underscores_and_numbers() {
        assert!(names_agree("runChain", "runChain"));
        // via-IR drops the leading underscore of `_checkOwner` and numbers shared names.
        assert!(names_agree("checkOwner", "_checkOwner"));
        assert!(names_agree("assertEq_57107", "assertEq"));
        assert!(names_agree("_checkOwner", "checkOwner"));
        assert!(!names_agree("step", "stop"));
        assert!(!names_agree("step_2", "stop"));
    }
}
