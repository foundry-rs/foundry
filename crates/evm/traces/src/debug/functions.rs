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
        DataLocation, ElementaryType, Expr, ExprKind, FunctionKind, ImportItems, Item,
        ItemFunction, ItemKind, LitKind, SourceUnit, Type, TypeKind, VariableDefinition,
        Visibility,
        visit::Visit,
        yul::{self, StmtKind as YulStmtKind},
    },
    data_structures::Never,
    interface::{Ident, source_map::SourceMap},
};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    ops::ControlFlow,
    sync::Arc,
};

/// The user-defined types of the sources by name, or `None` for a name that two types share.
pub type UserTypes = HashMap<String, Option<UserType>>;

/// A user-defined type that a source declares, in a contract or at the top level of the file.
#[derive(Clone, Debug)]
pub struct DeclaredType {
    /// The contract that declares it.
    pub contract: Option<String>,
    pub name: String,
    /// The type, or `None` if the name is not known to be one, such as an import alias.
    pub ty: Option<UserType>,
}

/// What a source declares: its functions by the offset of their definition, its types, and the
/// bases of its contracts.
#[derive(Debug, Default)]
pub struct Declarations {
    pub functions: HashMap<usize, Arc<SourceFunction>>,
    pub types: Vec<DeclaredType>,
    pub bases: Vec<(String, Vec<String>)>,
}

/// The user-defined types that a function sees, from the nearest scope out: the types of its
/// contract and the contracts that it inherits from, those of its file, and then the types of the
/// project that have one declaration of that name.
pub struct TypeScope<'a> {
    /// The contract of the function and its bases.
    pub contracts: Vec<String>,
    /// The types declared in contracts by contract and name, or `None` for two declarations.
    pub contract_types: &'a HashMap<(String, String), Option<UserType>>,
    /// The types declared in the file of the function.
    pub file: &'a [DeclaredType],
    /// The types of the project by name.
    pub project: &'a UserTypes,
}

impl<'a> TypeScope<'a> {
    /// A scope of the types of the project only.
    pub fn project(project: &'a UserTypes) -> Self {
        static NONE: std::sync::OnceLock<HashMap<(String, String), Option<UserType>>> =
            std::sync::OnceLock::new();
        Self {
            contracts: Vec::new(),
            contract_types: NONE.get_or_init(HashMap::new),
            file: &[],
            project,
        }
    }

    fn get_qualified(&self, qualifier: &str, name: &str) -> Option<&'a Option<UserType>> {
        self.contract_types
            .get(&(qualifier.to_string(), name.to_string()))
            .or_else(|| self.project.get(name))
    }

    fn get(&self, name: &str) -> Option<&'a Option<UserType>> {
        self.contracts
            .iter()
            .find_map(|contract| self.contract_types.get(&(contract.clone(), name.to_string())))
            .or_else(|| {
                self.file
                    .iter()
                    .find(|declared| declared.contract.is_none() && declared.name == name)
                    .map(|declared| &declared.ty)
            })
            .or_else(|| self.project.get(name))
    }
}

/// A user-defined type and how it appears in function signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserType {
    /// A contract, interface, or library, which the ABI treats as an address.
    Address,
    /// An enum, which the ABI treats as `uint8`.
    Enum,
    /// A user-defined value type of an elementary type.
    Value(DynSolType),
    /// A struct, which does not have a single stack word to decode, with the types of its fields.
    Struct(Vec<SourceType>),
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
    /// A hash of the body without spaces and of the parameter and return types, with the names of
    /// the parameters and return values replaced by their positions, for telling which functions
    /// can have the same code. `None` for getters and functions without a body.
    pub body_hash: Option<u64>,
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
    /// What the ABI spells for each parameter, or `None` if the parameters are not known.
    pub abi_types: Option<Vec<AbiType>>,
    /// Whether the dispatcher or the call frame itself enters it.
    pub is_entry: bool,
    /// The names of the functions without an entry point that the compiler may have merged into
    /// this function, because they take and leave as many words and have the same code. A call
    /// to the entry can be a call to any of them.
    pub merged: Vec<String>,
}

/// What the ABI spells for a parameter of a function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbiType {
    /// A canonical type name, such as `uint256[]`.
    Known(String),
    /// A struct, or an array of structs, which the ABI spells as a tuple.
    Tuple,
    /// Another type that the AST does not tell the ABI type of.
    Unknown,
}

