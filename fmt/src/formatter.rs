//! A solidity formatter

use std::fmt::Write;

use indent_write::fmt::IndentWriter;
use itertools::Itertools;
use solang::parser::pt::{
    ContractDefinition, EnumDefinition, Identifier, SourceUnit, StringLiteral,
};

use crate::visit::{VResult, Visitable, Visitor};

/// Contains the config and rule set
#[derive(Debug, Clone)]
pub struct FormatterConfig {
    /// Maximum line length where formatter will try to wrap the line
    pub line_length: usize,
    /// Number of spaces per indentation level
    pub tab_width: usize,
    /// Print spaces between brackets
    pub bracket_spacing: bool,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        FormatterConfig { line_length: 80, tab_width: 4, bracket_spacing: false }
    }
}

/// A solidity formatter
pub struct Formatter<'a, W> {
    config: FormatterConfig,
    level: usize,
    pending_indent: bool,
    w: &'a mut W,
}

impl<'a, W: Write> Formatter<'a, W> {
    pub fn new(w: &'a mut W, config: FormatterConfig) -> Self {
        Self { config, level: 0, pending_indent: true, w }
    }

    fn indent(&mut self, level: usize) {
        self.level = self.level.saturating_add(level)
    }

    fn dedent(&mut self, level: usize) {
        self.level = self.level.saturating_sub(level)
    }

    /// Write opening bracket with respect to `config.bracket_spacing` setting:
    /// `"{ "` if `true`, `"{"` if `false`
    fn write_opening_bracket(&mut self) -> std::fmt::Result {
        self.write_str(if self.config.bracket_spacing { "{ " } else { "{" })
    }

    /// Write closing bracket with respect to `config.bracket_spacing` setting:
    /// `" }"` if `true`, `"}"` if `false`
    fn write_closing_bracket(&mut self) -> std::fmt::Result {
        self.write_str(if self.config.bracket_spacing { " }" } else { "}" })
    }

    /// Write empty brackets with respect to `config.bracket_spacing` setting:
    /// `"{ }"` if `true`, `"{}"` if `false`
    fn write_empty_brackets(&mut self) -> std::fmt::Result {
        self.write_str(if self.config.bracket_spacing { "{ }" } else { "{}" })
    }

    /// Is length of the line consisting of `items` separated by `separator` greater
    /// than `config.line_length`
    fn is_separated_multiline(&self, items: &Vec<String>, separator: impl AsRef<str>) -> bool {
        items.iter().join(separator.as_ref()).len() > self.config.line_length
    }

    /// Write `items` separated by `separator` with respect to `config.line_length` setting
    fn write_separated(
        &mut self,
        items: &Vec<String>,
        separator: impl AsRef<str>,
    ) -> std::fmt::Result {
        let mut line_length = 0;

        for (i, item) in items.iter().enumerate() {
            let separated_item =
                format!("{}{}", if i == 0 { "" } else { separator.as_ref() }, item);

            if line_length + separated_item.len() > self.config.line_length
                && separated_item.len() < self.config.line_length
            {
                write!(self, "{}\n{}", separator.as_ref().trim_end(), item)?;
                line_length = item.len();
            } else {
                write!(self, "{}", separated_item)?;
                line_length += separated_item.len();
            }
        }

        Ok(())
    }
}

impl<'a, W: Write> Write for Formatter<'a, W> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let indent = " ".repeat(self.config.tab_width * self.level);

        if self.pending_indent {
            IndentWriter::new(&indent, &mut self.w).write_str(s)?;
        } else {
            self.w.write_str(s)?;
        }

        self.pending_indent = s.ends_with('\n');

        Ok(())
    }
}

// traverse the solidity AST and write to the code formatter
impl<'a, W: Write> Visitor for Formatter<'a, W> {
    // TODO implement all visit callback and write the formatted output to the underlying

    fn visit_source_unit(&mut self, source_unit: &mut SourceUnit) -> VResult {
        let source_unit_parts = source_unit.0.len();
        for (i, unit) in source_unit.0.iter_mut().enumerate() {
            unit.visit(self)?;

            if i != source_unit_parts - 1 {
                writeln!(self)?;
            }
        }

        Ok(())
    }

    fn visit_contract(&mut self, contract: &mut ContractDefinition) -> VResult {
        write!(self, "contract {} ", &contract.name.name)?;
        if contract.parts.is_empty() {
            self.write_empty_brackets()?;
        } else {
            writeln!(self, "{{")?;

            self.indent(1);

            let contract_parts = contract.parts.len();
            for (i, part) in contract.parts.iter_mut().enumerate() {
                part.visit(self)?;

                if i != contract_parts - 1 {
                    writeln!(self)?;
                }
            }
            self.dedent(1);

            write!(self, "}}")?;
        }
        writeln!(self)?;

        Ok(())
    }

    fn visit_pragma(&mut self, ident: &mut Identifier, str: &mut StringLiteral) -> VResult {
        writeln!(self, "pragma {} {};", &ident.name, &str.string)?;

        Ok(())
    }

