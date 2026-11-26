//! Functions, modifiers, and getters of a Solar AST, found by the offset where solc's source map
//! places their definition.
//!
//! solc's `functionDebugData` lists an internal function by the offset of its entry `JUMPDEST`,
//! and the source map puts that instruction at the start of the function's definition. The Solar
//! AST that Foundry parses for every source file holds the definition there, with the contract it
//! is in, its name, and its parameters. Solar resolves the types of the parameters when it lowers
//! the sources to its HIR, which is what the types of the functions come from.

use super::Param;
use alloy_dyn_abi::{DynSolType, parser::Storage};
use solar::{
    ast::{
        DataLocation, ElementaryType, Expr, ExprKind, FunctionKind, Item, ItemFunction, ItemKind,
        SourceUnit, Type, TypeKind, VariableDefinition, Visibility,
        visit::Visit,
        yul::{self, StmtKind as YulStmtKind},
    },
    data_structures::Never,
    interface::{Ident, source_map::SourceMap},
    sema::{
        Gcx,
        hir::ItemId,
        ty::{Ty, TyKind},
    },
};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    ops::ControlFlow,
    sync::Arc,
};

/// What a source declares: its functions by the offset of their definition, the bases of its
/// contracts, and whether each contract is a library.
#[derive(Debug, Default)]
pub struct Declarations {
    pub functions: HashMap<usize, Arc<SourceFunction>>,
    pub bases: Vec<(String, Vec<String>)>,
    /// The contracts by name, and whether they are libraries.
    pub libraries: Vec<(String, bool)>,
}

/// The types of the parameters and return values of a function that Solar resolved.
#[derive(Clone, Debug)]
pub struct ResolvedTypes {
    params: Vec<ResolvedType>,
    returns: Vec<ResolvedType>,
}

/// The type of a parameter or return value that Solar resolved.
#[derive(Clone, Debug)]
struct ResolvedType {
    /// The type that its stack words can be decoded as. Structs and function types are not.
    decodable: Option<DynSolType>,
    abi: AbiType,
    /// Whether a library ABI spells the type other than `abi`, or leaves out the function: enums,
    /// structs and contracts by name, and storage pointers.
    library_spelled: bool,
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
    /// What the ABI of a library spells for each parameter: the same, except that the types that
    /// a library ABI spells by name are not known.
    pub library_abi_types: Option<Vec<AbiType>>,
    /// Whether the dispatcher or the call frame itself enters it.
    pub is_entry: bool,
    /// Whether a library ABI spells a parameter other than the ABI types do, or leaves the
    /// function out, so that its selector is no selector of the ABI.
    pub library_spelled: bool,
    /// The names of the functions without an entry point that the compiler may have merged into
    /// this function, because they take and leave as many words and have the same code. A call
    /// to the entry can be a call to any of them.
    pub merged: Vec<String>,
}

/// Whether `name`, as the ABI spells a type, is an elementary type or an array of one.
fn is_elementary(name: &str) -> bool {
    let base = name.split('[').next().unwrap_or(name);
    let digits = |rest: &str| rest.bytes().all(|b| b.is_ascii_digit());
    matches!(base, "address" | "bool" | "string" | "bytes" | "function")
        || base.strip_prefix("bytes").is_some_and(|rest| !rest.is_empty() && digits(rest))
        || base.strip_prefix("uint").or_else(|| base.strip_prefix("int")).is_some_and(digits)
}

/// What the ABI spells for a parameter of a function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbiType {
    /// The name that the ABI spells, such as `uint256[]` or `(uint256,address)` for a struct.
    Known(String),
    /// A type that Solar did not resolve.
    Unknown,
    /// A type that a library ABI spells by name, such as `Lib.K` or `Token`, and not by an
    /// elementary type.
    ByName,
}

impl FunctionInfo {
    /// Whether `signature`, as the ABI spells it, can be the signature of this function. The
    /// parameters that are not known match any type.
    pub fn matches_signature(&self, signature: &str) -> bool {
        self.matches(signature, &self.abi_types)
    }