impl FunctionInfo {
    /// Whether `signature`, as the ABI spells it, can be the signature of this function. The
    /// parameters that are not known match any type, or any tuple if they are structs.
    pub fn matches_signature(&self, signature: &str) -> bool {
        let Some((name, arguments)) = signature.split_once('(') else { return false };
        if name != self.declared_name {
            return false;
        }
        let Some(types) = &self.abi_types else { return true };
        let Some(arguments) = arguments.strip_suffix(')') else { return false };
        // Split at the commas outside tuples.
        let mut depth = 0;
        let mut arguments = arguments
            .split(|c| {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                c == ',' && depth == 0
            })
            .filter(|argument| !argument.is_empty());
        let matches = types.iter().all(|ty| {
            arguments.next().is_some_and(|argument| match ty {
                AbiType::Known(known) => known == argument,
                AbiType::Tuple => argument.contains('('),
                AbiType::Unknown => true,
            })
        });
        matches && arguments.next().is_none()
    }

    /// A function that only the name is known of.
    pub fn untyped(name: String, is_entry: bool) -> Self {
        Self {
            declared_name: name.clone(),
            name,
            params: None,
            returns: None,
            abi_types: None,
            is_entry,
            merged: Vec::new(),
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
pub enum SourceType {
    Elementary(DynSolType),
    /// A user-defined type, by the last segment of its name.
    Custom(String),
    /// A user-defined type that a qualifier names, such as `Types.Amount`: the qualifier, which
    /// can be a contract or an import alias, and the last segment of the name.
    Qualified(String, String),
    Array(Box<Self>, Option<usize>),
    /// An external function type, which takes an address and a selector.
    ExternalFunction,
    /// A mapping or internal function type.
    Other,
}

impl SourceFunction {
    /// Resolves the types of the parameters and return values.
    pub fn info(&self, user_types: &TypeScope<'_>) -> FunctionInfo {
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
        let abi_types = self.params.as_ref().filter(|_| !self.is_yul).map(|params| {
            params
                .iter()
                .map(|param| match param.ty.abi_name(user_types, 0) {
                    Some(name) => AbiType::Known(name),
                    None if param.ty.is_struct(user_types) => AbiType::Tuple,
                    None => AbiType::Unknown,
                })
                .collect()
        });
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
            abi_types,
            is_entry: self.is_entry,
            merged: Vec::new(),
        }
    }
}

impl SourceType {
    fn resolve(&self, user_types: &TypeScope<'_>) -> Option<DynSolType> {
        match self {
            Self::Elementary(ty) => Some(ty.clone()),
            Self::Custom(_) | Self::Qualified(..) => match self.declared(user_types)?.as_ref()? {
                UserType::Address => Some(DynSolType::Address),
                UserType::Enum => Some(DynSolType::Uint(8)),
                UserType::Value(ty) => Some(ty.clone()),
                UserType::Struct(_) => None,
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

    /// The name that the ABI spells for the type, with structs as tuples of their fields, or
    /// `None` if it is not known. `depth` stops structs that contain themselves.
    fn abi_name(&self, user_types: &TypeScope<'_>, depth: usize) -> Option<String> {
        match self {
            Self::Elementary(ty) => Some(ty.sol_type_name().into_owned()),
            Self::Custom(_) | Self::Qualified(..) => match self.declared(user_types)?.as_ref()? {
                UserType::Address => Some("address".to_string()),
                UserType::Enum => Some("uint8".to_string()),
                UserType::Value(ty) => Some(ty.sol_type_name().into_owned()),
                UserType::Struct(fields) if depth < 8 => {
                    let fields = fields
                        .iter()
                        .map(|field| field.abi_name(user_types, depth + 1))
                        .collect::<Option<Vec<_>>>()?;
                    Some(format!("({})", fields.join(",")))
                }
                UserType::Struct(_) => None,
            },
            Self::Array(element, size) => {
                let element = element.abi_name(user_types, depth)?;
                Some(match size {
                    Some(size) => format!("{element}[{size}]"),
                    None => format!("{element}[]"),
                })
            }
            Self::ExternalFunction => Some("function".to_string()),
            Self::Other => None,
        }
    }

    /// The user-defined type that the name refers to, if there is one that is known to be it. A
    /// qualified name is a type of the qualifier if that is a contract that declares it, and
    /// otherwise the type of the project of that name, if it has one declaration: the declarations
    /// of the contract and the file of the function are not what it names.
    fn declared<'a>(&self, scope: &TypeScope<'a>) -> Option<&'a Option<UserType>> {
        match self {
            Self::Custom(name) => scope.get(name),
            Self::Qualified(qualifier, name) => scope.get_qualified(qualifier, name),
            _ => None,
        }
    }

    /// Whether the type is a struct or an array of structs.
    fn is_struct(&self, user_types: &TypeScope<'_>) -> bool {
        match self {
            Self::Custom(_) | Self::Qualified(..) => {
                matches!(self.declared(user_types), Some(Some(UserType::Struct(_))))
            }
            Self::Array(element, _) => element.is_struct(user_types),
            _ => false,
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
    declarations: &mut Declarations,
) {
    for item in ast.items.iter() {
        match &item.kind {
            ItemKind::Contract(contract) => {
                let name = contract.name.to_string();
                declarations.types.push(DeclaredType {
                    contract: None,
                    name: name.clone(),
                    ty: Some(UserType::Address),
                });
                let bases = contract
                    .bases
                    .iter()
                    .filter_map(|base| base.name.segments().last().map(|last| last.to_string()))
                    .collect();
                declarations.bases.push((name.clone(), bases));
                for member in contract.body.iter() {
                    collect_item(member, Some(&name), source_map, declarations);
                }
            }
            ItemKind::Import(import) => {
                // An alias can name any type, so a type of that name is not known by its name.
                if let ImportItems::Aliases(aliases) = &import.items {
                    let aliases = aliases.iter().filter_map(|(_, alias)| alias.as_ref());
                    declarations.types.extend(aliases.map(|alias| DeclaredType {
                        contract: None,
                        name: alias.to_string(),
                        ty: None,
                    }));
                }
            }
            _ => collect_item(item, None, source_map, declarations),
        }
    }
}

fn collect_item<'ast>(
    item: &'ast Item<'ast>,
    contract: Option<&str>,
    source_map: &SourceMap,
    declarations: &mut Declarations,
) {
    let declare = |declarations: &mut Declarations, name: String, ty: UserType| {
        declarations.types.push(DeclaredType {
            contract: contract.map(str::to_string),
            name,
            ty: Some(ty),
        });
    };
    let function = match &item.kind {
        ItemKind::Function(function) => {
            let mut yul =
                YulFunctions { contract, source_map, functions: &mut declarations.functions };
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
                body_hash: body_hash(function, source_map),
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
                body_hash: None,
                params: None,
                returns: None,
            })
        }
        ItemKind::Enum(item) => {
            declare(declarations, item.name.to_string(), UserType::Enum);
            None
        }
        ItemKind::Struct(item) => {
            let fields = item.fields.iter().map(|field| source_type(&field.ty)).collect();
            declare(declarations, item.name.to_string(), UserType::Struct(fields));
            None
        }
        ItemKind::Udvt(item) => {
            if let SourceType::Elementary(ty) = source_type(&item.ty) {
                declare(declarations, item.name.to_string(), UserType::Value(ty));
            }
            None
        }
        _ => None,
    };
    if let Some(function) = function
        && let Ok(range) = source_map.span_to_range(item.span)
    {
        declarations.functions.insert(range.start, Arc::new(function));
    }
}

/// Collects the call expressions of a parsed source file: the name that each calls, by the byte
/// offset where the expression starts. A call through a variable, such as a function pointer, is
/// recorded by the name of the variable.
pub fn collect_call_sites<'ast>(
    ast: &'ast SourceUnit<'ast>,
    source_map: &SourceMap,
    call_sites: &mut HashMap<usize, String>,
) {
    let _ = CallSites { source_map, call_sites }.visit_source_unit(ast);
}

struct CallSites<'a> {
    source_map: &'a SourceMap,
    call_sites: &'a mut HashMap<usize, String>,
}

