//! Module containing documentation preprocessors.

use crate::{Comments, Document};
use std::{collections::HashMap, fmt::Debug, path::PathBuf};

mod contract_inheritance;
pub use contract_inheritance::{ContractInheritance, CONTRACT_INHERITANCE_ID};

mod inheritdoc;
pub use inheritdoc::{Inheritdoc, INHERITDOC_ID};

mod git_source;
pub use git_source::{GitSource, GIT_SOURCE_ID};

/// The preprocessor id.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct PreprocessorId(&'static str);

/// Preprocessor output.
/// Wraps all exisiting preprocessor outputs
/// in a single abstraction.
#[derive(Debug, Clone)]
pub enum PreprocessorOutput {
    /// The contract inheritance output.
    /// The map of contract base idents to the path of the base contract.
    ContractInheritance(HashMap<String, PathBuf>),
    /// The inheritdoc output.
    /// The map of inherited item keys to their comments.
    Inheritdoc(HashMap<String, Comments>),
    /// The git source output.
    /// The git url of the item path.
    GitSource(String),
}

/// Trait for preprocessing and/or modifying existing documents
/// before writing the to disk.
pub trait Preprocessor: Debug {
    /// The id of the preprocessor.
    /// Used to write data to document context.
    fn id(&self) -> PreprocessorId;

    /// Preprocess the collection of documents
    fn preprocess(&self, documents: Vec<Document>) -> Result<Vec<Document>, eyre::Error>;
}
