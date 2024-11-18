use crate::comments::CommentWithMetadata;

/// Holds information about a non-whitespace-splittable string, and the surrounding comments
#[derive(Clone, Debug, Default)]
pub struct Chunk {
    pub postfixes_before: Vec<CommentWithMetadata>,
    pub prefixes: Vec<CommentWithMetadata>,
    pub content: String,
    pub postfixes: Vec<CommentWithMetadata>,
    pub needs_space: Option<bool>,
}

impl From<String> for Chunk {
    fn from(string: String) -> Self {
        Self { content: string, ..Default::default() }
    }
}

impl From<&str> for Chunk {
    fn from(string: &str) -> Self {
        Self { content: string.to_owned(), ..Default::default() }
    }
}

// The struct with information about chunks used in the [Formatter::surrounded] method
#[derive(Debug)]
pub struct SurroundingChunk {
    pub before: Option<usize>,
    pub next: Option<usize>,
    pub spaced: Option<bool>,
    pub content: String,
}

impl SurroundingChunk {
    pub fn new(
        content: impl std::fmt::Display,
        before: Option<usize>,
        next: Option<usize>,
    ) -> Self {
        Self { before, next, content: format!("{content}"), spaced: None }
    }

    pub fn spaced(mut self) -> Self {
        self.spaced = Some(true);
        self
    }

    pub fn non_spaced(mut self) -> Self {
        self.spaced = Some(false);
        self
    }

    pub fn loc_before(&self) -> usize {
        self.before.unwrap_or_default()
    }

    pub fn loc_next(&self) -> Option<usize> {
        self.next
    }
}
