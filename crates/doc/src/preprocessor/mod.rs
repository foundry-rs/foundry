//! Module containing documentation preprocessors.

use crate::{Comments, Document};
use alloy_primitives::map::HashMap;
use std::{fmt::Debug, path::PathBuf};

mod contract_inheritance;
pub use contract_inheritance::{CONTRACT_INHERITANCE_ID, ContractInheritance};

mod inheritdoc;
pub use inheritdoc::{INHERITDOC_ID, Inheritdoc};

mod infer_hyperlinks;
pub use infer_hyperlinks::{INFER_INLINE_HYPERLINKS_ID, InferInlineHyperlinks};

mod git_source;
pub use git_source::{GIT_SOURCE_ID, GitSource};

mod deployments;
pub use deployments::{DEPLOYMENTS_ID, Deployment, Deployments};

/// The preprocessor id.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct PreprocessorId(&'static str);

/// Preprocessor output.
/// Wraps all existing preprocessor outputs
/// in a single abstraction.
#[derive(Clone, Debug)]
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
    /// The deployments output.
    /// The deployment address of the item path.
    Deployments(Vec<Deployment>),
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