impl<'ast> Visit<'ast> for CallSites<'_> {
    type BreakValue = Never;

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Call(callee, _) = &expr.kind
            && let ExprKind::Ident(name) | ExprKind::Member(_, name) = &callee.kind
            && let Ok(range) = self.source_map.span_to_range(expr.span)
        {
            self.call_sites.insert(range.start, name.to_string());
        }
        self.walk_expr(expr)
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
                    body_hash: None,
                    params: Some(words(function.parameters)),
                    returns: Some(words(function.returns)),
                }),
            );
        }
        self.walk_yul_stmt(stmt)
    }
}

/// Hashes the body of a function and the types of its parameters and return values, with the names
/// of the parameters and return values replaced by their positions, so that functions with the
/// same code have the same hash.
fn body_hash(function: &ItemFunction<'_>, source_map: &SourceMap) -> Option<u64> {
    function.body.as_ref()?;
    let body = source_map.span_to_snippet(function.body_span).ok()?;
    let names = function
        .header
        .parameters
        .vars
        .iter()
        .chain(function.header.returns.iter().flat_map(|returns| returns.vars.iter()))
        .map(|var| var.name.map(|name| name.to_string()))
        .collect::<Vec<_>>();
    let mut hasher = DefaultHasher::new();
    // The code depends on the types of the parameters and return values as well.
    let returns = function.header.returns.iter().flat_map(|returns| returns.vars.iter());
    for var in function.header.parameters.vars.iter().chain(returns) {
        format!("{:?}{:?}", source_type(&var.ty), var.data_location).hash(&mut hasher);
    }
    let mut word = String::new();
    // Hash the words and the other characters of the body, skipping white space.
    let flush = |word: &mut String, hasher: &mut DefaultHasher| {
        if !word.is_empty() {
            match names.iter().position(|name| name.as_deref() == Some(word.as_str())) {
                Some(position) => (true, position).hash(hasher),
                None => (false, word.as_str()).hash(hasher),
            }
            word.clear();
        }
    };
    for c in body.chars() {
        if c.is_alphanumeric() || c == '_' || c == '$' {
            word.push(c);
        } else {
            flush(&mut word, &mut hasher);
            if !c.is_whitespace() {
                c.hash(&mut hasher);
            }
        }
    }
    flush(&mut word, &mut hasher);
    Some(hasher.finish())
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
        TypeKind::Custom(path) => match path.segments() {
            [.., qualifier, last] => SourceType::Qualified(qualifier.to_string(), last.to_string()),
            [last] => SourceType::Custom(last.to_string()),
            [] => SourceType::Other,
        },
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

    #[test]
    fn structs_are_spelled_as_tuples_of_their_fields() {
        let word = |bits| SourceType::Elementary(DynSolType::Uint(bits));
        let user_types = UserTypes::from([
            ("Unsigned".into(), Some(UserType::Struct(vec![word(256)]))),
            (
                "Signed".into(),
                Some(UserType::Struct(vec![SourceType::Elementary(DynSolType::Int(256))])),
            ),
            (
                "Nested".into(),
                Some(UserType::Struct(vec![SourceType::Custom("Unsigned".into()), word(8)])),
            ),
            ("Cyclic".into(), Some(UserType::Struct(vec![SourceType::Custom("Cyclic".into())]))),
        ]);
        let scope = TypeScope::project(&user_types);
        let abi = |ty: SourceType| ty.abi_name(&scope, 0);
        let custom = |name: &str| SourceType::Custom(name.into());

        assert_eq!(abi(custom("Unsigned")).as_deref(), Some("(uint256)"));
        assert_eq!(abi(custom("Signed")).as_deref(), Some("(int256)"));
        assert_eq!(abi(custom("Nested")).as_deref(), Some("((uint256),uint8)"));
        assert_eq!(
            abi(SourceType::Array(Box::new(custom("Unsigned")), Some(2))).as_deref(),
            Some("(uint256)[2]")
        );
        // A struct that contains itself has no finite spelling here.
        assert_eq!(abi(custom("Cyclic")), None);
        assert_eq!(abi(custom("Missing")), None);
    }

    #[test]
    fn a_qualified_type_is_not_a_declaration_of_the_contract_or_file() {
        let amount = |ty| Some(UserType::Value(ty));
        // The project has two `Amount` types, and the contract of the function declares one.
        let project = UserTypes::from([("Amount".into(), None)]);
        let contract_types = HashMap::from([
            (("Local".into(), "Amount".into()), amount(DynSolType::Address)),
            (("Types".into(), "Amount".into()), amount(DynSolType::Uint(256))),
        ]);
        let scope = TypeScope {
            contracts: vec!["Local".into()],
            contract_types: &contract_types,
            file: &[],
            project: &project,
        };
        let qualified = |qualifier: &str| SourceType::Qualified(qualifier.into(), "Amount".into());

        assert_eq!(SourceType::Custom("Amount".into()).resolve(&scope), Some(DynSolType::Address));
        // The qualifier is a contract that declares the type.
        assert_eq!(qualified("Types").resolve(&scope), Some(DynSolType::Uint(256)));
        // The qualifier is an import alias: the name is ambiguous in the project.
        assert_eq!(qualified("Imported").resolve(&scope), None);
    }
}
