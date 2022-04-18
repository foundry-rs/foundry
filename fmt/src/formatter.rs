//! A Solidity formatter

use std::fmt::Write;

use indent_write::fmt::IndentWriter;
use solang_parser::pt::{
    CodeLocation, ContractDefinition, DocComment, EnumDefinition, Expression, FunctionDefinition,
    Identifier, Loc, SourceUnit, SourceUnitPart, Statement, StringLiteral, StructDefinition, Type,
    TypeDefinition, VariableDeclaration,
};

use crate::{
    loc::LineOfCode,
    visit::{VResult, Visitable, Visitor},
};

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

// TODO: use it inside Formatter since they're sharing same fields
#[derive(Default)]
struct FormatBuffer {
    level: usize,
    current_line: usize,
    pending_indent: bool,
    w: String,
}

/// A Solidity formatter
pub struct Formatter<'a, W> {
    w: &'a mut W,
    source: &'a str,
    config: FormatterConfig,
    level: usize,
    pending_indent: bool,
    current_line: usize,
    bufs: Vec<FormatBuffer>,
}

impl<'a, W: Write> Formatter<'a, W> {
    pub fn new(w: &'a mut W, source: &'a str, config: FormatterConfig) -> Self {
        Self {
            w,
            source,
            config,
            level: 0,
            pending_indent: true,
            bufs: Vec::new(),
            current_line: 0,
        }
    }

    fn level(&mut self) -> &mut usize {
        if let Some(buf) = self.bufs.last_mut() {
            &mut buf.level
        } else {
            &mut self.level
        }
    }

    fn indent(&mut self, delta: usize) {
        let level = self.level();

        *level += delta;
    }

    fn dedent(&mut self, delta: usize) {
        let level = self.level();

        *level -= delta;
    }

    /// Write opening bracket with respect to `config.bracket_spacing` setting:
    /// `"{ "` if `true`, `"{"` if `false`
    fn write_opening_bracket(&mut self) -> std::fmt::Result {
        write!(self, "{}", if self.config.bracket_spacing { "{ " } else { "{" })
    }

    /// Write closing bracket with respect to `config.bracket_spacing` setting:
    /// `" }"` if `true`, `"}"` if `false`
    fn write_closing_bracket(&mut self) -> std::fmt::Result {
        write!(self, "{}", if self.config.bracket_spacing { " }" } else { "}" })
    }

    /// Write empty brackets with respect to `config.bracket_spacing` setting:
    /// `"{ }"` if `true`, `"{}"` if `false`
    fn write_empty_brackets(&mut self) -> std::fmt::Result {
        write!(self, "{}", if self.config.bracket_spacing { "{ }" } else { "{}" })
    }

    /// Length of the line `s` with respect to already written line
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

    fn visit_to_string(
        &mut self,
        visitable: &mut impl Visitable,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.bufs.push(FormatBuffer::default());
        Visitable::visit(visitable, self)?;
        let buf = self.bufs.pop().unwrap();

        Ok(buf.w)
    }

    /// Returns number of blank lines between two LOCs
    fn blank_lines(&self, a: Loc, b: Loc) -> usize {
        return self.source[a.end()..b.start()].matches('\n').count()
    }
}

impl<'a, W: Write> Write for Formatter<'a, W> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let (level, current_line, pending_indent, w): (_, _, _, &mut dyn Write) =
            if let Some(buf) = self.bufs.last_mut() {
                (buf.level, &mut buf.current_line, &mut buf.pending_indent, &mut buf.w)
            } else {
                (self.level, &mut self.current_line, &mut self.pending_indent, self.w)
            };

        if *pending_indent {
            let indent = " ".repeat(self.config.tab_width * level);
            IndentWriter::new(&indent, w).write_str(s)?;
        } else {
            w.write_str(s)?;
        }
        *current_line += s.len();

        *pending_indent = s.ends_with('\n');
        if *pending_indent {
            *current_line = 0;
        }

        Ok(())
    }
}

// Traverse the Solidity Parse Tree and write to the code formatter
impl<'a, W: Write> Visitor for Formatter<'a, W> {
    fn visit_source(&mut self, loc: Loc) -> VResult {
        let source = String::from_utf8(self.source.as_bytes()[loc.start()..loc.end()].to_vec())?;
        let mut lines = source.splitn(2, '\n');

        write!(self, "{}", lines.next().unwrap())?;
        if let Some(remainder) = lines.next() {
            // Call with `self.write_str` and not `write!`, so we can have `\n` at the beginning
            // without triggering an indentation
            self.write_str(&format!("\n{}", remainder))?;
        }

        Ok(())
    }

