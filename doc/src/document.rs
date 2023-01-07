use derive_more::Deref;
use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use crate::{ParseItem, PreprocessorId, PreprocessorOutput};

/// The wrapper around the [ParseItem] containing additional
/// information the original item and extra context for outputting it.
#[derive(Debug, Deref)]
pub struct Document {
    /// The underlying parsed item.
    #[deref]
    pub item: ParseItem,
    /// The original item path.
    pub item_path: PathBuf,
    /// The target path where the document will be written.
    pub target_path: PathBuf,
    /// The preprocessors results.
    context: Mutex<HashMap<PreprocessorId, PreprocessorOutput>>,
}

impl Document {
    /// Create new instance of [Document].
    pub fn new(item: ParseItem, item_path: PathBuf, target_path: PathBuf) -> Self {
        Self { item, item_path, target_path, context: Mutex::new(HashMap::default()) }
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

/// TODO: docs
macro_rules! read_context {
    ($doc: expr, $id: expr, $variant: ident) => {
        $doc.get_from_context($id).map(|out| match out {
            // Only a single variant is matched. Otherwise the code is invalid.
            PreprocessorOutput::$variant(inner) => inner,
        })
    };
}

pub(crate) use read_context;
