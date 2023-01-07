//! Module containing documentation preprocessors.

mod contract_inheritance;
use std::{collections::HashMap, fmt::Debug, path::PathBuf};

pub use contract_inheritance::{ContractInheritance, CONTRACT_INHERITANCE_ID};

use crate::Document;

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
}

/// Trait for preprocessing and/or modifying existing documents
/// before writing the to disk.
pub trait Preprocessor: Debug {
    // TODO:
    /// Preprocessor must specify the output type it's writing to the
    /// document context. This is the inner type in [PreprocessorOutput] variants.
    // type Output = ();

    /// The id of the preprocessor.
    /// Used to write data to document context.
    fn id(&self) -> PreprocessorId;

    /// Preprocess the collection of documents
    fn preprocess(&self, documents: Vec<Document>) -> Result<Vec<Document>, eyre::Error>;
}
