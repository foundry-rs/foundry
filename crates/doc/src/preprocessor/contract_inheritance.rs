use super::{Preprocessor, PreprocessorId};
use crate::{Document, ParseSource, PreprocessorOutput, document::DocumentContent};
use alloy_primitives::map::HashMap;
use forge_fmt::solang_ext::SafeUnwrap;
use std::path::PathBuf;

/// [ContractInheritance] preprocessor id.
pub const CONTRACT_INHERITANCE_ID: PreprocessorId = PreprocessorId("contract_inheritance");

/// The contract inheritance preprocessor.
///
/// It matches the documents with inner [`ParseSource::Contract`](crate::ParseSource) elements,
/// iterates over their [Base](solang_parser::pt::Base)s and attempts
/// to link them with the paths of the other contract documents.
///
/// This preprocessor writes to [Document]'s context.
#[derive(Debug, Default)]
pub struct ContractInheritance {
    /// Whether to capture inherited contracts from libraries.
    pub include_libraries: bool,
}

impl Preprocessor for ContractInheritance {
    fn id(&self) -> PreprocessorId {
        CONTRACT_INHERITANCE_ID
    }

    fn preprocess(&self, documents: Vec<Document>) -> Result<Vec<Document>, eyre::Error> {
        for document in &documents {
            if let DocumentContent::Single(ref item) = document.content
                && let ParseSource::Contract(ref contract) = item.source
            {
                let mut links = HashMap::default();

                // Attempt to match bases to other contracts
                for base in &contract.base {
                    let base_ident = base.name.identifiers.last().unwrap().name.clone();
                    if let Some(linked) = self.try_link_base(&base_ident, &documents) {
                        links.insert(base_ident, linked);
                    }
                }

                if !links.is_empty() {
                    // Write to context
                    document.add_context(self.id(), PreprocessorOutput::ContractInheritance(links));
                }
            }
        }

        Ok(documents)
    }
}

impl ContractInheritance {
    fn try_link_base(&self, base: &str, documents: &Vec<Document>) -> Option<PathBuf> {
        for candidate in documents {
            if candidate.from_library && !self.include_libraries {
                continue;
            }
            if let DocumentContent::Single(ref item) = candidate.content
                && let ParseSource::Contract(ref contract) = item.source
                && base == contract.name.safe_unwrap().name
            {
                return Some(candidate.target_path.clone());
            }
        }
        None
    }
}
