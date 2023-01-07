use solang_parser::{
    doccomment::DocCommentTag,
    pt::{
        ContractDefinition, EnumDefinition, ErrorDefinition, EventDefinition, FunctionDefinition,
        StructDefinition, VariableDefinition,
    },
};

/// The parsed item.
#[derive(Debug, PartialEq)]
pub struct ParseItem {
    /// The parse tree source.
    pub source: ParseSource,
    /// Item comments.
    pub comments: Vec<DocCommentTag>,
    /// Children items.
    pub children: Vec<ParseItem>,
}

/// Defines a method that filters [ParseItem]'s children and returns the source pt token of the
/// children matching the target variant as well as its comments.
/// Returns [Option::None] if no children matching the variant are found.
macro_rules! filter_children_fn {
    ($vis:vis fn $name:ident(&self, $variant:ident) -> $ret:ty) => {
        /// Filter children items for [ParseSource::$variant] variants.
        $vis fn $name<'a>(&'a self) -> Option<Vec<(&'a $ret, &'a Vec<DocCommentTag>)>> {
            let items = self.children.iter().filter_map(|item| match item.source {
                ParseSource::$variant(ref inner) => Some((inner, &item.comments)),
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
        Self { source, comments: Default::default(), children: Default::default() }
    }

    /// Set comments on the [ParseItem].
    pub fn with_comments(mut self, comments: Vec<DocCommentTag>) -> Self {
        self.comments = comments;
        self
    }

    /// Return item comments
    pub fn comments(&self) -> &Vec<DocCommentTag> {
        &self.comments
    }

    /// Format the item's filename.
    pub fn filename(&self) -> String {
        let prefix = match self.source {
            ParseSource::Contract(_) => "contract",
            ParseSource::Function(_) => "function",
            ParseSource::Variable(_) => "variable",
            ParseSource::Event(_) => "event",
            ParseSource::Error(_) => "error",
            ParseSource::Struct(_) => "struct",
            ParseSource::Enum(_) => "enum",
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
}

/// A wrapper type around pt token.
#[derive(Debug, PartialEq)]
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
}

impl ParseSource {
    /// Get the identity of the source
    pub fn ident(&self) -> String {
        match self {
            ParseSource::Contract(contract) => contract.name.name.to_owned(),
            ParseSource::Variable(var) => var.name.name.to_owned(),
            ParseSource::Event(event) => event.name.name.to_owned(),
            ParseSource::Error(error) => error.name.name.to_owned(),
            ParseSource::Struct(structure) => structure.name.name.to_owned(),
            ParseSource::Enum(enumerable) => enumerable.name.name.to_owned(),
            ParseSource::Function(func) => {
                func.name.as_ref().map_or(func.ty.to_string(), |n| n.name.to_owned())
            }
        }
    }
}