    /// Whether `signature`, as the ABI of a library spells it, can be the signature of this
    /// function.
    pub fn matches_library_signature(&self, signature: &str) -> bool {
        self.matches(signature, &self.library_abi_types)
    }

    fn matches(&self, signature: &str, types: &Option<Vec<AbiType>>) -> bool {
        let Some((name, arguments)) = signature.split_once('(') else { return false };
        if name != self.declared_name {
            return false;
        }
        let Some(types) = types else { return true };
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
                AbiType::Unknown => true,
                AbiType::ByName => !is_elementary(argument),
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
            library_abi_types: None,
            is_entry,
            library_spelled: false,
            merged: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SourceParam {
    ty: ResolvedType,
    storage: Option<Storage>,
    /// The number of stack words the value takes.
    slots: usize,
}

impl SourceFunction {
    /// The types of the parameters and return values.
    pub fn info(&self) -> FunctionInfo {
        let params = |params: &Option<Vec<SourceParam>>| {
            params.as_ref().map(|params| {
                params
                    .iter()
                    .map(|param| Param {
                        ty: param.ty.decodable.clone(),
                        storage: param.storage,
                        slots: param.slots,
                    })
                    .collect::<Vec<_>>()
            })
        };
        let params_and_returns = (params(&self.params), params(&self.returns));
        let (params, returns) = params_and_returns;
        // A parameter type without a canonical name leaves the name without types, and so do Yul
        // functions, which have none.
        let types = params.as_deref().filter(|_| !self.is_yul).and_then(|params| {
            params
                .iter()
                .map(|param| param.ty.as_ref().map(DynSolType::sol_type_name))
                .collect::<Option<Vec<_>>>()
        });
        let signature = types.map(|types| format!("{}({})", self.name, types.join(",")));
        let abi_types = self
            .params
            .as_ref()
            .filter(|_| !self.is_yul)
            .map(|params| params.iter().map(|param| param.ty.abi.clone()).collect());
        let spelled_by_name = |param: &SourceParam| {
            param.ty.library_spelled || param.storage == Some(Storage::Storage)
        };
        let library_abi_types =
            self.params.as_ref().filter(|_| !self.is_yul).map(|params| {
                params
                    .iter()
                    .map(|param| {
                        if spelled_by_name(param) { AbiType::ByName } else { param.ty.abi.clone() }
                    })
                    .collect()
            });
        let library_spelled = self.params.iter().flatten().any(spelled_by_name);
        let spelled = signature.as_ref().unwrap_or(&self.name);
        let name = match &self.contract {
            Some(contract) => format!("{contract}::{spelled}"),
            None => spelled.clone(),
        };
        FunctionInfo {
            name,
            params,
            returns,
            declared_name: self.name.clone(),
            abi_types,
            library_abi_types,
            is_entry: self.is_entry,
            library_spelled,
            merged: Vec::new(),
        }
    }
}

/// Resolves the types of the parameters and return values of the functions of a source, by the
/// offset of the function, with the types that Solar resolved when it lowered the sources to HIR.
/// A source that is not lowered has none.
pub fn resolve_types(
    gcx: Gcx<'_>,
    source_map: &SourceMap,
    path: impl Into<solar::interface::source_map::FileName>,
) -> HashMap<usize, ResolvedTypes> {
    let mut resolved = HashMap::new();
    if gcx.stage().is_none_or(|stage| stage.is_parsing()) {
        return resolved;
    }
    let Some((_, source)) = gcx.get_hir_source(path) else { return resolved };
    let mut function = |id| {
        let function = gcx.hir.function(id);
        if function.gettee.is_some() {
            return;
        }
        let Ok(range) = source_map.span_to_range(function.span) else { return };
        let types = |variables: &[solar::sema::hir::VariableId]| {
            variables
                .iter()
                .map(|&variable| resolve_type(gcx, gcx.type_of_item(variable.into())))
                .collect()
        };
        resolved.insert(
            range.start,
            ResolvedTypes { params: types(function.parameters), returns: types(function.returns) },
        );
    };
    for item in source.items {
        match item {
            ItemId::Function(id) => function(*id),
            ItemId::Contract(id) => gcx.hir.contract(*id).functions().for_each(&mut function),
            _ => {}
        }
    }
    resolved
}

fn resolve_type<'gcx>(gcx: Gcx<'gcx>, ty: Ty<'gcx>) -> ResolvedType {
    let library_spelled = library_spelled(ty);
    match dyn_type(gcx, ty, 0) {
        Some(dyn_type) => ResolvedType {
            abi: AbiType::Known(abi_name(&dyn_type)),
            decodable: Some(dyn_type).filter(|ty| !has_tuple_or_function(ty)),
            library_spelled,
        },
        None => ResolvedType { decodable: None, abi: AbiType::Unknown, library_spelled },
    }
}

/// Whether the ABI of a library spells the type by name, or leaves out a function that has it:
/// enums, structs and contracts, mappings, and storage pointers. User-defined value types are
/// spelled as their underlying type.
fn library_spelled(ty: Ty<'_>) -> bool {
    match ty.kind {
        TyKind::Ref(inner, location) => {
            matches!(location, DataLocation::Storage) || library_spelled(inner)
        }
        TyKind::Array(element, _) | TyKind::DynArray(element) => library_spelled(element),
        TyKind::Enum(_) | TyKind::Struct(_) | TyKind::Contract(_) | TyKind::Mapping(..) => true,
        TyKind::Udvt(inner, _) => library_spelled(inner),
        _ => false,
    }
}

/// The type that the ABI has for a type of Solar: user-defined value types are their underlying
/// type, enums `uint8`, contracts `address`, and structs tuples of their fields. `depth` stops the
/// structs that contain themselves.
fn dyn_type<'gcx>(gcx: Gcx<'gcx>, ty: Ty<'gcx>, depth: usize) -> Option<DynSolType> {
    Some(match ty.kind {
        TyKind::Elementary(ty) => elementary(ty),
        TyKind::Ref(inner, _) | TyKind::Udvt(inner, _) => return dyn_type(gcx, inner, depth),
        TyKind::Array(element, size) => {
            DynSolType::FixedArray(Box::new(dyn_type(gcx, element, depth)?), size.try_into().ok()?)
        }
        TyKind::DynArray(element) => DynSolType::Array(Box::new(dyn_type(gcx, element, depth)?)),
        TyKind::Struct(id) if depth < 8 => DynSolType::Tuple(
            gcx.struct_field_types(id)
                .iter()
                .map(|field| dyn_type(gcx, *field, depth + 1))
                .collect::<Option<Vec<_>>>()?,
        ),
        TyKind::Enum(_) => DynSolType::Uint(8),
        TyKind::Contract(_) => DynSolType::Address,
        TyKind::Fn(function) if function.is_external() => DynSolType::Function,
        _ => return None,
    })
}

