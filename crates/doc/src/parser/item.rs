use crate::Comments;

pub use super::source::{
    ContractSource, EnumSource, ErrorSource, EventSource, FunctionSource, ParseSource,
    StructSource, VariableSource,
};

/// The parsed item.
#[derive(Clone, Debug, PartialEq)]
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

/// Defines a method that filters [ParseItem]'s children and returns the source data of the
/// children matching the target variant as well as its comments.
/// Returns [Option::None] if no children matching the variant are found.
macro_rules! filter_children_fn {
    ($vis:vis fn $name:ident(&self, $variant:ident) -> $ret:ty) => {
        /// Filter children items for matching variants.
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

/// Defines a method that returns the inner element if it matches the variant.
macro_rules! as_inner_source {
    ($vis:vis fn $name:ident(&self, $variant:ident) -> $ret:ty) => {
        /// Return inner element if it matches the variant.
        /// If the element doesn't match, returns [None].
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

    /// Set the code string for this [ParseItem].
    pub fn with_code(mut self, code: String) -> Self {
        self.code = code;
        self
    }

    /// Format the item's filename.
    pub fn filename(&self) -> String {
        let prefix = match &self.source {
            ParseSource::Contract(c) => c.kind.as_str(),
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

    filter_children_fn!(pub fn variables(&self, Variable) -> VariableSource);
    filter_children_fn!(pub fn functions(&self, Function) -> FunctionSource);
    filter_children_fn!(pub fn events(&self, Event) -> EventSource);
    filter_children_fn!(pub fn errors(&self, Error) -> ErrorSource);
    filter_children_fn!(pub fn structs(&self, Struct) -> StructSource);
    filter_children_fn!(pub fn enums(&self, Enum) -> EnumSource);

    as_inner_source!(pub fn as_contract(&self, Contract) -> ContractSource);
    as_inner_source!(pub fn as_variable(&self, Variable) -> VariableSource);
    as_inner_source!(pub fn as_function(&self, Function) -> FunctionSource);
}
