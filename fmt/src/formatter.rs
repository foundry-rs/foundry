//! A Solidity formatter

use std::fmt::Write;

use indent_write::fmt::IndentWriter;
use solang::parser::pt::{
    ContractDefinition, EnumDefinition, Identifier, SourceUnit, SourceUnitPart, StringLiteral,
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
        FormatterConfig { line_length: 79, tab_width: 4, bracket_spacing: false }
    }
}

/// A Solidity formatter
pub struct Formatter<'a, W> {
    w: &'a mut W,
    config: FormatterConfig,
    level: usize,
    pending_indent: bool,
    current_line: usize,
}

impl<'a, W: Write> Formatter<'a, W> {
    pub fn new(w: &'a mut W, config: FormatterConfig) -> Self {
        Self { w, config, level: 0, pending_indent: true, current_line: 0 }
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

    /// Length of the line consisting of `items` separated by `separator` with respect to
    /// already written line
    fn len_indented_with_current(&self, s: &str) -> usize {
        if self.pending_indent { self.config.tab_width * self.level } else { 0 }
            .saturating_add(self.current_line)
            .saturating_add(s.len())
    }

    /// Is length of the line consisting of `items` separated by `separator` with respect to
    /// already written line greater than `config.line_length`
    fn is_separated_multiline(&self, items: &[String], separator: &str) -> bool {
        self.len_indented_with_current(&items.join(separator)) > self.config.line_length
    }

    /// Write `items` separated by `separator` with respect to `config.line_length` setting
    fn write_separated(
        &mut self,
        items: &[String],
        separator: &str,
        multiline: bool,
    ) -> std::fmt::Result {
        if multiline {
            for (i, item) in items.iter().enumerate() {
                write!(self, "{}", item)?;

                if i != items.len() - 1 {
                    writeln!(self, "{}", separator.trim_end())?;
                }
            }
        } else {
            write!(self, "{}", items.join(separator))?;
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
        self.current_line += s.len();

        self.pending_indent = s.ends_with('\n');
        if self.pending_indent {
            self.current_line = 0;
        }

        Ok(())
    }
}

// Traverse the Solidity Parse Tree and write to the code formatter
impl<'a, W: Write> Visitor for Formatter<'a, W> {
    fn visit_source_unit(&mut self, source_unit: &mut SourceUnit) -> VResult {
        source_unit.0.sort_by_key(|item| match item {
            SourceUnitPart::ImportDirective(_, _) => 0,
            _ => usize::MAX,
        });

        let source_unit_parts = source_unit.0.len();
        for (i, unit) in source_unit.0.iter_mut().enumerate() {
            let is_declaration = !matches!(
                unit,
                SourceUnitPart::ImportDirective(_, _) | SourceUnitPart::PragmaDirective(_, _, _)
            );
            if i != 0 && is_declaration {
                writeln!(self)?;
            }

            unit.visit(self)?;
            writeln!(self)?;

            if i != source_unit_parts - 1 && is_declaration {
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
                writeln!(self)?;

                if i != contract_parts - 1 {
                    writeln!(self)?;
                }
            }
            self.dedent(1);

            write!(self, "}}")?;
        }

        Ok(())
    }

    fn visit_pragma(&mut self, ident: &mut Identifier, str: &mut StringLiteral) -> VResult {
        write!(self, "pragma {} {};", &ident.name, &str.string)?;

        Ok(())
    }

    fn visit_import_plain(&mut self, import: &mut StringLiteral) -> VResult {
        write!(self, "import \"{}\";", &import.string)?;

        Ok(())
    }

    fn visit_import_global(
        &mut self,
        global: &mut StringLiteral,
        alias: &mut Identifier,
    ) -> VResult {
        write!(self, "import \"{}\" as {};", global.string, alias.name)?;

        Ok(())
    }

    fn visit_import_renames(
        &mut self,
        imports: &mut Vec<(Identifier, Option<Identifier>)>,
        from: &mut StringLiteral,
    ) -> VResult {
        write!(self, "import ")?;

        let mut imports = imports
            .iter()
            .map(|(ident, alias)| {
                format!(
                    "{}{}",
                    ident.name,
                    alias.as_ref().map_or("".to_string(), |alias| format!(" as {}", alias.name))
                )
            })
            .collect::<Vec<_>>();
        imports.sort();

        let multiline = self.is_separated_multiline(&imports, ", ");

        if multiline {
            writeln!(self, "{{")?;
            self.indent(1);
        } else {
            self.write_opening_bracket()?;
        }

        self.write_separated(&imports, ", ", multiline)?;

        if multiline {
            self.dedent(1);
            write!(self, "\n}}")?;
        } else {
            self.write_closing_bracket()?;
        }

        write!(self, " from \"{}\";", from.string)?;

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
    sym3,
    symbol1 as alias,
    symbol2
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
        test_formatter(
            Default::default(),
            r#"
import "library.sol";

contract Empty {}

import "token.sol";
"#,
            r#"
import "library.sol";
import "token.sol";

contract Empty {}
"#,
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