const fn elementary(ty: ElementaryType) -> DynSolType {
    match ty {
        ElementaryType::Address(_) => DynSolType::Address,
        ElementaryType::Bool => DynSolType::Bool,
        ElementaryType::String => DynSolType::String,
        ElementaryType::Bytes => DynSolType::Bytes,
        ElementaryType::Int(size) => DynSolType::Int(size.bits() as usize),
        ElementaryType::UInt(size) => DynSolType::Uint(size.bits() as usize),
        ElementaryType::FixedBytes(size) => DynSolType::FixedBytes(size.bytes() as usize),
    }
}

/// The name that the ABI spells for a type, with structs as tuples.
fn abi_name(ty: &DynSolType) -> String {
    match ty {
        DynSolType::Tuple(fields) => {
            format!("({})", fields.iter().map(abi_name).collect::<Vec<_>>().join(","))
        }
        DynSolType::Array(element) => format!("{}[]", abi_name(element)),
        DynSolType::FixedArray(element, size) => format!("{}[{size}]", abi_name(element)),
        DynSolType::Function => "function".to_string(),
        ty => ty.sol_type_name().into_owned(),
    }
}

/// Whether the words of a value of the type are a pointer or a function: not a value to decode.
fn has_tuple_or_function(ty: &DynSolType) -> bool {
    match ty {
        DynSolType::Tuple(_) | DynSolType::Function => true,
        DynSolType::Array(element) | DynSolType::FixedArray(element, _) => {
            has_tuple_or_function(element)
        }
        _ => false,
    }
}

