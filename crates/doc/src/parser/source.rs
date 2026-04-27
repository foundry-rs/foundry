//! Owned source types extracted from the parse tree for documentation generation.
//!
//! These types hold only the data needed by the doc writer and preprocessors,
//! avoiding lifetime dependencies on the parser's arena-allocated AST.

/// Information about a parameter, struct field, event/error parameter, etc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamInfo {
    /// The parameter name, if any.
    pub name: Option<String>,
    /// The type rendered as a string (e.g. `"uint256"`, `"address"`).
    pub ty: String,
}

/// A base contract reference (e.g. `is IERC721, Ownable`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseInfo {
    /// The full dotted name (e.g. `"IERC721"` or `"SomeLib.Base"`).
    pub name: String,
    /// The last identifier in the path, used for linking.
    pub ident: String,
}

/// The kind of a contract-like definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractKind {
    Contract,
    Abstract,
    Interface,
    Library,
}

impl ContractKind {
    /// Returns the lowercase keyword string.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Abstract => "abstract",
            Self::Interface => "interface",
            Self::Library => "library",
        }
    }
}

/// Owned contract definition data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractSource {
    pub name: String,
    pub kind: ContractKind,
    pub bases: Vec<BaseInfo>,
}

/// Owned function definition data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionSource {
    /// The function name, or `None` for unnamed functions (fallback/receive).
    pub name: Option<String>,
    /// The function kind as a display string (e.g. `"function"`, `"constructor"`, `"fallback"`).
    pub kind: String,
    /// Function parameters.
    pub params: Vec<ParamInfo>,
    /// Return parameters.
    pub returns: Vec<ParamInfo>,
}

impl FunctionSource {
    /// Get the signature of the function, including parameter types.
    pub fn signature(&self) -> String {
        let name = self.name.as_deref().unwrap_or(&self.kind);
        if self.params.is_empty() {
            return name.to_string();
        }
        format!(
            "{}({})",
            name,
            self.params.iter().map(|p| p.ty.as_str()).collect::<Vec<_>>().join(",")
        )
    }
}

/// Variable attribute relevant for doc generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariableAttr {
    Constant,
    Immutable,
}

/// Owned variable definition data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariableSource {
    pub name: String,
    pub attrs: Vec<VariableAttr>,
}

/// Owned event definition data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventSource {
    pub name: String,
    pub fields: Vec<ParamInfo>,
}

/// Owned error definition data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorSource {
    pub name: String,
    pub fields: Vec<ParamInfo>,
}

/// Owned struct definition data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructSource {
    pub name: String,
    pub fields: Vec<ParamInfo>,
}

/// Owned enum definition data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumSource {
    pub name: String,
    pub variants: Vec<String>,
}

/// Owned type definition data (user-defined value types).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeSource {
    pub name: String,
}

/// A wrapper type around owned source data extracted from the parse tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseSource {
    /// Source contract definition.
    Contract(ContractSource),
    /// Source function definition.
    Function(FunctionSource),
    /// Source variable definition.
    Variable(VariableSource),
    /// Source event definition.
    Event(EventSource),
    /// Source error definition.
    Error(ErrorSource),
    /// Source struct definition.
    Struct(StructSource),
    /// Source enum definition.
    Enum(EnumSource),
    /// Source type definition.
    Type(TypeSource),
}

impl ParseSource {
    /// Get the identity of the source.
    pub fn ident(&self) -> String {
        match self {
            Self::Contract(c) => c.name.clone(),
            Self::Variable(v) => v.name.clone(),
            Self::Event(e) => e.name.clone(),
            Self::Error(e) => e.name.clone(),
            Self::Struct(s) => s.name.clone(),
            Self::Enum(e) => e.name.clone(),
            Self::Function(f) => f.name.clone().unwrap_or_else(|| f.kind.clone()),
            Self::Type(t) => t.name.clone(),
        }
    }

    /// Get the signature of the source (for functions, includes parameter types).
    pub fn signature(&self) -> String {
        match self {
            Self::Function(f) => f.signature(),
            _ => self.ident(),
        }
    }
}