    fn visit_import_plain(&mut self, import: &mut StringLiteral) -> VResult {
        writeln!(self, "import \"{}\";", &import.string)?;

        Ok(())
    }

    fn visit_import_global(
        &mut self,
        global: &mut StringLiteral,
        alias: &mut Identifier,
    ) -> VResult {
        writeln!(self, "import \"{}\" as {};", global.string, alias.name)?;

        Ok(())
    }

    fn visit_import_renames(
        &mut self,
        imports: &mut Vec<(Identifier, Option<Identifier>)>,
        from: &mut StringLiteral,
    ) -> VResult {
        write!(self, "import ")?;

        let imports = imports
            .iter()
            .map(|(ident, alias)| {
                format!(
                    "{}{}",
                    ident.name,
                    alias.as_ref().map_or("".to_string(), |alias| format!(" as {}", alias.name))
                )
            })
            .collect::<Vec<_>>();

        let multiline = self.is_separated_multiline(&imports, ", ");

        if multiline {
            writeln!(self, "{{")?;
            self.indent(1);
        } else {
            self.write_opening_bracket()?;
        }

        self.write_separated(&imports, ", ")?;

        if multiline {
            self.dedent(1);
            write!(self, "\n}}")?;
        } else {
            self.write_closing_bracket()?;
        }

        writeln!(self, " from \"{}\";", from.string)?;

        Ok(())
    }

    fn visit_enum(&mut self, enumeration: &mut EnumDefinition) -> VResult {
        write!(self, "enum {} ", &enumeration.name.name)?;
        if enumeration.values.is_empty() {
            self.write_empty_brackets()?;
        } else {
            writeln!(self, "{{")?;

            self.indent(1);
            for (i, value) in enumeration.values.iter().enumerate() {
                write!(self, "{}", &value.name)?;

                if i != enumeration.values.len() - 1 {
                    write!(self, ",")?;
                }

                writeln!(self)?;
            }
            self.dedent(1);

            write!(self, "}}")?;
        }
        writeln!(self)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::visit::Visitable;

    use super::*;

    fn test_formatter(config: FormatterConfig, source: &str, expected: &str) {
        #[derive(PartialEq, Eq)]
        struct PrettyString(String);

        impl std::fmt::Debug for PrettyString {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        let mut source_unit = solang::parser::parse(source, 1).unwrap();
        let mut result = String::new();
        let mut f = Formatter::new(&mut result, config);

        source_unit.visit(&mut f).unwrap();

        let formatted = PrettyString(result);
        let expected = PrettyString(expected.trim_start().to_string());

        pretty_assertions::assert_eq!(formatted, expected, "(formatted == expected)");
    }

    #[test]
    fn contract() {
        test_formatter(
            Default::default(),
            r#"
contract Empty {
    
}
contract Enums {enum Empty {}}
"#,
            r#"
contract Empty {}

contract Enums {
    enum Empty {}
}
"#,
        );
        test_formatter(
            FormatterConfig { bracket_spacing: true, ..Default::default() },
            r#"
contract Empty {
}
"#,
            r#"
contract Empty { }
"#,
        );
    }

    #[test]
    fn pragma() {
        test_formatter(
            Default::default(),
            "pragma    solidity  ^0.5.0;",
            "pragma solidity ^0.5.0;\n",
        );
    }

    #[test]
    fn import() {
        test_formatter(
            Default::default(),
            "import    \"library.sol\"  ;",
            "import \"library.sol\";\n",
        );
        test_formatter(
            Default::default(),
            "import    \"library.sol\"   as   file  ;",
            "import \"library.sol\" as file;\n",
        );
        test_formatter(
            FormatterConfig { line_length: 15, ..Default::default() },
            "import{  symbol1  as   alias,   symbol2, sym3  } from   \"filename\"   ;",
            r#"
import {
    symbol1 as alias,
    symbol2, sym3
} from "filename";
"#,
        );
        test_formatter(
            Default::default(),
            "import{  symbol1  as   alias,   symbol2  } from   \"filename\"   ;",
            "import {symbol1 as alias, symbol2} from \"filename\";\n",
        );
        test_formatter(
            FormatterConfig { bracket_spacing: true, ..Default::default() },
            "import{  symbol1  as   alias,   symbol2  } from   \"filename\"   ;",
            "import { symbol1 as alias, symbol2 } from \"filename\";\n",
        );
    }

    #[test]
    fn enumeration() {
        test_formatter(
            Default::default(),
            r#"
enum Empty {
    
}
enum States { State1, State2, State3, State4, State5, State6, State7, State8, State9 }
"#,
            r#"
enum Empty {}

enum States {
    State1,
    State2,
    State3,
    State4,
    State5,
    State6,
    State7,
    State8,
    State9
}
"#,
        );
        test_formatter(
            FormatterConfig { bracket_spacing: true, ..Default::default() },
            r#"
enum Empty {
}
"#,
            r#"
enum Empty { }
"#,
        );
    }
}
