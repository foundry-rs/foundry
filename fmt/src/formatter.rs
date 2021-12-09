//! A solidity formatter

use std::fmt::Write;

use indenter::CodeFormatter;
use solang::parser::pt::{ContractDefinition, EnumDefinition, SourceUnit};

use crate::visit::{VResult, Visitable, Visitor};

/// A solidity formatter
pub struct Formatter<'a, W> {
    config: FormatterConfig,
    /// indent aware code formatter that drives the output
    f: CodeFormatter<'a, W>,
}

impl<'a, W: Write> Formatter<'a, W> {
    pub fn new(w: &'a mut W, config: FormatterConfig) -> Self {
        let f = CodeFormatter::new(w, " ".repeat(config.tab_width));
        Self { config, f }
    }
}

// traverse the solidity AST and write to the code formatter
impl<'a, W: Write> Visitor for Formatter<'a, W> {
    // TODO implement all visit callback and write the formatted output to the underlying

    fn visit_source_unit(&mut self, source_unit: &SourceUnit) -> VResult {
        for (i, unit) in source_unit.0.iter().enumerate() {
            unit.visit(self)?;

            if i != source_unit.0.len() - 1 {
                writeln!(self.f)?;
            }
        }

        Ok(())
    }

    fn visit_contract(&mut self, contract: &ContractDefinition) -> VResult {
        if contract.parts.is_empty() {
            writeln!(self.f, "contract {} {{}}", &contract.name.name)?;
        } else {
            writeln!(self.f, "contract {} {{", &contract.name.name)?;

            self.f.indent(1);
            for (i, part) in contract.parts.iter().enumerate() {
                part.visit(self)?;

                if i != contract.parts.len() - 1 {
                    writeln!(self.f)?;
                }
            }
            self.f.dedent(1);

            writeln!(self.f, "}}")?;
        }

        Ok(())
    }

    fn visit_enum(&mut self, enumeration: &EnumDefinition) -> VResult {
        if enumeration.values.is_empty() {
            writeln!(self.f, "\nenum {} {{}}", &enumeration.name.name)?;
        } else {
            write!(self.f, "\nenum {} {{", &enumeration.name.name)?;

            self.f.indent(1);
            for (i, value) in enumeration.values.iter().enumerate() {
                writeln!(
                    self.f,
                    "\n{}{}",
                    &value.name,
                    if i != enumeration.values.len() - 1 { "," } else { "" }
                )?;
            }
            self.f.dedent(1);

            writeln!(self.f, "\n}}")?;
        }

        Ok(())
    }
}

/// Contains the config and rule set
#[derive(Debug, Clone)]
pub struct FormatterConfig {
    pub tab_width: usize,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        FormatterConfig { tab_width: 4 }
    }
}

#[cfg(test)]
mod tests {
    use crate::visit::Visitable;

    use super::*;

    fn test_formatter(source: &str, expected: &str) {
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

        let s = solang::parser::parse(source, 1).unwrap();
        let mut out = String::new();
        let mut f = Formatter::new(&mut out, Default::default());

        s.visit(&mut f).unwrap();

        let expected = expected.trim_start();

        assert_eq!(PrettyString(&out), PrettyString(expected), "(formatted == expected)");
    }

    #[test]
    fn contract() {
        let source = r#"
contract Empty {
    
}
contract Enums {enum Empty {}}
"#;

        let expected = r#"
contract Empty {}

contract Enums {
    enum Empty {}
}
"#;

        test_formatter(source, expected);
    }

    #[test]
    fn enumeration() {
        let source = r#"
contract EnumDefinitions {
    enum Empty {
        
    }
    enum States { State1, State2, State3, State4, State5, State6, State7, State8, State9 }
}
"#;

        let expected = r#"
contract EnumDefinitions {
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
}
"#;

        test_formatter(source, expected);
    }
}
