//! A Solidity formatter

use std::fmt::Write;

use indent_write::fmt::IndentWriter;
use itertools::Itertools;
use solang_parser::pt::*;

use crate::{
    loc::LineOfCode,
    visit::{ParameterList, VResult, Visitable, Visitor},
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

// TODO: store context entities as references without copying
#[derive(Default)]
struct Context {
    contract: Option<ContractDefinition>,
    function: Option<FunctionDefinition>,
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
    context: Context,
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
            context: Context::default(),
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
    fn len_indented_with_current(&self, s: impl AsRef<str>) -> usize {
        if self.pending_indent { self.config.tab_width * self.level } else { 0 }
            .saturating_add(self.current_line)
            .saturating_add(s.as_ref().len())
    }

    /// Is length of the `text` with respect to already written line <= `config.line_length`
    fn will_it_fit(&self, text: impl AsRef<str>) -> bool {
        self.len_indented_with_current(text) <= self.config.line_length
    }

    /// Is length of the line consisting of `items` separated by `separator` with respect to
    /// already written line greater than `config.line_length`
    fn is_separated_multiline(
        &self,
        items: impl IntoIterator<Item = impl AsRef<str> + std::fmt::Display>,
        separator: &str,
    ) -> bool {
        !self.will_it_fit(items.into_iter().join(separator))
    }

    /// Write `items` with no separator with respect to `config.line_length` setting
    fn write_items(
        &mut self,
        items: impl IntoIterator<Item = impl AsRef<str> + std::fmt::Display>,
        multiline: bool,
    ) -> std::fmt::Result {
        self.write_items_separated(items, "", multiline)
    }

    /// Write `items` separated by `separator` with respect to `config.line_length` setting
    fn write_items_separated(
        &mut self,
        items: impl IntoIterator<Item = impl AsRef<str> + std::fmt::Display>,
        separator: &str,
        multiline: bool,
    ) -> std::fmt::Result {
        if multiline {
            let mut items = items.into_iter().peekable();
            while let Some(item) = items.next() {
                write!(self, "{}", item.as_ref())?;

                if items.peek().is_some() {
                    writeln!(self, "{}", separator.trim_end())?;
                }
            }
        } else {
            write!(self, "{}", items.into_iter().join(separator))?;
        }

        Ok(())
    }

    fn visit_to_string(
        &mut self,
        visitable: &mut impl Visitable,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.bufs.push(FormatBuffer::default());
        visitable.visit(self)?;
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
            self.write_str(&format!("\n{remainder}"))?;
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
        let mut iter = doc_comments.iter_mut();
        if let Some(doc_comment) = iter.next() {
            doc_comment.visit(self)?
        }
        for doc_comment in iter {
            writeln!(self)?;
            doc_comment.visit(self)?;
        }

        Ok(())
    }

    fn visit_contract(&mut self, contract: &mut ContractDefinition) -> VResult {
        self.context.contract = Some(contract.clone());

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
                .map(|base| self.visit_to_string(base))
                .collect::<Result<Vec<_>, _>>()?;

            let multiline = self.is_separated_multiline(&bases, ", ");

            if multiline {
                writeln!(self)?;
                self.indent(1);
            } else {
                write!(self, " ")?;
            }

            self.write_items_separated(&bases, ", ", multiline)?;

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

                // If source has zero blank lines between parts and the current part is not a
                // function, leave it as is. If it has one or more blank lines or
                // the current part is a function, separate parts with one blank
                // line.
                if let Some(next_part) = contract_parts_iter.peek() {
                    let blank_lines = self.blank_lines(part.loc(), next_part.loc());
                    let is_function =
                        if let ContractPart::FunctionDefinition(function_definition) = part {
                            matches!(
                                **function_definition,
                                FunctionDefinition {
                                    ty: FunctionTy::Function |
                                        FunctionTy::Receive |
                                        FunctionTy::Fallback,
                                    ..
                                }
                            )
                        } else {
                            false
                        };
                    if is_function && blank_lines > 0 || blank_lines > 1 {
                        writeln!(self)?;
                    }
                }
            }
            self.dedent(1);

            write!(self, "}}")?;
        }

        self.context.contract = None;

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

        self.write_items_separated(&imports, ", ", multiline)?;

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

    fn visit_emit(&mut self, _loc: Loc, event: &mut Expression) -> VResult {
        write!(self, "emit ")?;
        event.loc().visit(self)?;
        write!(self, ";")?;

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

    fn visit_break(&mut self) -> VResult {
        write!(self, "break;")?;

        Ok(())
    }

    fn visit_continue(&mut self) -> VResult {
        write!(self, "continue;")?;

        Ok(())
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> VResult {
        self.context.function = Some(func.clone());

        if !func.doc.is_empty() {
            func.doc.visit(self)?;
            writeln!(self)?;
        }

        write!(self, "{}", func.ty)?;

        if let Some(Identifier { name, .. }) = &func.name {
            write!(self, " {name}")?;
        }

        let params = self.visit_to_string(&mut func.params)?;
        let params_multiline = params.contains('\n');

        self.write_items(params.lines(), params_multiline)?;

        let attributes = self.visit_to_string(&mut func.attributes)?;
        let attributes = attributes.lines().collect::<Vec<_>>();

        let returns = self.visit_to_string(&mut func.returns)?;
        let returns_multiline = returns.contains('\n');
        let returns_indent = params_multiline || !attributes.is_empty() || returns_multiline;

        // Compose one line string consisting of attributes and return parameters.
        let attributes_returns = format!(
            "{} {}",
            attributes.join(" "),
            if func.returns.is_empty() { "".to_string() } else { format!("returns {returns}") }
        );
        let attributes_returns = attributes_returns.trim();

        let (body, body_first_line) = match &mut func.body {
            Some(body) => {
                let body_string = self.visit_to_string(body)?;
                let first_line = body_string.lines().next().unwrap_or_default();

                (Some(body), format!(" {first_line}"))
            }
            None => (None, ";".to_string()),
        };

        let attributes_returns_fits_one_line = self
            .will_it_fit(&format!(" {attributes_returns}{body_first_line}")) &&
            !returns_multiline;

        // Check that we can fit both attributes and return arguments in one line.
        if !attributes_returns.is_empty() && attributes_returns_fits_one_line {
            write!(self, " {attributes_returns}")?;
        } else {
            // If attributes and returns can't fit in one line, we write all attributes in multiple
            // lines.
            if !func.attributes.is_empty() {
                writeln!(self)?;
                self.indent(1);
                func.attributes.visit(self)?;
                self.dedent(1);
            }

            if !func.returns.is_empty() {
                if returns_indent {
                    self.indent(1);
                }
                writeln!(self)?;
                write!(self, "returns ")?;

                self.write_items(returns.lines(), returns_multiline)?;

                if returns_indent {
                    self.dedent(1);
                }
            }
        }

        match body {
            Some(body) => {
                if self.will_it_fit(format!(" {}", body_first_line)) &&
                    attributes_returns_fits_one_line
                {
                    write!(self, " ")?;
                } else {
                    writeln!(self)?;
                }
                // TODO: when we implement visitors for statements, write `body_string` here instead
                //  of visiting it twice.
                body.visit(self)?;
            }
            None => write!(self, ";")?,
        }

        self.context.function = None;

        Ok(())
    }

    /// Write each function attribute on a new line because we don't have enough information in
    /// visit function regarding one line/multiline cases. We can transform it into one line later
    /// by `.split("\n").join(" ")`.
    fn visit_function_attribute_list(&mut self, list: &mut Vec<FunctionAttribute>) -> VResult {
        let mut attributes = list
            .iter_mut()
            .sorted_by_key(|attribute| match attribute {
                FunctionAttribute::Visibility(_) => 0,
                FunctionAttribute::Mutability(_) => 1,
                FunctionAttribute::Virtual(_) => 2,
                FunctionAttribute::Override(_, _) => 3,
                FunctionAttribute::BaseOrModifier(_, _) => 4,
            })
            .peekable();

        while let Some(attribute) = attributes.next() {
            attribute.visit(self)?;
            if attributes.peek().is_some() {
                writeln!(self)?;
            }
        }

        Ok(())
    }

    fn visit_function_attribute(&mut self, attribute: &mut FunctionAttribute) -> VResult {
        match attribute {
            FunctionAttribute::Mutability(mutability) => write!(self, "{mutability}")?,
            FunctionAttribute::Visibility(visibility) => write!(self, "{visibility}")?,
            FunctionAttribute::Virtual(_) => write!(self, "virtual")?,
            FunctionAttribute::Override(_, args) => {
                write!(self, "override")?;
                if !args.is_empty() {
                    write!(self, "(")?;

                    let args = args.iter().map(|arg| &arg.name).collect::<Vec<_>>();

                    let multiline = self.is_separated_multiline(&args, ", ");

                    if multiline {
                        writeln!(self)?;
                        self.indent(1);
                    }

                    self.write_items_separated(&args, ", ", multiline)?;

                    if multiline {
                        self.dedent(1);
                        writeln!(self)?;
                    }

                    write!(self, ")")?;
                }
            }
            FunctionAttribute::BaseOrModifier(_, base) => {
                let is_contract_base = self.context.contract.as_ref().map_or(false, |contract| {
                    contract
                        .base
                        .iter()
                        .any(|contract_base| contract_base.name.name == base.name.name)
                });

                if is_contract_base {
                    base.visit(self)?;
                } else {
                    let base_or_modifier = self.visit_to_string(base)?;
                    write!(
                        self,
                        "{}",
                        base_or_modifier.strip_suffix("()").unwrap_or(&base_or_modifier)
                    )?;
                }
            }
        };

        Ok(())
    }

    fn visit_base(&mut self, base: &mut Base) -> VResult {
        let need_parents = self.context.function.is_some() || base.args.is_some();

        write!(self, "{}", base.name.name)?;

        if need_parents {
            self.visit_opening_paren()?;
        }

        if let Some(args) = &mut base.args {
            let args = args
                .iter_mut()
                .map(|arg| self.visit_to_string(arg))
                .collect::<Result<Vec<_>, _>>()?;

            let multiline = self.is_separated_multiline(&args, ", ");

            if multiline {
                writeln!(self)?;
                self.indent(1);
            }

            self.write_items_separated(&args, ", ", multiline)?;

            if multiline {
                self.dedent(1);
                writeln!(self)?;
            }
        }

        if need_parents {
            self.visit_closing_paren()?;
        }

        Ok(())
    }

    fn visit_parameter(&mut self, parameter: &mut Parameter) -> VResult {
        parameter.ty.visit(self)?;

        if let Some(storage) = &parameter.storage {
            write!(self, " {storage}")?;
        }

        if let Some(name) = &parameter.name {
            write!(self, " {}", name.name)?;
        }

        Ok(())
    }

    /// Write parameter list with opening and closing parenthesis respecting multiline case.
    /// More info in [Visitor::visit_parameter_list].
    fn visit_parameter_list(&mut self, list: &mut ParameterList) -> VResult {
        let params = list
            .iter_mut()
            .map(|(_, param)| param.as_mut().map(|param| self.visit_to_string(param)).transpose())
            .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        let params_multiline = params.len() > 2 || self.is_separated_multiline(&params, ", ");

        self.visit_opening_paren()?;
        if params_multiline {
            writeln!(self)?;
            self.indent(1);
        }
        self.write_items_separated(&params, ", ", params_multiline)?;
        if params_multiline {
            self.dedent(1);
            writeln!(self)?;
        }
        self.visit_closing_paren()?;

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

        let multiline = self.source[loc.start()..loc.end()].contains('\n');

        if multiline {
            writeln!(self, "{{")?;
            self.indent(1);
        } else {
            self.write_opening_bracket()?;
        }

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

    fn visit_opening_paren(&mut self) -> VResult {
        write!(self, "(")?;

        Ok(())
    }

    fn visit_closing_paren(&mut self) -> VResult {
        write!(self, ")")?;

        Ok(())
    }

    fn visit_newline(&mut self) -> VResult {
        writeln!(self)?;

        Ok(())
    }

    fn visit_event(&mut self, event: &mut EventDefinition) -> VResult {
        if !event.doc.is_empty() {
            event.doc.visit(self)?;
            writeln!(self)?;
        }

        write!(self, "event {}(", event.name.name)?;

        let params = event
            .fields
            .iter_mut()
            .map(|param| self.visit_to_string(param))
            .collect::<Result<Vec<_>, _>>()?;

        let multiline = self.is_separated_multiline(&params, ", ");

        if multiline {
            writeln!(self)?;
            self.indent(1);
        }

        self.write_items_separated(&params, ", ", multiline)?;

        if multiline {
            self.dedent(1);
            writeln!(self)?;
        }

        write!(self, ")")?;

        if event.anonymous {
            write!(self, " anonymous")?;
        }

        write!(self, ";")?;

        Ok(())
    }

    fn visit_event_parameter(&mut self, param: &mut EventParameter) -> VResult {
        param.ty.visit(self)?;

        if param.indexed {
            write!(self, " indexed")?;
        }
        if let Some(name) = &param.name {
            write!(self, " {}", name.name)?;
        }

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
    fn constructor_definition() {
        test_directory("ConstructorDefinition");
    }

    #[test]
    fn contract_definition() {
        test_directory("ContractDefinition");
    }

    #[test]
    fn enum_definition() {
        test_directory("EnumDefinition");
    }

    #[test]
    fn event_definition() {
        test_directory("EventDefinition");
    }

    #[test]
    fn function_definition() {
        test_directory("FunctionDefinition");
    }

    #[test]
    fn import_directive() {
        test_directory("ImportDirective");
    }

    #[test]
    fn modifier_definition() {
        test_directory("ModifierDefinition");
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