    fn visit_source_unit(&mut self, source_unit: &mut SourceUnit) -> VResult {
        // TODO: do we need to put pragma and import directives at the top of the file?
        // source_unit.0.sort_by_key(|item| match item {
        //     SourceUnitPart::PragmaDirective(_, _, _) => 0,
        //     SourceUnitPart::ImportDirective(_, _) => 1,
        //     _ => usize::MAX,
        // });

        let mut source_unit_parts_iter = source_unit.0.iter_mut().peekable();
        while let Some(unit) = source_unit_parts_iter.next() {
            let is_pragma =
                |u: &SourceUnitPart| matches!(u, SourceUnitPart::PragmaDirective(_, _, _, _));
            let is_import = |u: &SourceUnitPart| matches!(u, SourceUnitPart::ImportDirective(_, _));
            let is_declaration = |u: &SourceUnitPart| !(is_pragma(u) || is_import(u));

            unit.visit(self)?;
            writeln!(self)?;

            if let Some(next_unit) = source_unit_parts_iter.peek() {
                if (is_declaration(unit) || is_declaration(next_unit)) ||
                    (is_pragma(unit) || is_pragma(next_unit)) ||
                    (is_import(unit) &&
                        is_import(next_unit) &&
                        // If source has zero blank lines between imports, leave it as is. If one
                        //  or more, separate imports with one blank line.
                        self.blank_lines(unit.loc(), next_unit.loc()) > 1)
                {
                    writeln!(self)?;
                }
            }
        }

        Ok(())
    }

    fn visit_doc_comment(&mut self, doc_comment: &mut DocComment) -> VResult {
        match doc_comment {
            DocComment::Line { comment } => {
                write!(self, "/// @{}", comment.tag)?;
                if !comment.value.is_empty() {
                    let mut lines = comment.value.split('\n');
                    write!(self, " {}", lines.next().unwrap())?;

                    for line in lines {
                        writeln!(self)?; // Write newline separately to trigger an indentation
                        write!(self, "/// {}", line)?;
                    }
                }
            }
            DocComment::Block { comments } => {
                writeln!(self, "/**")?;
                for comment in comments {
                    write!(self, "@{} ", comment.tag)?;
                    for line in comment.value.split('\n') {
                        writeln!(self, "{}", line)?;
                    }
                }
                write!(self, "*/")?;
            }
        };

        Ok(())
    }

    fn visit_doc_comments(&mut self, doc_comments: &mut [DocComment]) -> VResult {
        for (i, doc_comment) in doc_comments.iter_mut().enumerate() {
            if i > 0 {
                writeln!(self)?;
            }
            doc_comment.visit(self)?;
        }

        Ok(())
    }

    fn visit_contract(&mut self, contract: &mut ContractDefinition) -> VResult {
        if !contract.doc.is_empty() {
            contract.doc.visit(self)?;
            writeln!(self)?;
        }

        write!(self, "{} {} ", contract.ty, contract.name.name)?;

        if !contract.base.is_empty() {
            write!(self, "is")?;

            let bases = contract
                .base
                .iter_mut()
                .map(|base| {
                    // TODO
                    self.visit_to_string(&mut base.loc)
                })
                .collect::<Result<Vec<_>, _>>()?;

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
            } else {
                write!(self, " ")?;
            }
        }

        if contract.parts.is_empty() {
            self.write_empty_brackets()?;
        } else {
            writeln!(self, "{{")?;

            self.indent(1);
            let mut contract_parts_iter = contract.parts.iter_mut().peekable();
            while let Some(part) = contract_parts_iter.next() {
                part.visit(self)?;
                writeln!(self)?;

                // If source has zero blank lines between declarations, leave it as is. If one
                //  or more, separate declarations with one blank line.
                if let Some(next_part) = contract_parts_iter.peek() {
                    if self.blank_lines(part.loc(), next_part.loc()) > 1 {
                        writeln!(self)?;
                    }
                }
            }
            self.dedent(1);

            write!(self, "}}")?;
        }

