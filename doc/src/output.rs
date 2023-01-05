use crate::format::{DocFormat, DocResult};

/// TODO: rename
pub(crate) enum DocOutput<'a> {
    H1(&'a str),
    H2(&'a str),
    H3(&'a str),
    Bold(&'a str),
    Link(&'a str, &'a str),
    CodeBlock(&'a str, &'a str),
}

impl<'a> DocFormat for DocOutput<'a> {
    fn doc(&self) -> DocResult {
        let doc = match self {
            Self::H1(val) => format!("# {val}"),
            Self::H2(val) => format!("## {val}"),
            Self::H3(val) => format!("### {val}"),
            Self::Bold(val) => format!("**{val}**"),
            Self::Link(val, link) => format!("[{val}]({link})"),
            Self::CodeBlock(lang, val) => format!("```{lang}\n{val}\n```"),
        };
        Ok(doc)
    }
}

impl<'a> std::fmt::Display for DocOutput<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.doc()?))
    }
}
