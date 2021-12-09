//! A solidity formatter

use std::fmt::Write;

use indent_write::fmt::IndentWriter;
use itertools::Itertools;
use solang::parser::pt::{
    ContractDefinition, EnumDefinition, Identifier, SourceUnit, StringLiteral,
};

use crate::visit::{VResult, Visitable, Visitor};

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

    /// Respects the `config.bracket_spacing` setting:
    /// `"{ "` if `true`, `"{"` if `false`
    fn opening_bracket(&self) -> String {
        if self.config.bracket_spacing { "{ " } else { "{" }.to_string()
    }

    /// Respects the `config.bracket_spacing` setting:
    /// `" }"` if `true`, `"}"` if `false`
    fn closing_bracket(&self) -> String {
        if self.config.bracket_spacing { " }" } else { "}" }.to_string()
    }

    /// Respects the `config.bracket_spacing` setting:
    /// `"{ }"` if `true`, `"{}"` if `false`
    fn empty_brackets(&self) -> String {
        if self.config.bracket_spacing { "{ }" } else { "{}" }.to_string()
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
            writeln!(self, "{}", self.empty_brackets())?;
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

            writeln!(self, "}}")?;
        }

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
        write!(self, "import {}", self.opening_bracket())?;
        write!(
            self,
            "{}",
            imports
                .iter()
                .map(|(ident, alias)| format!(
                    "{}{}",
                    ident.name,
                    alias.as_ref().map_or("".to_string(), |alias| format!(" as {}", alias.name))
                ))
                .join(", ")
        )?;
        writeln!(self, "{} from \"{}\";", self.closing_bracket(), from.string)?;

        Ok(())
    }

    fn visit_enum(&mut self, enumeration: &mut EnumDefinition) -> VResult {
        write!(self, "enum {} ", &enumeration.name.name)?;
        if enumeration.values.is_empty() {
            writeln!(self, "{}", self.empty_brackets())?;
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

            writeln!(self, "}}")?;
        }

        Ok(())
    }
}

/// Contains the config and rule set
#[derive(Debug, Clone)]
pub struct FormatterConfig {
    /// Number of spaces per indentation level
    pub tab_width: usize,
    /// Print spaces between brackets
    pub bracket_spacing: bool,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        FormatterConfig { tab_width: 4, bracket_spacing: false }
    }
}

#[cfg(test)]
mod tests {
    use crate::visit::Visitable;

    use super::*;

    fn test_formatter(config: FormatterConfig, source: &str, expected: &str) {
        use pretty_assertions::assert_eq;
        use std::fmt;

        #[derive(PartialEq, Eq)]
        #[doc(hidden)]
        pub struct PrettyString<'a>(pub &'a str);

        /// Make diff to display string as multi-line string
        impl<'a> fmt::Debug for PrettyString<'a> {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(self.0)
            }
        }

        let mut s = solang::parser::parse(source, 1).unwrap();
        let mut out = String::new();
        let mut f = Formatter::new(&mut out, config);

        s.visit(&mut f).unwrap();

        let expected = expected.trim_start();

        assert_eq!(PrettyString(&out), PrettyString(expected), "(formatted == expected)");
    }

    #[test]
    fn contract() {
        test_formatter(
            FormatterConfig { bracket_spacing: false, ..Default::default() },
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
            FormatterConfig { bracket_spacing: false, ..Default::default() },
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
            FormatterConfig { bracket_spacing: false, ..Default::default() },
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
