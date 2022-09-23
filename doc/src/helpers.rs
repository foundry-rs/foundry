use solang_parser::doccomment::DocCommentTag;

/// TODO:
pub fn filter_comments_by_tag<'a>(
    comments: &'a Vec<DocCommentTag>,
    tag: &str,
) -> Vec<&'a DocCommentTag> {
    comments.iter().filter(|c| c.tag == tag).collect()
}

pub fn filter_comments_without_tags<'a>(
    comments: &'a Vec<DocCommentTag>,
    tags: Vec<&str>,
) -> Vec<&'a DocCommentTag> {
    comments.iter().filter(|c| !tags.contains(&c.tag.as_str())).collect()
}
