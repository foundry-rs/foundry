use crate::format::DocFormat;

pub enum DocOutput<'a> {
    H1(&'a str),
    H2(&'a str),
    H3(&'a str),
    Bold(&'a str),
    Link(&'a str, &'a str),
    CodeBlock(&'a str, &'a str),
}

impl<'a> std::fmt::Display for DocOutput<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.doc()))
    }
}
