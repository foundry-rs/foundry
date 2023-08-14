use crate::{AsDoc, AsDocResult};

/// The markdown format.
#[derive(Debug)]
pub enum Markdown<'a> {
    /// H1 heading item.
    H1(&'a str),
    /// H2 heading item.
    H2(&'a str),
    /// H3 heading item.
    H3(&'a str),
    /// Italic item.
    Italic(&'a str),
    /// Bold item.
    Bold(&'a str),
    /// Link item.
    Link(&'a str, &'a str),
    /// Code item.
    Code(&'a str),
    /// Code block item.
    CodeBlock(&'a str, &'a str),
}

impl<'a> AsDoc for Markdown<'a> {
    fn as_doc(&self) -> AsDocResult {
        let doc = match self {
            Self::H1(val) => format!("# {val}"),
            Self::H2(val) => format!("## {val}"),
            Self::H3(val) => format!("### {val}"),
            Self::Italic(val) => format!("*{val}*"),
            Self::Bold(val) => format!("**{val}**"),
            Self::Link(val, link) => format!("[{val}]({link})"),
            Self::Code(val) => format!("`{val}`"),
            Self::CodeBlock(lang, val) => format!("```{lang}\n{val}\n```"),
        };
        Ok(doc)
    }
}

impl<'a> std::fmt::Display for Markdown<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.as_doc()?))
    }
}
