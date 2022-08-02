use solang_parser::pt::Comment;

#[derive(Debug, PartialEq, Clone)]
pub enum DocComment {
    Line { comment: DocCommentTag },
    Block { comments: Vec<DocCommentTag> },
}

impl DocComment {
    pub fn comments(&self) -> Vec<&DocCommentTag> {
        match self {
            DocComment::Line { comment } => vec![comment],
            DocComment::Block { comments } => comments.iter().collect(),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct DocCommentTag {
    pub tag: String,
    pub tag_offset: usize,
    pub value: String,
    pub value_offset: usize,
}

enum CommentType {
    Line,
    Block,
}

/// From the start to end offset, filter all the doc comments out of the comments and parse
/// them into tags with values.
pub fn parse_doccomments(comments: &[Comment], start: usize, end: usize) -> Vec<DocComment> {
    // first extract the tags
    let mut tags = Vec::new();

    let lines = filter_comments(comments, start, end);

    for (ty, comment_lines) in lines {
        let mut single_tags = Vec::new();

        for (start_offset, line) in comment_lines {
            let mut chars = line.char_indices().peekable();

            if let Some((_, '@')) = chars.peek() {
                // step over @
                let (tag_start, _) = chars.next().unwrap();
                let mut tag_end = tag_start;

                while let Some((offset, c)) = chars.peek() {
                    if c.is_whitespace() {
                        break
                    }

                    tag_end = *offset;

                    chars.next();
                }

                let leading =
                    line[tag_end + 1..].chars().take_while(|ch| ch.is_whitespace()).count();

                // tag value
                single_tags.push(DocCommentTag {
                    tag_offset: start_offset + tag_start + 1,
                    tag: line[tag_start + 1..tag_end + 1].to_owned(),
                    value_offset: start_offset + tag_end + leading + 1,
                    value: line[tag_end + 1..].trim().to_owned(),
                });
            } else if !single_tags.is_empty() || !tags.is_empty() {
                let line = line.trim();
                if !line.is_empty() {
                    let single_doc_comment = if let Some(single_tag) = single_tags.last_mut() {
                        Some(single_tag)
                    } else if let Some(tag) = tags.last_mut() {
                        match tag {
                            DocComment::Line { comment } => Some(comment),
                            DocComment::Block { comments } => comments.last_mut(),
                        }
                    } else {
                        None
                    };

                    if let Some(comment) = single_doc_comment {
                        comment.value.push('\n');
                        comment.value.push_str(line);
                    }
                }
            } else {
                let leading = line.chars().take_while(|ch| ch.is_whitespace()).count();

                single_tags.push(DocCommentTag {
                    tag_offset: start_offset + start_offset + leading,
                    tag: String::from("notice"),
                    value_offset: start_offset + start_offset + leading,
                    value: line.trim().to_owned(),
                });
            }
        }

        match ty {
            CommentType::Line if !single_tags.is_empty() => {
                tags.push(DocComment::Line { comment: single_tags[0].to_owned() })
            }
            CommentType::Block => tags.push(DocComment::Block { comments: single_tags }),
            _ => {}
        }
    }

    tags
}

/// Convert the comment to lines, stripping whitespace, comment characters and leading * in block
/// comments
fn filter_comments(
    comments: &[Comment],
    start: usize,
    end: usize,
) -> Vec<(CommentType, Vec<(usize, &str)>)> {
    let mut res = Vec::new();

    for comment in comments.iter() {
        let mut grouped_comments = Vec::new();

        match comment {
            Comment::DocLine(loc, comment) => {
                if loc.start() >= end || loc.end() < start {
                    continue
                }

                // remove the leading ///
                let leading = comment[3..].chars().take_while(|ch| ch.is_whitespace()).count();

                grouped_comments.push((loc.start() + leading + 3, comment[3..].trim()));

                res.push((CommentType::Line, grouped_comments));
            }
            Comment::DocBlock(loc, comment) => {
                if loc.start() >= end || loc.end() < start {
                    continue
                }

                let mut start = loc.start() + 3;

                let len = comment.len();

                // remove the leading /** and tailing */
                for s in comment[3..len - 2].lines() {
                    if let Some((i, _)) =
                        s.char_indices().find(|(_, ch)| !ch.is_whitespace() && *ch != '*')
                    {
                        grouped_comments.push((start + i, s[i..].trim_end()));
                    }

                    start += s.len() + 1;
                }

                res.push((CommentType::Block, grouped_comments));
            }
            _ => (),
        }
    }

    res
}
