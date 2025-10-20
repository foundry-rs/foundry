use crate::{Comments, helpers::function_signature, solang_ext::SafeUnwrap};
use solang_parser::pt::{
    ContractDefinition, ContractTy, EnumDefinition, ErrorDefinition, EventDefinition,
    FunctionDefinition, StructDefinition, TypeDefinition, VariableDefinition,
};
use std::ops::Range;

/// The parsed item.
#[derive(Debug, PartialEq)]
pub struct ParseItem {
    /// The parse tree source.
    pub source: ParseSource,
    /// Item comments.
    pub comments: Comments,
    /// Children items.
    pub children: Vec<Self>,
    /// Formatted code string.
    pub code: String,
}

/// Defines a method that filters [ParseItem]'s children and returns the source pt token of the
/// children matching the target variant as well as its comments.
/// Returns [Option::None] if no children matching the variant are found.
macro_rules! filter_children_fn {
    ($vis:vis fn $name:ident(&self, $variant:ident) -> $ret:ty) => {
        /// Filter children items for [ParseSource::$variant] variants.
        $vis fn $name(&self) -> Option<Vec<(&$ret, &Comments, &String)>> {
            let items = self.children.iter().filter_map(|item| match item.source {
                ParseSource::$variant(ref inner) => Some((inner, &item.comments, &item.code)),
                _ => None,
            });
            let items = items.collect::<Vec<_>>();
            if !items.is_empty() {
                Some(items)
            } else {
                None
            }
        }
    };
}

/// Defines a method that returns [ParseSource] inner element if it matches
/// the variant
macro_rules! as_inner_source {
    ($vis:vis fn $name:ident(&self, $variant:ident) -> $ret:ty) => {
        /// Return inner element if it matches $variant.
        /// If the element doesn't match, returns [None]
        $vis fn $name(&self) -> Option<&$ret> {
            match self.source {
                ParseSource::$variant(ref inner) => Some(inner),
                _ => None
            }
        }
    };
}

impl ParseItem {
    /// Create new instance of [ParseItem].
    pub fn new(source: ParseSource) -> Self {
        Self {
            source,
            comments: Default::default(),
            children: Default::default(),
            code: Default::default(),
        }
    }

    /// Set comments on the [ParseItem].
    pub fn with_comments(mut self, comments: Comments) -> Self {
        self.comments = comments;
        self
    }

    /// Set children on the [ParseItem].
    pub fn with_children(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self
    }

    /// Set the source code of this [ParseItem].
    ///
    /// The parameter should be the full source file where this parse item originated from.
    pub fn with_code(mut self, source: &str, tab_width: usize) -> Self {
        let mut code = source[self.source.range()].to_string();

        // Special function case, add `;` at the end of definition.
        if let ParseSource::Function(_) | ParseSource::Error(_) | ParseSource::Event(_) =
            self.source
        {
            code.push(';');
        }

        // Remove extra indent from source lines.
        let prefix = &" ".repeat(tab_width);
        self.code = code
            .lines()
            .map(|line| line.strip_prefix(prefix).unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        self
    }

    /// Format the item's filename.
    pub fn filename(&self) -> String {
        let prefix = match self.source {
            ParseSource::Contract(ref c) => match c.ty {
                ContractTy::Contract(_) => "contract",
                ContractTy::Abstract(_) => "abstract",
                ContractTy::Interface(_) => "interface",
                ContractTy::Library(_) => "library",
            },
            ParseSource::Function(_) => "function",
            ParseSource::Variable(_) => "variable",
            ParseSource::Event(_) => "event",
            ParseSource::Error(_) => "error",
            ParseSource::Struct(_) => "struct",
            ParseSource::Enum(_) => "enum",
            ParseSource::Type(_) => "type",
        };
        let ident = self.source.ident();
        format!("{prefix}.{ident}.md")
    }

    filter_children_fn!(pub fn variables(&self, Variable) -> VariableDefinition);
    filter_children_fn!(pub fn functions(&self, Function) -> FunctionDefinition);
    filter_children_fn!(pub fn events(&self, Event) -> EventDefinition);
    filter_children_fn!(pub fn errors(&self, Error) -> ErrorDefinition);
    filter_children_fn!(pub fn structs(&self, Struct) -> StructDefinition);
    filter_children_fn!(pub fn enums(&self, Enum) -> EnumDefinition);

    as_inner_source!(pub fn as_contract(&self, Contract) -> ContractDefinition);
    as_inner_source!(pub fn as_variable(&self, Variable) -> VariableDefinition);
    as_inner_source!(pub fn as_function(&self, Function) -> FunctionDefinition);
}

/// A wrapper type around pt token.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum ParseSource {
    /// Source contract definition.
    Contract(Box<ContractDefinition>),
    /// Source function definition.
    Function(FunctionDefinition),
    /// Source variable definition.
    Variable(VariableDefinition),
    /// Source event definition.
    Event(EventDefinition),
    /// Source error definition.
    Error(ErrorDefinition),
    /// Source struct definition.
    Struct(StructDefinition),
    /// Source enum definition.
    Enum(EnumDefinition),
    /// Source type definition.
    Type(TypeDefinition),
}

impl ParseSource {
    /// Get the identity of the source
    pub fn ident(&self) -> String {
        match self {
            Self::Contract(contract) => contract.name.safe_unwrap().name.to_owned(),
            Self::Variable(var) => var.name.safe_unwrap().name.to_owned(),
            Self::Event(event) => event.name.safe_unwrap().name.to_owned(),
            Self::Error(error) => error.name.safe_unwrap().name.to_owned(),
            Self::Struct(structure) => structure.name.safe_unwrap().name.to_owned(),
            Self::Enum(enumerable) => enumerable.name.safe_unwrap().name.to_owned(),
            Self::Function(func) => {
                func.name.as_ref().map_or(func.ty.to_string(), |n| n.name.to_owned())
            }
            Self::Type(ty) => ty.name.name.to_owned(),
        }
    }

    /// Get the signature of the source (for functions, includes parameter types)
    pub fn signature(&self) -> String {
        match self {
            Self::Function(func) => function_signature(func),
            _ => self.ident(),
        }
    }

    /// Get the range of this item in the source file.
    pub fn range(&self) -> Range<usize> {
        match self {
            Self::Contract(contract) => contract.loc,
            Self::Variable(var) => var.loc,
            Self::Event(event) => event.loc,
            Self::Error(error) => error.loc,
            Self::Struct(structure) => structure.loc,
            Self::Enum(enumerable) => enumerable.loc,
            Self::Function(func) => func.loc_prototype,
            Self::Type(ty) => ty.loc,
        }
        .range()
    }
}
