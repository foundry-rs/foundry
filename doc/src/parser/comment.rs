use derive_more::{Deref, DerefMut};
use solang_parser::doccomment::DocCommentTag;
use std::collections::HashMap;

/// The natspec comment tag explaining the purpose of the comment.
/// See: https://docs.soliditylang.org/en/v0.8.17/natspec-format.html#tags.
#[derive(PartialEq, Clone, Debug)]
pub enum CommentTag {
    /// A title that should describe the contract/interface
    Title,
    /// The name of the author
    Author,
    /// Explain to an end user what this does
    Notice,
    /// Explain to a developer any extra details
    Dev,
    /// Documents a parameter just like in Doxygen (must be followed by parameter name)
    Param,
    /// Documents the return variables of a contract’s function
    Return,
    /// Copies all missing tags from the base function (must be followed by the contract name)
    Inheritdoc,
    /// Custom tag, semantics is application-defined
    Custom(String),
}

impl CommentTag {
    fn from_str(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        let tag = match trimmed {
            "title" => CommentTag::Title,
            "author" => CommentTag::Author,
            "notice" => CommentTag::Notice,
            "dev" => CommentTag::Dev,
            "param" => CommentTag::Param,
            "return" => CommentTag::Return,
            "inheritdoc" => CommentTag::Inheritdoc,
            _ if trimmed.starts_with("custom:") => {
                // `@custom:param` tag will be parsed as `CommentTag::Param` due to a limitation
                // on specifying parameter docs for unnamed function arguments.
                let custom_tag = trimmed.trim_start_matches("custom:").trim();
                match custom_tag {
                    "param" => CommentTag::Param,
                    _ => CommentTag::Custom(custom_tag.to_owned()),
                }
            }
            _ => {
                tracing::warn!(target: "forge::doc", tag = trimmed, "unknown comment tag. custom tags must be preceded by `custom:`");
                return None
            }
        };
        Some(tag)
    }
}

/// The natspec documentation comment.
/// https://docs.soliditylang.org/en/v0.8.17/natspec-format.html
#[derive(PartialEq, Clone, Debug)]
pub struct Comment {
    /// The doc comment tag.
    pub tag: CommentTag,
    /// The doc comment value.
    pub value: String,
}

impl Comment {
    /// Create new instance of [Comment].
    pub fn new(tag: CommentTag, value: String) -> Self {
        Self { tag, value }
    }

    /// Create new instance of [Comment] from [DocCommentTag]
    /// if it has a valid natspec tag.
    pub fn from_doc_comment(value: DocCommentTag) -> Option<Self> {
        CommentTag::from_str(&value.tag).map(|tag| Self { tag, value: value.value })
    }

    /// Split the comment at first word.
    /// Useful for [CommentTag::Param] and [CommentTag::Return] comments.
    pub fn split_first_word(&self) -> Option<(&str, &str)> {
        self.value.trim_start().split_once(' ')
    }

    /// Match the first word of the comment with the expected.
    /// Returns [None] if the word doesn't match.
    /// Useful for [CommentTag::Param] and [CommentTag::Return] comments.
    pub fn match_first_word(&self, expected: &str) -> Option<&str> {
        self.split_first_word().and_then(
            |(word, rest)| {
                if word.eq(expected) {
                    Some(rest)
                } else {
                    None
                }
            },
        )
    }
}

/// The collection of natspec [Comment] items.
#[derive(Deref, DerefMut, PartialEq, Default, Clone, Debug)]
pub struct Comments(Vec<Comment>);

/// Forward the [Comments] function implementation to the [CommentsRef]
/// reference type.
macro_rules! ref_fn {
    ($vis:vis fn $name:ident(&self$(, )?$($arg_name:ident: $arg:ty),*) -> $ret:ty) => {
        /// Forward the function implementation to [CommentsRef] reference type.
        $vis fn $name(&self, $($arg_name: $arg),*) -> $ret {
            CommentsRef::from(self).$name($($arg_name),*)
        }
    };
}

