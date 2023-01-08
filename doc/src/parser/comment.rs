use derive_more::Deref;
use solang_parser::doccomment::DocCommentTag;
use std::str::FromStr;

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

impl FromStr for CommentTag {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
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
                CommentTag::Custom(trimmed.trim_start_matches("custom:").trim().to_owned())
            }
            _ => eyre::bail!("unknown comment tag: {trimmed}"),
        };
        Ok(tag)
    }
}

/// The natspec documentation comment.
/// https://docs.soliditylang.org/en/v0.8.17/natspec-format.html
#[derive(PartialEq, Debug)]
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

    /// Split the comment at first word.
    /// Useful for [CommentTag::Param] and [CommentTag::Return] comments.
    pub fn split_first_word<'a>(&'a self) -> Option<(&'a str, &'a str)> {
        self.value.trim_start().split_once(' ')
    }

    /// Match the first word of the comment with the expected.
    /// Returns [None] if the word doesn't match.
    /// Useful for [CommentTag::Param] and [CommentTag::Return] comments.
    pub fn match_first_word<'a>(&'a self, expected: &str) -> Option<&'a str> {
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

impl TryFrom<DocCommentTag> for Comment {
    type Error = eyre::Error;

    fn try_from(value: DocCommentTag) -> Result<Self, Self::Error> {
        let tag = CommentTag::from_str(&value.tag)?;
        Ok(Self { tag, value: value.value })
    }
}

/// The collection of natspec [Comment] items.
#[derive(Deref, PartialEq, Default, Debug)]
pub struct Comments(Vec<Comment>);

/// Forward the [Comments] function implementation to the [CommentsRef]
/// reference type.
macro_rules! ref_fn {
    ($vis:vis fn $name:ident(&self, $arg:ty)) => {
        /// Forward the function implementation to [CommentsRef] reference type.
        $vis fn $name<'a>(&'a self, arg: $arg) -> CommentsRef<'a> {
            CommentsRef::from(self).$name(arg)
        }
    };
}

impl Comments {
    ref_fn!(pub fn include_tag(&self, CommentTag));
    ref_fn!(pub fn include_tags(&self, &[CommentTag]));
    ref_fn!(pub fn exclude_tags(&self, &[CommentTag]));
}

impl TryFrom<Vec<DocCommentTag>> for Comments {
    type Error = eyre::Error;

    fn try_from(value: Vec<DocCommentTag>) -> Result<Self, Self::Error> {
        Ok(Self(value.into_iter().map(TryInto::try_into).collect::<Result<Vec<_>, _>>()?))
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
        assert_eq!(CommentTag::from_str("title").unwrap(), CommentTag::Title);
        assert_eq!(CommentTag::from_str(" title  ").unwrap(), CommentTag::Title);
        assert_eq!(CommentTag::from_str("author").unwrap(), CommentTag::Author);
        assert_eq!(CommentTag::from_str("notice").unwrap(), CommentTag::Notice);
        assert_eq!(CommentTag::from_str("dev").unwrap(), CommentTag::Dev);
        assert_eq!(CommentTag::from_str("param").unwrap(), CommentTag::Param);
        assert_eq!(CommentTag::from_str("return").unwrap(), CommentTag::Return);
        assert_eq!(CommentTag::from_str("inheritdoc").unwrap(), CommentTag::Inheritdoc);
        assert_eq!(CommentTag::from_str("custom:").unwrap(), CommentTag::Custom("".to_owned()));
        assert_eq!(
            CommentTag::from_str("custom:some").unwrap(),
            CommentTag::Custom("some".to_owned())
        );
        assert_eq!(
            CommentTag::from_str("  custom:   some   ").unwrap(),
            CommentTag::Custom("some".to_owned())
        );

        assert!(CommentTag::from_str("").is_err());
        assert!(CommentTag::from_str("custom").is_err());
        assert!(CommentTag::from_str("sometag").is_err());
    }
}