        Ok(())
    }

    fn visit_pragma(&mut self, ident: &mut Identifier, str: &mut StringLiteral) -> VResult {
        write!(self, "pragma {} ", &ident.name)?;

        #[allow(clippy::if_same_then_else)]
        if ident.name == "solidity" {
            // There are some issues with parsing Solidity's versions with crates like `semver`:
            // 1. Ranges like `>=0.4.21<0.6.0` or `>=0.4.21 <0.6.0` are not parseable at all.
            // 2. Versions like `0.8.10` got transformed into `^0.8.10` which is not the same.
            // TODO: semver-solidity crate :D
            write!(self, "{};", str.string)?;
        } else {
            write!(self, "{};", str.string)?;
        }

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
        imports: &mut [(Identifier, Option<Identifier>)],
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
        if !enumeration.doc.is_empty() {
            enumeration.doc.visit(self)?;
            writeln!(self)?;
        }

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

    fn visit_expr(&mut self, loc: Loc, expr: &mut Expression) -> VResult {
        match expr {
            Expression::Type(_, typ) => match typ {
                Type::Address => write!(self, "address")?,
                Type::AddressPayable => write!(self, "address payable")?,
                Type::Payable => write!(self, "payable")?,
                Type::Bool => write!(self, "bool")?,
                Type::String => write!(self, "string")?,
                Type::Int(n) => write!(self, "int{}", n)?,
                Type::Uint(n) => write!(self, "uint{}", n)?,
                Type::Bytes(n) => write!(self, "bytes{}", n)?,
                Type::Rational => write!(self, "rational")?,
                Type::DynamicBytes => write!(self, "bytes")?,
                Type::Mapping(_, from, to) => {
                    write!(self, "mapping(")?;
                    from.visit(self)?;
                    write!(self, " => ")?;
                    to.visit(self)?;
                    write!(self, ")")?;
                }
                Type::Function { .. } => self.visit_source(loc)?,
            },
            _ => self.visit_source(loc)?,
        };

        Ok(())
    }

    fn visit_var_declaration(&mut self, var: &mut VariableDeclaration) -> VResult {
        var.ty.visit(self)?;

        if let Some(storage) = &var.storage {
            write!(self, " {}", storage)?;
        }

        write!(self, " {}", var.name.name)?;

        Ok(())
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> VResult {
        if !func.doc.is_empty() {
            func.doc.visit(self)?;
            writeln!(self)?;
        }

        // Workaround for cases when function in the original source code had parameters and
        // modifiers spanned across multiple lines, thus having its own indentation that we
        // need to reset.
        let signature = self.visit_to_string(&mut func.loc)?;
        let level = self.level;
        self.dedent(level);
        write!(self, "{}{}", " ".repeat(self.config.tab_width * level), signature)?;
        self.indent(level);

        match &mut func.body {
            Some(body) => {
                // Same, until we reconstruct function parameters and modifiers on our own,
                // we need to respect style of the original source code
                if self.blank_lines(func.loc, body.loc()) > 0 {
                    writeln!(self)?;
                } else if !signature.ends_with(char::is_whitespace) {
                    write!(self, " ")?;
                }
                body.visit(self)?
            }
            None => write!(self, ";")?,
        };

        Ok(())
    }

    fn visit_struct(&mut self, structure: &mut StructDefinition) -> VResult {
        if !structure.doc.is_empty() {
            structure.doc.visit(self)?;
            writeln!(self)?;
        }

        write!(self, "struct {} ", &structure.name.name)?;

        if structure.fields.is_empty() {
            self.write_empty_brackets()?;
        } else {
            writeln!(self, "{{")?;

            self.indent(1);
            for field in structure.fields.iter_mut() {
                field.visit(self)?;
                writeln!(self, ";")?;
            }
            self.dedent(1);

            write!(self, "}}")?;
        }

        Ok(())
    }

    fn visit_type_definition(&mut self, def: &mut TypeDefinition) -> VResult {
        if !def.doc.is_empty() {
            def.doc.visit(self)?;
            writeln!(self)?;
        }

        write!(self, "type {} is ", def.name.name)?;
        def.ty.visit(self)?;
        write!(self, ";")?;

        Ok(())
    }

    fn visit_stray_semicolon(&mut self) -> VResult {
        write!(self, ";")?;

        Ok(())
    }

    fn visit_newline(&mut self) -> VResult {
        writeln!(self)?;

        Ok(())
    }

    fn visit_block(
        &mut self,
        loc: Loc,
        unchecked: bool,
        statements: &mut Vec<Statement>,
    ) -> VResult {
        if unchecked {
            write!(self, "unchecked ")?;
        }

        if statements.is_empty() {
            self.write_empty_brackets()?;
            return Ok(())
        }

        let multiline = self.source[loc.start()..loc.end()].matches('\n').count() > 0;

        if multiline {
            writeln!(self, "{{")?;
            self.indent(1);
        } else {
            self.write_opening_bracket()?;
        }

        // We need to skip statements which evaluate to empty string on visiting.
        // It may happen on empty unchecked blocks,
        let mut statements_iter = statements.iter_mut().peekable();
        while let Some(stmt) = statements_iter.next() {
            stmt.visit(self)?;
            if multiline {
                writeln!(self)?;
            }

            // If source has zero blank lines between statements, leave it as is. If one
            //  or more, separate statements with one blank line.
            if let Some(next_stmt) = statements_iter.peek() {
                if self.blank_lines(stmt.loc(), next_stmt.loc()) > 1 {
                    writeln!(self)?;
                }
            }
        }

        if multiline {
            self.dedent(1);
            write!(self, "}}")?;
        } else {
            self.write_closing_bracket()?;
        }

        Ok(())
    }

    fn visit_break(&mut self) -> VResult {
        write!(self, "break;")?;

        Ok(())
    }

    fn visit_continue(&mut self) -> VResult {
        write!(self, "continue;")?;

        Ok(())
    }

    fn visit_emit(&mut self, _loc: Loc, event: &mut Expression) -> VResult {
        write!(self, "emit ")?;
        self.visit_source(event.loc())?;
        write!(self, ";")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use itertools::Itertools;
    use std::{fs, path::PathBuf};

    use crate::visit::Visitable;

    use super::*;

    fn test_directory(base_name: &str) {
        let mut original = None;

        let tests = fs::read_dir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join(base_name),
        )
        .unwrap()
        .filter_map(|path| {
            let path = path.unwrap().path();
            let source = fs::read_to_string(&path).unwrap();

            if let Some(filename) = path.file_name().and_then(|name| name.to_str()) {
                if filename == "original.sol" {
                    original = Some(source);
                } else if filename
                    .strip_suffix("fmt.sol")
                    .map(|filename| filename.strip_suffix('.'))
                    .is_some()
                {
                    let mut config = FormatterConfig::default();

                    let mut lines = source.split('\n').peekable();
                    while let Some(line) = lines.peek() {
                        let entry = line
                            .strip_prefix("//")
                            .and_then(|line| line.trim().strip_prefix("config:"))
                            .map(str::trim);
                        if entry.is_none() {
                            break
                        }

                        if let Some((key, value)) = entry.unwrap().split_once("=") {
                            match key {
                                "line-length" => config.line_length = value.parse().unwrap(),
                                "tab-width" => config.tab_width = value.parse().unwrap(),
                                "bracket-spacing" => {
                                    config.bracket_spacing = value.parse().unwrap()
                                }
                                _ => panic!("Unknown config key: {key}"),
                            }
                        }

                        lines.next();
                    }

                    return Some((filename.to_string(), config, lines.join("\n")))
                }
            }

            None
        })
        .collect::<Vec<_>>();

        for (filename, config, formatted) in tests {
            test_formatter(
                &filename,
                config,
                original.as_ref().expect("original.sol not found"),
                &formatted,
            );
        }
    }

    fn test_formatter(filename: &str, config: FormatterConfig, source: &str, expected: &str) {
        #[derive(PartialEq, Eq)]
        struct PrettyString(String);

        impl std::fmt::Debug for PrettyString {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        let (mut source_unit, _comments) = solang_parser::parse(source, 1).unwrap();
        let mut result = String::new();
        let mut f = Formatter::new(&mut result, source, config);

        source_unit.visit(&mut f).unwrap();

        let formatted = PrettyString(result);
        let expected = PrettyString(expected.trim_start().to_string());

        pretty_assertions::assert_eq!(
            formatted,
            expected,
            "(formatted == expected) in {}",
            filename
        );
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

    #[test]
    fn statement_block() {
        test_directory("StatementBlock");
    }

    #[test]
    fn struct_definition() {
        test_directory("StructDefinition");
    }

    #[test]
    fn type_definition() {
        test_directory("TypeDefinition");
    }
}