impl Comments {
    ref_fn!(pub fn include_tag(&self, tag: CommentTag) -> CommentsRef<'_>);
    ref_fn!(pub fn include_tags(&self, tags: &[CommentTag]) -> CommentsRef<'_>);
    ref_fn!(pub fn exclude_tags(&self, tags: &[CommentTag]) -> CommentsRef<'_>);
    ref_fn!(pub fn contains_tag(&self, tag: &Comment) -> bool);
    ref_fn!(pub fn find_inheritdoc_base(&self) -> Option<&'_ str>);

    /// Attempt to lookup
    ///
    /// Merges two comments collections by inserting [CommentTag] from the second collection
    /// into the first unless they are present.
    pub fn merge_inheritdoc(
        &self,
        ident: &str,
        inheritdocs: Option<HashMap<String, Comments>>,
    ) -> Comments {
        let mut result = Comments(Vec::from_iter(self.iter().cloned()));

        if let (Some(inheritdocs), Some(base)) = (inheritdocs, self.find_inheritdoc_base()) {
            let key = format!("{base}.{ident}");
            if let Some(other) = inheritdocs.get(&key) {
                for comment in other.iter() {
                    if !result.contains_tag(comment) {
                        result.push(comment.clone());
                    }
                }
            }
        }

        result
    }
}

impl From<Vec<DocCommentTag>> for Comments {
    fn from(value: Vec<DocCommentTag>) -> Self {
        Self(value.into_iter().flat_map(Comment::from_doc_comment).collect())
    }
}

/// The collection of references to natspec [Comment] items.
#[derive(Deref, PartialEq, Default, Debug)]
pub struct CommentsRef<'a>(Vec<&'a Comment>);

impl<'a> CommentsRef<'a> {
    /// Filter a collection of comments and return only those that match a provided tag
    pub fn include_tag(&self, tag: CommentTag) -> CommentsRef<'a> {
        self.include_tags(&[tag])
    }

    /// Filter a collection of comments and return only those that match provided tags
    pub fn include_tags(&self, tags: &[CommentTag]) -> CommentsRef<'a> {
        // Cloning only references here
        CommentsRef(self.iter().cloned().filter(|c| tags.contains(&c.tag)).collect())
    }

    /// Filter a collection of comments and return  only those that do not match provided tags
    pub fn exclude_tags(&self, tags: &[CommentTag]) -> CommentsRef<'a> {
        // Cloning only references here
        CommentsRef(self.iter().cloned().filter(|c| !tags.contains(&c.tag)).collect())
    }

    /// Check if the collection contains a target comment.
    pub fn contains_tag(&self, target: &Comment) -> bool {
        self.iter().any(|c| match (&c.tag, &target.tag) {
            (CommentTag::Inheritdoc, CommentTag::Inheritdoc) => c.value == target.value,
            (CommentTag::Param, CommentTag::Param) | (CommentTag::Return, CommentTag::Return) => {
                c.split_first_word().map(|(name, _)| name) ==
                    target.split_first_word().map(|(name, _)| name)
            }
            (tag1, tag2) => tag1 == tag2,
        })
    }

    /// Find an [CommentTag::Inheritdoc] comment and extract the base.
    fn find_inheritdoc_base(&self) -> Option<&'a str> {
        self.iter()
            .find(|c| matches!(c.tag, CommentTag::Inheritdoc))
            .and_then(|c| c.value.split_whitespace().next())
    }
}

impl<'a> From<&'a Comments> for CommentsRef<'a> {
    fn from(value: &'a Comments) -> Self {
        Self(value.iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_comment_tag() {
        assert_eq!(CommentTag::from_str("title"), Some(CommentTag::Title));
        assert_eq!(CommentTag::from_str(" title  "), Some(CommentTag::Title));
        assert_eq!(CommentTag::from_str("author"), Some(CommentTag::Author));
        assert_eq!(CommentTag::from_str("notice"), Some(CommentTag::Notice));
        assert_eq!(CommentTag::from_str("dev"), Some(CommentTag::Dev));
        assert_eq!(CommentTag::from_str("param"), Some(CommentTag::Param));
        assert_eq!(CommentTag::from_str("return"), Some(CommentTag::Return));
        assert_eq!(CommentTag::from_str("inheritdoc"), Some(CommentTag::Inheritdoc));
        assert_eq!(CommentTag::from_str("custom:"), Some(CommentTag::Custom("".to_owned())));
        assert_eq!(
            CommentTag::from_str("custom:some"),
            Some(CommentTag::Custom("some".to_owned()))
        );
        assert_eq!(
            CommentTag::from_str("  custom:   some   "),
            Some(CommentTag::Custom("some".to_owned()))
        );

        assert_eq!(CommentTag::from_str(""), None);
        assert_eq!(CommentTag::from_str("custom"), None);
        assert_eq!(CommentTag::from_str("sometag"), None);
    }
}
