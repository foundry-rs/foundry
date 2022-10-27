use solang_parser::doccomment::DocCommentTag;

/// Filter a collection of comments and return
/// only those that match a given tag
pub(crate) fn filter_comments_by_tag<'a>(
    comments: &'a [DocCommentTag],
    tag: &str,
) -> Vec<&'a DocCommentTag> {
    comments.iter().filter(|c| c.tag == tag).collect()
}

/// Filter a collection of comments and return
/// only those that do not have provided tags
pub(crate) fn filter_comments_without_tags<'a>(
    comments: &'a [DocCommentTag],
    tags: Vec<&str>,
) -> Vec<&'a DocCommentTag> {
    comments.iter().filter(|c| !tags.contains(&c.tag.as_str())).collect()
}
