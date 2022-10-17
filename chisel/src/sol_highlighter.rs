use rustyline::highlight::Highlighter;
use std::borrow::Cow;

/// A rustyline syntax highlighter for Solidity code
#[allow(dead_code)]
struct SolHighlighter {}

/// Highlighter implementation for `SolHighlighter`
impl Highlighter for SolHighlighter {
    #[allow(unused)]
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        todo!()
    }
}
