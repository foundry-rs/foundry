//! A solidity formatter

use crate::visit::Visitor;
use indenter::CodeFormatter;
use std::fmt::Write;

/// A solidity formatter
pub struct Formatter<'a, W> {
    config: FormatterConfig,
    /// indent aware code formatter that drives the output
    f: CodeFormatter<'a, W>,
}

impl<'a, W: Write> Formatter<'a, W> {
    pub fn new(w: &'a mut W, config: FormatterConfig) -> Self {
        Self { config, f: CodeFormatter::new(w, "   ") }
    }
}

// traverse the solidity AST and write to the code formatter
impl<'a, W: Write> Visitor for Formatter<'a, W> {
    // TODO implement all visit callback and write the formatted output to the underlying
    // CodeFormatter
}

/// Contains the config and rule set
#[derive(Debug, Clone)]
pub struct FormatterConfig {
    // TODO various rules/settings
}

impl Default for FormatterConfig {
    fn default() -> Self {
        FormatterConfig {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visit::Visitable;

    #[test]
    fn can_format() {
        let s = r#"
        contract Foo {}
        "#;
        let mut s = solang::parser::parse(s, 1).unwrap();
        let mut out = String::new();
        let mut f = Formatter::new(&mut out, Default::default());
        s.visit(&mut f).unwrap();
    }
}
