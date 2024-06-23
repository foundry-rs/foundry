use crate::{error::ParserResult, Comments};
use forge_fmt::{
    solang_ext::SafeUnwrap, Comments as FmtComments, Formatter, FormatterConfig, InlineConfig,
    Visitor,
};
use solang_parser::pt::{
    ContractDefinition, ContractTy, EnumDefinition, ErrorDefinition, EventDefinition,
    FunctionDefinition, StructDefinition, TypeDefinition, VariableDefinition,
};

/// The parsed item.
#[derive(Debug, PartialEq)]
pub struct ParseItem {
    /// The parse tree source.
    pub source: ParseSource,
    /// Item comments.
    pub comments: Comments,
    /// Children items.
    pub children: Vec<ParseItem>,
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

    /// Set formatted code on the [ParseItem].
    pub fn with_code(mut self, source: &str, config: FormatterConfig) -> ParserResult<Self> {
        let mut code = String::new();
        let mut fmt = Formatter::new(
            &mut code,
            source,
            FmtComments::default(),
            InlineConfig::default(),
            config,
        );

        match self.source.clone() {
            ParseSource::Contract(mut contract) => {
                contract.parts = vec![];
                fmt.visit_contract(&mut contract)?
            }
            ParseSource::Function(mut func) => {
                func.body = None;
                fmt.visit_function(&mut func)?
            }
            ParseSource::Variable(mut var) => fmt.visit_var_definition(&mut var)?,
            ParseSource::Event(mut event) => fmt.visit_event(&mut event)?,
            ParseSource::Error(mut error) => fmt.visit_error(&mut error)?,
            ParseSource::Struct(mut structure) => fmt.visit_struct(&mut structure)?,
            ParseSource::Enum(mut enumeration) => fmt.visit_enum(&mut enumeration)?,
            ParseSource::Type(mut ty) => fmt.visit_type_definition(&mut ty)?,
        };

        self.code = code;

        Ok(self)
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
}
