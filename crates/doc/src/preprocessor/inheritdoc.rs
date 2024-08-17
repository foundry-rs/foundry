use super::{Preprocessor, PreprocessorId};
use crate::{
    document::DocumentContent, Comments, Document, ParseItem, ParseSource, PreprocessorOutput,
};
use forge_fmt::solang_ext::SafeUnwrap;
use std::collections::HashMap;

/// [`Inheritdoc`] preprocessor ID.
pub const INHERITDOC_ID: PreprocessorId = PreprocessorId("inheritdoc");

/// The inheritdoc preprocessor.
/// Traverses the documents and attempts to find inherited
/// comments for inheritdoc comment tags.
///
/// This preprocessor writes to [Document]'s context.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct Inheritdoc;

impl Preprocessor for Inheritdoc {
    fn id(&self) -> PreprocessorId {
        INHERITDOC_ID
    }

    fn preprocess(&self, documents: Vec<Document>) -> Result<Vec<Document>, eyre::Error> {
        for document in documents.iter() {
            if let DocumentContent::Single(ref item) = document.content {
                let context = self.visit_item(item, &documents);
                if !context.is_empty() {
                    document.add_context(self.id(), PreprocessorOutput::Inheritdoc(context));
                }
            }
        }

        Ok(documents)
    }
}

impl Inheritdoc {
    fn visit_item(&self, item: &ParseItem, documents: &Vec<Document>) -> HashMap<String, Comments> {
        let mut context = HashMap::default();

        // Match for the item first.
        let matched = item
            .comments
            .find_inheritdoc_base()
            .and_then(|base| self.try_match_inheritdoc(base, &item.source, documents));
        if let Some((key, comments)) = matched {
            context.insert(key, comments);
        }

        // Match item's children.
        for ch in item.children.iter() {
            let matched = ch
                .comments
                .find_inheritdoc_base()
                .and_then(|base| self.try_match_inheritdoc(base, &ch.source, documents));
            if let Some((key, comments)) = matched {
                context.insert(key, comments);
            }
        }

        context
    }

    fn try_match_inheritdoc(
        &self,
        base: &str,
        source: &ParseSource,
        documents: &Vec<Document>,
    ) -> Option<(String, Comments)> {
        for candidate in documents {
            if let DocumentContent::Single(ref item) = candidate.content {
                if let ParseSource::Contract(ref contract) = item.source {
                    if base == contract.name.safe_unwrap().name {
                        // Not matched for the contract because it's a noop
                        // https://docs.soliditylang.org/en/v0.8.17/natspec-format.html#tags

                        for children in item.children.iter() {
                            // TODO: improve matching logic
                            if source.ident() == children.source.ident() {
                                let key = format!("{}.{}", base, source.ident());
                                return Some((key, children.comments.clone()))
                            }
                        }
                    }
                }
            }
        }
        None
    }
}
