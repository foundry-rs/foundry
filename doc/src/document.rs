use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use crate::{ParseItem, PreprocessorId, PreprocessorOutput};

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
}

/// The content of the document.
#[derive(Debug)]
pub enum DocumentContent {
    Empty,
    Single(ParseItem),
    Constants(Vec<ParseItem>),
    OverloadedFunctions(Vec<ParseItem>),
}

impl Document {
    /// Create new instance of [Document].
    pub fn new(item_path: PathBuf, target_path: PathBuf) -> Self {
        Self {
            item_path,
            target_path,
            item_content: String::default(),
            identity: String::default(),
            content: DocumentContent::Empty,
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
