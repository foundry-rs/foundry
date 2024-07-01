use crate::{DocBuilder, ParseItem, PreprocessorId, PreprocessorOutput};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    slice::IterMut,
    sync::Mutex,
};

/// The wrapper around the [ParseItem] containing additional
/// information the original item and extra context for outputting it.
#[derive(Debug)]
pub struct Document {
    /// The underlying parsed items.
    pub content: DocumentContent,
    /// The original item path.
    pub item_path: PathBuf,
    /// The original item file content.
    pub item_content: String,
    /// The target path where the document will be written.
    pub target_path: PathBuf,
    /// The document display identity.
    pub identity: String,
    /// The preprocessors results.
    context: Mutex<HashMap<PreprocessorId, PreprocessorOutput>>,
    /// Whether the document is from external library.
    pub from_library: bool,
    /// The target directory for the doc output.
    pub out_target_dir: PathBuf,
}

impl Document {
    /// Create new instance of [Document].
    pub fn new(
        item_path: PathBuf,
        target_path: PathBuf,
        from_library: bool,
        out_target_dir: PathBuf,
    ) -> Self {
        Self {
            item_path,
            target_path,
            from_library,
            item_content: String::default(),
            identity: String::default(),
            content: DocumentContent::Empty,
            out_target_dir,
            context: Mutex::new(HashMap::default()),
        }
    }

    /// Set content and identity on the [Document].
    #[must_use]
    pub fn with_content(mut self, content: DocumentContent, identity: String) -> Self {
        self.content = content;
        self.identity = identity;
        self
    }

    /// Add a preprocessor result to inner document context.
    pub fn add_context(&self, id: PreprocessorId, output: PreprocessorOutput) {
        let mut context = self.context.lock().expect("failed to lock context");
        context.insert(id, output);
    }

    /// Read preprocessor result from context
    pub fn get_from_context(&self, id: PreprocessorId) -> Option<PreprocessorOutput> {
        let context = self.context.lock().expect("failed to lock context");
        context.get(&id).cloned()
    }

    fn try_relative_output_path(&self) -> Option<&Path> {
        self.target_path.strip_prefix(&self.out_target_dir).ok()?.strip_prefix(DocBuilder::SRC).ok()
    }

    /// Returns the relative path of the document output.
    pub fn relative_output_path(&self) -> &Path {
        self.try_relative_output_path().unwrap_or(self.target_path.as_path())
    }
}

/// The content of the document.
#[derive(Debug)]
pub enum DocumentContent {
    Empty,
    Single(ParseItem),
    Constants(Vec<ParseItem>),
    OverloadedFunctions(Vec<ParseItem>),
}

impl DocumentContent {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Single(_) => 1,
            Self::Constants(items) => items.len(),
            Self::OverloadedFunctions(items) => items.len(),
        }
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut ParseItem> {
        match self {
            Self::Empty => None,
            Self::Single(item) => {
                if index == 0 {
                    Some(item)
                } else {
                    None
                }
            }
            Self::Constants(items) => items.get_mut(index),
            Self::OverloadedFunctions(items) => items.get_mut(index),
        }
    }

    pub fn iter_items(&self) -> ParseItemIter<'_> {
        match self {
            Self::Empty => ParseItemIter { next: None, other: None },
            Self::Single(item) => ParseItemIter { next: Some(item), other: None },
            Self::Constants(items) => ParseItemIter { next: None, other: Some(items.iter()) },
            Self::OverloadedFunctions(items) => {
                ParseItemIter { next: None, other: Some(items.iter()) }
            }
        }
    }

    pub fn iter_items_mut(&mut self) -> ParseItemIterMut<'_> {
        match self {
            Self::Empty => ParseItemIterMut { next: None, other: None },
            Self::Single(item) => ParseItemIterMut { next: Some(item), other: None },
            Self::Constants(items) => {
                ParseItemIterMut { next: None, other: Some(items.iter_mut()) }
            }
            Self::OverloadedFunctions(items) => {
                ParseItemIterMut { next: None, other: Some(items.iter_mut()) }
            }
        }
    }
}

#[derive(Debug)]
pub struct ParseItemIter<'a> {
    next: Option<&'a ParseItem>,
    other: Option<std::slice::Iter<'a, ParseItem>>,
}

impl<'a> Iterator for ParseItemIter<'a> {
    type Item = &'a ParseItem;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(next) = self.next.take() {
            return Some(next)
        }
        if let Some(other) = self.other.as_mut() {
            return other.next()
        }

        None
    }
}

#[derive(Debug)]
pub struct ParseItemIterMut<'a> {
    next: Option<&'a mut ParseItem>,
    other: Option<IterMut<'a, ParseItem>>,
}

impl<'a> Iterator for ParseItemIterMut<'a> {
    type Item = &'a mut ParseItem;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(next) = self.next.take() {
            return Some(next)
        }
        if let Some(other) = self.other.as_mut() {
            return other.next()
        }

        None
    }
}

/// Read the preprocessor output variant from document context.
/// Returns [None] if there is no output.
macro_rules! read_context {
    ($doc: expr, $id: expr, $variant: ident) => {
        $doc.get_from_context($id).and_then(|out| match out {
            // Only a single variant is matched. Otherwise the code is invalid.
            PreprocessorOutput::$variant(inner) => Some(inner),
            _ => None,
        })
    };
}

pub(crate) use read_context;
