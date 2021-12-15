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
        FormatterConfig { line_length: 80, tab_width: 4, bracket_spacing: false }
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
        // TODO: do we need to put pragma and import directives at the top of the file?
        // source_unit.0.sort_by_key(|item| match item {
        //     SourceUnitPart::PragmaDirective(_, _, _) => 0,
        //     SourceUnitPart::ImportDirective(_, _) => 1,
        //     _ => usize::MAX,
        // });

        let source_unit_parts = source_unit.0.len();
        for (i, unit) in source_unit.0.iter_mut().enumerate() {
            let is_declaration = !matches!(
                unit,
                SourceUnitPart::ImportDirective(_, _) | SourceUnitPart::PragmaDirective(_, _, _)
            );

            unit.visit(self)?;
            writeln!(self)?;

            if i != source_unit_parts - 1 && is_declaration {
                writeln!(self)?;
            }
        }

        Ok(())
    }

    fn visit_contract(&mut self, contract: &mut ContractDefinition) -> VResult {
        write!(self, "{} {} ", contract.ty, contract.name.name)?;

        if !contract.base.is_empty() {
            write!(self, "is")?;

            let bases = contract
                .base
                .iter_mut()
                .map(|base| {
                    // TODO: write `base.args`

                    base.name.name.to_string()
                })
                .collect::<Vec<_>>();

            let multiline = self.is_separated_multiline(&bases, ", ");

            if multiline {
                writeln!(self)?;
                self.indent(1);
            } else {
                write!(self, " ")?;
            }

            self.write_separated(&bases, ", ", multiline)?;

            if multiline {
                self.dedent(1);
                writeln!(self)?;
            }
        }

        if contract.parts.is_empty() {
            self.write_empty_brackets()?;
        } else {
            writeln!(self, "{{")?;

            self.indent(1);
            // let contract_parts = contract.parts.len();
            for (_i, part) in contract.parts.iter_mut().enumerate() {
                part.visit(self)?;
                writeln!(self)?;

                // TODO: if source has zero blank lines between declarations, leave it as is. If one
                //  or more, separate declarations with one blank line.
                // if i != contract_parts - 1 {
                //     writeln!(self)?;
                // }
            }
            self.dedent(1);

            write!(self, "}}")?;
        }

        Ok(())
    }

    fn visit_pragma(&mut self, ident: &mut Identifier, str: &mut StringLiteral) -> VResult {
        // Ranges like `>=0.4.21<0.6.0` or `>=0.4.21 <0.6.0` are not parseable by `semver`
        // TODO: semver-solidity crate :D
        let semver = semver::VersionReq::parse(&str.string)?;

        write!(self, "pragma {} {};", &ident.name, semver)?;

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
    use std::{fs, path::PathBuf};

    use crate::visit::Visitable;

    use super::*;

    fn test_directory(dir: &str) {
        let snapshot_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/prettier-plugin-solidity/tests/format")
            .join(dir)
            .join("__snapshots__/jsfmt.spec.js.snap");

        let snapshot = fs::read_to_string(snapshot_path).unwrap();

        snapshot
            .strip_prefix("// Jest Snapshot v1, https://goo.gl/fbAQLP")
            .unwrap()
            .split("`;")
            .for_each(|test| {
                let is_header = |line: &str, name: &str| {
                    line.starts_with("=") && line.ends_with("=") && line.contains(name)
                };

                let mut lines = test.split('\n');

                let config = lines
                    .by_ref()
                    .skip_while(|line| !is_header(line, "options"))
                    .take_while(|line| !is_header(line, "input"))
                    .filter_map(|line| {
                        let parts = line.splitn(2, ":").collect::<Vec<_>>();

                        if parts.len() != 2 {
                            return None
                        }

                        let key = parts[0];
                        let value = parts[1].trim();

                        if key == "parsers" && value != r#"["solidity-parse"]"# {
                            return None
                        }

                        Some((key, value))
                    })
                    .fold(FormatterConfig::default(), |mut config, (key, value)| {
                        match key {
                            "bracketSpacing" => config.bracket_spacing = value == "true",
                            "compiler" => (),      // TODO: set compiler in config
                            "explicitTypes" => (), // TODO: set explicit_types in config
                            "parsers" => (),
                            "printWidth" => config.line_length = value.parse().unwrap(),
                            _ => panic!("Unknown snapshot options key: {}", key),
                        }

                        config
                    });

                let input = lines
                    .by_ref()
                    .take_while(|line| !is_header(line, "output"))
                    .collect::<Vec<_>>()
                    .join("\n");

                let output =
                    lines.take_while(|line| !is_header(line, "")).collect::<Vec<_>>().join("\n");

                test_formatter(config, &input, &output);
            });
    }

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
    fn contract_definitions() {
        test_directory("ContractDefinitions");
    }

    #[test]
    fn enum_definitions() {
        test_directory("EnumDefinitions");
    }

    #[test]
    fn import_directive() {
        test_directory("ImportDirective");
    }
}