/// Collects the functions of a parsed source file, with the types that `types` has for them by the
/// offset of their definition.
///
/// `declarations.functions` maps the byte offset of each definition in the source to the function.
pub fn collect<'ast>(
    ast: &'ast SourceUnit<'ast>,
    source_map: &SourceMap,
    types: &HashMap<usize, ResolvedTypes>,
    declarations: &mut Declarations,
) {
    for item in ast.items.iter() {
        match &item.kind {
            ItemKind::Contract(contract) => {
                let name = contract.name.to_string();
                let bases = contract
                    .bases
                    .iter()
                    .filter_map(|base| base.name.segments().last().map(|last| last.to_string()))
                    .collect();
                declarations.bases.push((name.clone(), bases));
                declarations.libraries.push((name.clone(), contract.kind.is_library()));
                for member in contract.body.iter() {
                    collect_item(member, Some(&name), source_map, types, declarations);
                }
            }
            _ => collect_item(item, None, source_map, types, declarations),
        }
    }
}

fn collect_item<'ast>(
    item: &'ast Item<'ast>,
    contract: Option<&str>,
    source_map: &SourceMap,
    types: &HashMap<usize, ResolvedTypes>,
    declarations: &mut Declarations,
) {
    let Ok(range) = source_map.span_to_range(item.span) else { return };
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
            let returns = function.header.returns.as_ref().map_or(&[][..], |r| r.vars);
            let resolved = types.get(&range.start);
            let parameters = params(function.header.parameters.vars, resolved.map(|r| &r.params));
            let returns = params(returns, resolved.map(|r| &r.returns));
            Some(SourceFunction {
                contract: contract.map(str::to_string),
                name: name.to_string(),
                is_entry: function.kind == FunctionKind::Function && public,
                is_yul: false,
                body_hash: body_hash(function, source_map, &parameters, &returns),
                params: Some(parameters),
                returns: Some(returns),
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
        _ => None,
    };
    if let Some(function) = function {
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
                ty: ResolvedType {
                    decodable: Some(DynSolType::Uint(256)),
                    abi: AbiType::Known("uint256".to_string()),
                    library_spelled: false,
                },
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
fn body_hash(
    function: &ItemFunction<'_>,
    source_map: &SourceMap,
    params: &[SourceParam],
    returns: &[SourceParam],
) -> Option<u64> {
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
    for param in params.iter().chain(returns) {
        format!("{:?}{:?}", param.ty.abi, param.storage).hash(&mut hasher);
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

/// The parameters or return values that `vars` define, with the types that Solar resolved if it
/// resolved as many.
fn params(
    vars: &[VariableDefinition<'_>],
    resolved: Option<&Vec<ResolvedType>>,
) -> Vec<SourceParam> {
    let resolved = resolved.filter(|resolved| resolved.len() == vars.len());
    vars.iter()
        .enumerate()
        .map(|(i, var)| {
            let storage = var.data_location.map(|location| match location {
                DataLocation::Memory => Storage::Memory,
                DataLocation::Calldata => Storage::Calldata,
                DataLocation::Storage | DataLocation::Transient => Storage::Storage,
            });
            let ty = resolved.map_or(
                ResolvedType { decodable: None, abi: AbiType::Unknown, library_spelled: false },
                |resolved| resolved[i].clone(),
            );
            SourceParam { slots: slots(&var.ty, storage), ty, storage }
        })
        .collect()
}

/// The number of stack words a value of the type takes: calldata arrays, `bytes` and `string` take
/// an offset and a length, and external function types an address and a selector.
fn slots(ty: &Type<'_>, storage: Option<Storage>) -> usize {
    match (&ty.kind, storage) {
        (TypeKind::Function(function), _)
            if function.visibility() == Some(Visibility::External) =>
        {
            2
        }
        (
            TypeKind::Elementary(ElementaryType::String | ElementaryType::Bytes),
            Some(Storage::Calldata),
        ) => 2,
        (TypeKind::Array(array), Some(Storage::Calldata)) if array.size.is_none() => 2,
        _ => 1,
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

    /// Parses `source` as `test.sol`, and lowers it if `lower`.
    fn compiler(source: &str, lower: bool) -> solar::sema::Compiler {
        use solar::{interface::Session, sema::Compiler};
        let mut compiler =
            Compiler::new(Session::builder().with_buffer_emitter(Default::default()).build());
        compiler.enter_mut(|compiler| {
            let file = compiler
                .sess()
                .source_map()
                .new_source_file(std::path::PathBuf::from("test.sol"), source)
                .unwrap();
            let mut parser = compiler.parse();
            parser.add_file(file);
            parser.parse();
            if lower {
                let _ = compiler.lower_asts();
            }
        });
        compiler
    }

    const TYPES: &str = "
type Amount is uint256;
enum Kind { A, B }
interface Token {}
struct Inner { uint8 x; }
struct Outer { Inner inner; address[] list; Amount amount; }
contract C {
    function f(
        Amount a,
        Kind k,
        Token t,
        Outer memory o,
        uint256[2][] calldata array,
        function(uint256) external callback,
        mapping(uint256 => uint256) storage map,
        string calldata text
    ) internal returns (Outer[] memory, int8) {}
}
";

    #[test]
    fn types_are_resolved_by_solar_to_the_types_of_the_abi() {
        let resolved = |lower| {
            compiler(TYPES, lower).enter(|compiler| {
                resolve_types(
                    compiler.gcx(),
                    compiler.sess().source_map(),
                    std::path::PathBuf::from("test.sol"),
                )
            })
        };
        let known = |name: &str| AbiType::Known(name.into());
        let at = TYPES.find("function f").unwrap();

        let resolved = resolved(true);
        let function = &resolved[&at];
        let abi =
            |types: &[ResolvedType]| types.iter().map(|ty| ty.abi.clone()).collect::<Vec<_>>();
        assert_eq!(
            abi(&function.params),
            [
                known("uint256"),
                known("uint8"),
                known("address"),
                known("((uint8),address[],uint256)"),
                known("uint256[2][]"),
                known("function"),
                AbiType::Unknown,
                known("string"),
            ]
        );
        assert_eq!(abi(&function.returns), [known("((uint8),address[],uint256)[]"), known("int8")]);
        // Only the values with words to decode are decoded: not structs, function types, or what
        // has no ABI type.
        let decodable = function.params.iter().map(|ty| ty.decodable.clone()).collect::<Vec<_>>();
        assert_eq!(
            decodable,
            [
                Some(DynSolType::Uint(256)),
                Some(DynSolType::Uint(8)),
                Some(DynSolType::Address),
                None,
                Some(DynSolType::Array(Box::new(DynSolType::FixedArray(
                    Box::new(DynSolType::Uint(256)),
                    2
                )))),
                None,
                None,
                Some(DynSolType::String),
            ]
        );
    }

    #[test]
    fn types_of_sources_that_are_not_lowered_are_unknown() {
        let resolved = compiler(TYPES, false).enter(|compiler| {
            resolve_types(
                compiler.gcx(),
                compiler.sess().source_map(),
                std::path::PathBuf::from("test.sol"),
            )
        });

        assert!(resolved.is_empty());
    }
}
