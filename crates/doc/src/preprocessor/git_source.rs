use super::{Preprocessor, PreprocessorId};
use crate::{Document, PreprocessorOutput};
use std::path::PathBuf;

/// [GitSource] preprocessor id.
pub const GIT_SOURCE_ID: PreprocessorId = PreprocessorId("git_source");

/// The git source preprocessor.
///
/// This preprocessor writes to [Document]'s context.
#[derive(Debug)]
pub struct GitSource {
    /// The project root.
    pub root: PathBuf,
    /// The current commit hash.
    pub commit: Option<String>,
    /// The repository url.
    pub repository: Option<String>,
}

impl Preprocessor for GitSource {
    fn id(&self) -> PreprocessorId {
        GIT_SOURCE_ID
    }

    fn preprocess(&self, documents: Vec<Document>) -> Result<Vec<Document>, eyre::Error> {
        if let Some(ref repo) = self.repository {
            let repo = repo.trim_end_matches('/');
            let commit = self.commit.clone().unwrap_or("master".to_owned());
            for document in documents.iter() {
                let git_url = format!(
                    "{repo}/blob/{commit}/{}",
                    document.item_path.strip_prefix(&self.root)?.display()
                );
                document.add_context(self.id(), PreprocessorOutput::GitSource(git_url));
            }
        }

        Ok(documents)
    }
}
