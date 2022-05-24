//! A Solidity formatter

use std::fmt::Write;

use indent_write::fmt::IndentWriter;
use itertools::Itertools;
use solang_parser::pt::*;

use crate::{
    comments::Comments,
    helpers,
    solang_ext::*,
    visit::{ParameterList, VError, VResult, Visitable, Visitor},
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

struct FormatBuffer<W: Sized> {
    level: usize,
    tab_width: usize,
    is_beggining_of_line: bool,
    is_beggining_of_group: bool,
    last_indent: String,
    last_char: Option<char>,
    current_line_len: usize,
    w: W,
}

impl<W: Sized> FormatBuffer<W> {
    fn new(w: W, tab_width: usize) -> Self {
        Self {
            w,
            tab_width,
            level: 0,
            current_line_len: 0,
            is_beggining_of_line: true,
            is_beggining_of_group: true,
            last_indent: String::new(),
            last_char: None,
        }
    }

    fn indent(&mut self, delta: usize) {
        self.level += delta;
    }

    fn dedent(&mut self, delta: usize) {
        self.level -= delta;
    }

    fn len_indented_with_current(&self, s: impl AsRef<str>) -> usize {
        self.last_indent
            .len()
            .saturating_add(self.current_line_len)
            .saturating_add(s.as_ref().len())
    }

    fn is_beginning_of_line(&self) -> bool {
        self.is_beggining_of_line
    }

    // fn is_beginning_of_group(&self) -> bool {
    //     self.is_beggining_of_group
    // }

    // fn start_group(&mut self) {
    //     self.is_beggining_of_group = true
    // }

    fn last_char_is_whitespace(&self) -> bool {
        self.last_char.map(|ch| ch.is_whitespace()).unwrap_or(true)
    }
}

impl<W: Write> FormatBuffer<W> {
    fn write_raw(&mut self, s: impl AsRef<str>) -> std::fmt::Result {
        self.w.write_str(s.as_ref())
    }
}

impl<W: Write> Write for FormatBuffer<W> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        if s.is_empty() {
            return Ok(())
        }

        if self.is_beggining_of_line && !s.trim_start().is_empty() {
            let level = if self.is_beggining_of_group { self.level } else { self.level + 1 };
            let indent = " ".repeat(self.tab_width * level);
            self.write_raw(&indent)?;
            self.last_indent = indent;
        }

        let indent = " ".repeat(self.tab_width * (self.level + 1));
        IndentWriter::new_skip_initial(&indent, &mut self.w).write_str(s)?;

        if let Some(last_char) = s.chars().next_back() {
            self.last_char = Some(last_char);
        }

        if s.contains('\n') {
            self.last_indent = indent;
            self.is_beggining_of_line = s.ends_with('\n');
            if self.is_beggining_of_line {
                self.current_line_len = 0;
            } else {
                self.current_line_len = s.lines().last().unwrap().len();
            }
        } else {
            self.is_beggining_of_line = false;
            self.current_line_len += s.len();
        }

        Ok(())
    }
}

// TODO: store context entities as references without copying
#[derive(Default)]
struct Context {
    contract: Option<ContractDefinition>,
    function: Option<FunctionDefinition>,
}

/// A Solidity formatter
pub struct Formatter<'a, W> {
    buf: FormatBuffer<&'a mut W>,
    source: &'a str,
    config: FormatterConfig,
    temp_bufs: Vec<FormatBuffer<String>>,
    context: Context,
    comments: Comments,
}

macro_rules! write_chunk {
    ($self:ident, $loc:expr) => {{
        write_chunk!($self, $loc, "")
    }};
    ($self:ident, $loc:expr, $($arg:tt)*) => {{
        // println!("write_chunk[{}:{}]", file!(), line!());
        $self.write_chunk($loc, format_args!($($arg)*))
    }};
}

macro_rules! writeln_chunk {
    ($self:ident, $loc:expr) => {{
        writeln_chunk!($self, $loc, "")
    }};
    ($self:ident, $loc:expr, $($arg:tt)*) => {{
        write_chunk!($self, $loc, "{}\n", format_args!($($arg)*))
    }};
}

macro_rules! buf_fn {
    ($vis:vis fn $name:ident(&self $(,)? $($arg_name:ident : $arg_ty:ty),*) $(-> $ret:ty)?) => {
        $vis fn $name(&self, $($arg_name : $arg_ty),*) $(-> $ret)? {
            if self.temp_bufs.is_empty() {
                self.buf.$name($($arg_name),*)
            } else {
                self.temp_bufs.last().unwrap().$name($($arg_name),*)
            }
        }
    };
    ($vis:vis fn $name:ident(&mut self $(,)? $($arg_name:ident : $arg_ty:ty),*) $(-> $ret:ty)?) => {
        $vis fn $name(&mut self, $($arg_name : $arg_ty),*) $(-> $ret)? {
            if self.temp_bufs.is_empty() {
                self.buf.$name($($arg_name),*)
            } else {
                self.temp_bufs.last_mut().unwrap().$name($($arg_name),*)
            }
        }
    };
}

impl<'a, W: Write> Formatter<'a, W> {
    pub fn new(w: &'a mut W, source: &'a str, comments: Comments, config: FormatterConfig) -> Self {
        Self {
            buf: FormatBuffer::new(w, config.tab_width),
            source,
            config,
            temp_bufs: Vec::new(),
            context: Context::default(),
            comments,
        }
    }

    fn buf(&mut self) -> &mut dyn Write {
        if self.temp_bufs.is_empty() {
            &mut self.buf as &mut dyn Write
        } else {
            self.temp_bufs.last_mut().unwrap() as &mut dyn Write
        }
    }

    buf_fn! { fn indent(&mut self, delta: usize) }
    buf_fn! { fn dedent(&mut self, delta: usize) }
    // buf_fn! { fn current_indent_len(&self) -> usize }
    buf_fn! { fn len_indented_with_current(&self, s: impl AsRef<str>) -> usize }
    buf_fn! { fn is_beginning_of_line(&self) -> bool }
    buf_fn! { fn last_char_is_whitespace(&self) -> bool }
    buf_fn! { fn write_raw(&mut self, s: impl AsRef<str>) -> std::fmt::Result }

    /// Write opening bracket with respect to `config.bracket_spacing` setting:
    /// `"{ "` if `true`, `"{"` if `false`
    fn write_opening_bracket(&mut self) -> std::fmt::Result {
        let bracket = if self.config.bracket_spacing { "{ " } else { "{" };
        write!(self.buf(), "{}", bracket)
    }

    /// Write closing bracket with respect to `config.bracket_spacing` setting:
    /// `" }"` if `true`, `"}"` if `false`
    fn write_closing_bracket(&mut self) -> std::fmt::Result {
        let bracket = if self.config.bracket_spacing { " }" } else { "}" };
        write!(self.buf(), "{}", bracket)
    }

    /// Write empty brackets with respect to `config.bracket_spacing` setting:
    /// `"{ }"` if `true`, `"{}"` if `false`
    fn write_empty_brackets(&mut self) -> std::fmt::Result {
        let brackets = if self.config.bracket_spacing { "{ }" } else { "{}" };
        write!(self.buf(), "{}", brackets)
    }

    /// Is length of the `text` with respect to already written line <= `config.line_length`
    fn will_it_fit(&self, text: impl AsRef<str>) -> bool {
        if text.as_ref().contains('\n') {
            return false
        }
        self.len_indented_with_current(text) <= self.config.line_length
    }

    fn will_chunk_fit(&self, byte_end: usize, chunk: impl std::fmt::Display) -> bool {
        let mut string = chunk.to_string();
        if string.contains('\n') {
            return false
        }
        // we don't care about order we just care about string length
        for comment in self.comments.get_comments_before(byte_end) {
            if comment.needs_newline() {
                return false
            }

            string.push_str(&format!(" {} ", comment.comment))
        }
        self.will_it_fit(string)
    }

    fn are_chunks_separated_multiline<'b>(
        &self,
        items: impl IntoIterator<Item = &'b (usize, impl std::fmt::Display + 'b)> + 'b,
        separator: &str,
    ) -> bool {
        let mut string = String::new();
        let mut items = items.into_iter().peekable();
        let mut max_byte_end: Option<usize> = None;
        while let Some((byte_end, item)) = items.next() {
            // find end location of items
            max_byte_end = Some(if let Some(old_max) = max_byte_end {
                if *byte_end > old_max {
                    *byte_end
                } else {
                    old_max
                }
            } else {
                *byte_end
            });
            let item = item.to_string();
            if item.contains('\n') {
                return true
            }
            // create separated string
            string.push_str(&item);
            if items.peek().is_some() {
                string.push_str(separator);
            }
        }
        if let Some(byte_end) = max_byte_end {
            !self.will_chunk_fit(byte_end, string)
        } else {
            // impllies empty items
            false
        }
    }

    fn write_chunks<'b>(
        &mut self,
        items: impl IntoIterator<Item = &'b (usize, impl std::fmt::Display + 'b)> + 'b,
        multiline: bool,
    ) -> std::fmt::Result {
        self.write_chunks_separated(items, "", multiline)
    }

    /// Write `items` separated by `separator` with respect to `config.line_length` setting
    fn write_chunks_separated<'b>(
        &mut self,
        items: impl IntoIterator<Item = &'b (usize, impl std::fmt::Display + 'b)> + 'b,
        separator: &str,
        multiline: bool,
    ) -> std::fmt::Result {
        let separator =
            if multiline { format!("{}\n", separator.trim_end()) } else { separator.to_string() };
        let mut items = items.into_iter().peekable();
        while let Some((byte_end, item)) = items.next() {
            write_chunk!(self, *byte_end, "{}", item)?;

            if let Some((next_byte_end, _)) = items.peek() {
                write!(self.buf(), "{}", separator)?;
                write_chunk!(self, *next_byte_end)?;
            }
        }

        Ok(())
    }

    fn write_chunks_separated_with_paren<'b>(
        &mut self,
        items: impl IntoIterator<Item = &'b (usize, impl std::fmt::Display + 'b)> + 'b,
        separator: &str,
        multiline: bool,
    ) -> Result<(), VError> {
        self.visit_opening_paren()?;
        if multiline {
            writeln!(self.buf())?;
            self.indent(1);
        }
        self.write_chunks_separated(items, separator, multiline)?;
        if multiline {
            self.dedent(1);
            writeln!(self.buf())?;
        }
        self.visit_closing_paren()?;
        Ok(())
    }

    fn visit_to_string(&mut self, visitable: &mut impl Visitable) -> Result<String, VError> {
        self.temp_bufs.push(FormatBuffer::new(String::new(), self.config.tab_width));
        visitable.visit(self)?;
        let buf = self.temp_bufs.pop().unwrap();
        Ok(buf.w)
    }

    /// Returns number of blank lines between two LOCs
    fn blank_lines(&self, a: Loc, b: Loc) -> usize {
        self.source[a.end()..b.start()].matches('\n').count()
    }

    fn write_postfix_comments_before(&mut self, byte_end: usize) -> std::fmt::Result {
        while let Some(postfix) = self.comments.pop_postfix(byte_end) {
            if !self.is_beginning_of_line() && !self.last_char_is_whitespace() {
                write!(self.buf(), " ")?;
            }
            if postfix.is_line() {
                // TODO handle indent for blocks (most likely handled by some kind of block
                // context)
                writeln!(self.buf(), "{}", postfix.comment)?;
            } else {
                write!(self.buf(), "{}", postfix.comment)?;
            }
        }
        Ok(())
    }

    fn write_prefix_comments_before(&mut self, byte_end: usize) -> std::fmt::Result {
        if !self.is_beginning_of_line() && self.comments.peek_prefix(byte_end).is_some() {
            writeln!(self.buf())?;
        }
        while let Some(prefix) = self.comments.pop_prefix(byte_end) {
            writeln!(self.buf(), "{}", prefix.comment)?;
        }
        Ok(())
    }

    fn write_chunk(&mut self, byte_end: usize, chunk: impl std::fmt::Display) -> std::fmt::Result {
        let last_char_was_whitespace = self.last_char_is_whitespace();
        self.write_postfix_comments_before(byte_end)?;
        self.write_prefix_comments_before(byte_end)?;
        if last_char_was_whitespace && !self.last_char_is_whitespace() {
            write!(self.buf(), " ")?;
        }
        write!(self.buf(), "{}", chunk)
    }
}

// Traverse the Solidity Parse Tree and write to the code formatter
impl<'a, W: Write> Visitor for Formatter<'a, W> {
    fn visit_source(&mut self, loc: Loc) -> VResult {
        let source = String::from_utf8(self.source.as_bytes()[loc.start()..loc.end()].to_vec())?;
        let mut lines = source.splitn(2, '\n');

        write_chunk!(self, loc.end(), "{}", lines.next().unwrap())?;
        if let Some(remainder) = lines.next() {
            // Call with `self.write_str` and not `write!`, so we can have `\n` at the beginning
            // without triggering an indentation
            self.write_raw(&format!("\n{remainder}"))?;
        }

        let _ = self.comments.remove_comments_before(loc.end());

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
                |u: &SourceUnitPart| matches!(u, SourceUnitPart::PragmaDirective(_, _, _));
            let is_import = |u: &SourceUnitPart| matches!(u, SourceUnitPart::ImportDirective(_));
            let is_error = |u: &SourceUnitPart| matches!(u, SourceUnitPart::ErrorDefinition(_));
            let is_declaration =
                |u: &SourceUnitPart| !(is_pragma(u) || is_import(u) || is_error(u));
            let is_comment = |u: &SourceUnitPart| matches!(u, SourceUnitPart::DocComment(_));

            unit.visit(self)?;

            if let Some(next_unit) = source_unit_parts_iter.peek() {
                self.write_postfix_comments_before(next_unit.loc().start())?;

                if !is_comment(unit) && !self.is_beginning_of_line() {
                    writeln!(self.buf())?;
                }

                // If source has zero blank lines between imports or errors, leave it as is. If one
                // or more, separate with one blank line.
                let separate = (is_import(unit) || is_error(unit)) &&
                    (is_import(next_unit) || is_error(next_unit)) &&
                    self.blank_lines(unit.loc(), next_unit.loc()) > 1;

                if (is_declaration(unit) || is_declaration(next_unit)) ||
                    (is_pragma(unit) || is_pragma(next_unit)) ||
                    separate
                {
                    writeln!(self.buf())?;
                }
            }
        }

        let mut comments = self.comments.drain().into_iter().peekable();
        while let Some(comment) = comments.next() {
            if comment.is_prefix() {
                writeln!(self.buf())?;
            } else if !self.is_beginning_of_line() {
                write!(self.buf(), " ")?;
            }
            if comment.is_line() && comments.peek().is_some() {
                writeln!(self.buf(), "{}", comment.comment)?;
            } else {
                write!(self.buf(), "{}", comment.comment)?;
            }
        }
        Ok(())
    }

    fn visit_doc_comment(&mut self, doc_comment: &mut DocComment) -> VResult {
        match doc_comment.ty {
            CommentType::Line => {
                write!(self.buf(), "///{}", doc_comment.comment)?;
            }
            CommentType::Block => {
                let lines = doc_comment
                    .comment
                    .trim_end()
                    .lines()
                    .map(|line| line.trim_start())
                    .peekable()
                    .collect::<Vec<_>>();
                if lines.iter().skip(1).all(|line| line.starts_with('*')) {
                    writeln!(self.buf(), "/**")?;
                    let mut lines = lines.into_iter();
                    if let Some(first_line) = lines.next() {
                        if !first_line.is_empty() {
                            // write the original first line
                            writeln!(
                                self.buf(),
                                " *{}",
                                doc_comment.comment.lines().next().unwrap()
                            )?;
                        }
                    }
                    for line in lines {
                        writeln!(self.buf(), " *{}", &line[1..])?;
                    }
                    write!(self.buf(), " */")?;
                } else {
                    write!(self.buf(), "/**")?;
                    self.write_raw(&doc_comment.comment)?;
                    write!(self.buf(), "*/")?;
                }
            }
        }

        Ok(())
    }

    fn visit_doc_comments(&mut self, doc_comments: &mut [DocComment]) -> VResult {
        let mut iter = doc_comments.iter_mut();
        if let Some(doc_comment) = iter.next() {
            doc_comment.visit(self)?
        }
        for doc_comment in iter {
            writeln!(self.buf())?;
            doc_comment.visit(self)?;
        }

        Ok(())
    }

    fn visit_contract(&mut self, contract: &mut ContractDefinition) -> VResult {
        self.context.contract = Some(contract.clone());

        write_chunk!(self, contract.loc.start(), "{} ", contract.ty)?;
        write_chunk!(self, contract.name.loc.end(), "{} ", contract.name.name)?;

        if !contract.base.is_empty() {
            // TODO check if chunk fits?
            write_chunk!(self, contract.base.first().unwrap().loc.start(), "is")?;

            let bases = contract
                .base
                .iter_mut()
                .map(|base| Ok((base.loc.end(), self.visit_to_string(base)?)))
                .collect::<Result<Vec<_>, VError>>()?;

            let multiline = self.are_chunks_separated_multiline(&bases, ", ");

            if multiline {
                writeln!(self.buf())?;
                self.indent(1);
            } else {
                write!(self.buf(), " ")?;
            }

            self.write_chunks_separated(&bases, ", ", multiline)?;

            if multiline {
                self.dedent(1);
                writeln!(self.buf())?;
            } else {
                write!(self.buf(), " ")?;
            }
        }

        if contract.parts.is_empty() {
            self.write_empty_brackets()?;
        } else {
            writeln!(self.buf(), "{{")?;

            self.indent(1);
            let mut contract_parts_iter = contract.parts.iter_mut().peekable();
            while let Some(part) = contract_parts_iter.next() {
                part.visit(self)?;
                writeln!(self.buf())?;

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
                        writeln!(self.buf())?;
                    }
                }
            }
            self.dedent(1);

            write!(self.buf(), "}}")?;
        }

        self.context.contract = None;

        Ok(())
    }

    fn visit_pragma(&mut self, ident: &mut Identifier, str: &mut StringLiteral) -> VResult {
        write!(self.buf(), "pragma {} ", &ident.name)?;

        #[allow(clippy::if_same_then_else)]
        if ident.name == "solidity" {
            // There are some issues with parsing Solidity's versions with crates like `semver`:
            // 1. Ranges like `>=0.4.21<0.6.0` or `>=0.4.21 <0.6.0` are not parseable at all.
            // 2. Versions like `0.8.10` got transformed into `^0.8.10` which is not the same.
            // TODO: semver-solidity crate :D
            write!(self.buf(), "{};", str.string)?;
        } else {
            write!(self.buf(), "{};", str.string)?;
        }

        Ok(())
    }

    fn visit_import_plain(&mut self, import: &mut StringLiteral) -> VResult {
        write!(self.buf(), "import \"{}\";", &import.string)?;

        Ok(())
    }

    fn visit_import_global(
        &mut self,
        global: &mut StringLiteral,
        alias: &mut Identifier,
    ) -> VResult {
        write!(self.buf(), "import \"{}\" as {};", global.string, alias.name)?;

        Ok(())
    }

    fn visit_import_renames(
        &mut self,
        imports: &mut [(Identifier, Option<Identifier>)],
        from: &mut StringLiteral,
    ) -> VResult {
        write!(self.buf(), "import ")?;

        let mut imports = imports
            .iter()
            .map(|(ident, alias)| {
                (
                    ident.loc.end(), // TODO:
                    format!(
                        "{}{}",
                        ident.name,
                        alias
                            .as_ref()
                            .map_or("".to_string(), |alias| format!(" as {}", alias.name))
                    ),
                )
            })
            .collect::<Vec<_>>();
        imports.sort();

        let multiline = self.are_chunks_separated_multiline(&imports, ", ");

        if multiline {
            writeln!(self.buf(), "{{")?;
            self.indent(1);
        } else {
            self.write_opening_bracket()?;
        }

        self.write_chunks_separated(&imports, ", ", multiline)?;

        if multiline {
            self.dedent(1);
            write!(self.buf(), "\n}}")?;
        } else {
            self.write_closing_bracket()?;
        }

        write!(self.buf(), " from \"{}\";", from.string)?;

        Ok(())
    }

    fn visit_enum(&mut self, enumeration: &mut EnumDefinition) -> VResult {
        write_chunk!(self, enumeration.loc.start(), "enum {} ", &enumeration.name.name)?;

        if enumeration.values.is_empty() {
            self.write_empty_brackets()?;
        } else {
            // TODO rewrite with some enumeration
            write!(self.buf(), "{{")?;

            self.indent(1);
            for (i, value) in enumeration.values.iter().enumerate() {
                writeln_chunk!(self, value.loc.start())?;
                write_chunk!(self, value.loc.end(), "{}", &value.name)?;

                if i != enumeration.values.len() - 1 {
                    write!(self.buf(), ",")?;
                }
            }
            self.dedent(1);

            self.write_postfix_comments_before(enumeration.loc.end())?;
            self.write_prefix_comments_before(enumeration.loc.end())?;
            writeln!(self.buf())?;
            write!(self.buf(), "}}")?;
        }

        Ok(())
    }

    fn visit_expr(&mut self, loc: Loc, expr: &mut Expression) -> VResult {
        match expr {
            Expression::Type(loc, typ) => match typ {
                Type::Address => write_chunk!(self, loc.start(), "address")?,
                Type::AddressPayable => write_chunk!(self, loc.start(), "address payable")?,
                Type::Payable => write_chunk!(self, loc.start(), "payable")?,
                Type::Bool => write_chunk!(self, loc.start(), "bool")?,
                Type::String => write_chunk!(self, loc.start(), "string")?,
                Type::Int(n) => write_chunk!(self, loc.start(), "int{}", n)?,
                Type::Uint(n) => write_chunk!(self, loc.start(), "uint{}", n)?,
                Type::Bytes(n) => write_chunk!(self, loc.start(), "bytes{}", n)?,
                Type::Rational => write_chunk!(self, loc.start(), "rational")?,
                Type::DynamicBytes => write_chunk!(self, loc.start(), "bytes")?,
                Type::Mapping(loc, from, to) => {
                    write_chunk!(self, loc.start(), "mapping(")?;
                    from.visit(self)?;
                    write!(self.buf(), " => ")?;
                    to.visit(self)?;
                    write!(self.buf(), ")")?;
                }
                Type::Function { .. } => self.visit_source(*loc)?,
            },
            Expression::ArraySubscript(_, ty_exp, size_exp) => {
                ty_exp.visit(self)?;
                write!(self.buf(), "[")?;
                if let Some(size_exp) = size_exp {
                    size_exp.visit(self)?;
                }
                write!(self.buf(), "]")?;
            }
            _ => self.visit_source(loc)?,
        };

        Ok(())
    }

    fn visit_ident(&mut self, loc: Loc, ident: &mut Identifier) -> VResult {
        write_chunk!(self, loc.end(), "{}", ident.name)?;
        Ok(())
    }

    fn visit_emit(&mut self, _loc: Loc, event: &mut Expression) -> VResult {
        write!(self.buf(), "emit ")?;
        event.loc().visit(self)?;
        write!(self.buf(), ";")?;

        Ok(())
    }

    fn visit_var_declaration(&mut self, var: &mut VariableDeclaration) -> VResult {
        var.ty.visit(self)?;

        if let Some(storage) = &var.storage {
            write!(self.buf(), " {}", storage)?;
        }

        write!(self.buf(), " {}", var.name.name)?;

        Ok(())
    }

    fn visit_break(&mut self) -> VResult {
        write!(self.buf(), "break;")?;

        Ok(())
    }

    fn visit_continue(&mut self) -> VResult {
        write!(self.buf(), "continue;")?;

        Ok(())
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> VResult {
        self.context.function = Some(func.clone());

        write!(self.buf(), "{}", func.ty)?;

        if let Some(Identifier { name, .. }) = &func.name {
            write!(self.buf(), " {name}")?;
        }

        // TODO:
        // let params = self.visit_to_string(&mut func.params)?;
        // let params_multiline = params.contains('\n');
        let params = func
            .params
            .iter_mut()
            .map(|(loc, param)| {
                Ok((loc.end(), self.visit_to_string(param.as_mut().unwrap() /* TODO: */)?))
            })
            .collect::<Result<Vec<_>, VError>>()?;
        let params_multiline = self.are_chunks_separated_multiline(&params, ", ");
        self.write_chunks_separated_with_paren(&params, ", ", params_multiline)?;

        // TODO:
        let attributes = self.visit_to_string(&mut func.attributes)?;
        let attributes = attributes.lines().collect::<Vec<_>>();

        // TODO:
        // let returns = self.visit_to_string(&mut func.returns)?;
        // let returns_multiline = returns.contains('\n');
        let returns = func
            .returns
            .iter_mut()
            .map(|(loc, param)| {
                Ok((loc.end(), self.visit_to_string(param.as_mut().unwrap() /* TODO: */)?))
            })
            .collect::<Result<Vec<_>, VError>>()?;
        let returns_multiline =
            self.are_chunks_separated_multiline(&returns, ", ") || params_multiline;
        let returns_indent = !attributes.is_empty() || returns_multiline;

        // Compose one line string consisting of attributes and return parameters.
        let attributes_returns = format!(
            "{}{}",
            attributes.join(" "),
            if func.returns.is_empty() {
                "".to_string()
            } else {
                format!(" returns ({})", returns.iter().map(|r| r.1.clone()).join(", ")) // TODO:
            }
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

        let attributes_returns_multiline = !self
            .will_it_fit(&format!(" {attributes_returns}{body_first_line}")) ||
            (!returns.is_empty() && returns_multiline);

        // Check that we can fit both attributes and return arguments in one line.
        if !attributes_returns.is_empty() && !attributes_returns_multiline {
            write!(self.buf(), " {attributes_returns}")?;
        } else {
            // If attributes and returns can't fit in one line, we write all attributes in
            // multiple lines.
            if !func.attributes.is_empty() {
                writeln!(self.buf())?;
                self.indent(1);
                func.attributes.visit(self)?;
                self.dedent(1);
            }

            if !func.returns.is_empty() {
                if returns_indent {
                    self.indent(1);
                }
                writeln!(self.buf())?;
                write!(self.buf(), "returns ")?;

                self.write_chunks_separated_with_paren(&returns, ", ", returns_multiline)?;

                if returns_indent {
                    self.dedent(1);
                }
            }
        }

        match body {
            Some(body) => {
                if self.will_it_fit(format!(" {}", body_first_line)) &&
                    !attributes_returns_multiline
                {
                    write!(self.buf(), " ")?;
                } else {
                    writeln!(self.buf())?;
                }
                // TODO: when we implement visitors for statements, write `body_string` here instead
                //  of visiting it twice.
                body.visit(self)?;
            }
            None => write!(self.buf(), ";")?,
        }

        self.context.function = None;

        Ok(())
    }

    /// Write each function attribute on a new line because we don't have enough information in
    /// visit function regarding one line/multiline cases. We can transform it into one line later
    /// by `.split("\n").join(" ")`.
    fn visit_function_attribute_list(&mut self, list: &mut Vec<FunctionAttribute>) -> VResult {
        let mut attributes = list.iter_mut().attr_sorted().peekable();

        while let Some(attribute) = attributes.next() {
            attribute.visit(self)?;
            if attributes.peek().is_some() {
                writeln!(self.buf())?;
            }
        }

        Ok(())
    }

    fn visit_function_attribute(&mut self, attribute: &mut FunctionAttribute) -> VResult {
        match attribute {
            FunctionAttribute::Mutability(mutability) => write!(self.buf(), "{mutability}")?,
            FunctionAttribute::Visibility(visibility) => write!(self.buf(), "{visibility}")?,
            FunctionAttribute::Virtual(_) => write!(self.buf(), "virtual")?,
            FunctionAttribute::Immutable(_) => write!(self.buf(), "immutable")?,
            FunctionAttribute::Override(_, args) => {
                write!(self.buf(), "override")?;
                if !args.is_empty() {
                    let args =
                        args.iter().map(|arg| (arg.loc.end(), &arg.name)).collect::<Vec<_>>();
                    let multiline = self.are_chunks_separated_multiline(&args, ", ");
                    self.write_chunks_separated_with_paren(&args, ", ", multiline)?;
                }
            }
            FunctionAttribute::BaseOrModifier(_, base) => {
                let is_contract_base = self.context.contract.as_ref().map_or(false, |contract| {
                    contract.base.iter().any(|contract_base| {
                        helpers::namespace_matches(&contract_base.name, &base.name)
                    })
                });

                if is_contract_base {
                    base.visit(self)?;
                } else {
                    let base_or_modifier = self.visit_to_string(base)?;
                    write!(
                        self.buf(),
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

        self.visit_expr(LineOfCode::loc(&base.name), &mut base.name)?;

        if need_parents {
            self.visit_opening_paren()?;
        }

        if let Some(args) = &mut base.args {
            let args = args
                .iter_mut()
                .map(|arg| Ok((arg.loc().end(), self.visit_to_string(arg)?)))
                .collect::<Result<Vec<_>, VError>>()?;

            let multiline = self.are_chunks_separated_multiline(&args, ", ");

            if multiline {
                writeln!(self.buf())?;
                self.indent(1);
            }

            self.write_chunks_separated(&args, ", ", multiline)?;

            if multiline {
                self.dedent(1);
                writeln!(self.buf())?;
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
            write!(self.buf(), " {storage}")?;
        }

        if let Some(name) = &parameter.name {
            write!(self.buf(), " {}", name.name)?;
        }

        Ok(())
    }

    /// Write parameter list with opening and closing parenthesis respecting multiline case.
    /// More info in [Visitor::visit_parameter_list].
    fn visit_parameter_list(&mut self, list: &mut ParameterList) -> VResult {
        let params = list
            .iter_mut()
            .map(|(_, param)| {
                param
                    .as_mut()
                    .map(|param| Ok((param.loc.end(), self.visit_to_string(param)?)))
                    .transpose()
            })
            .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        let params_multiline =
            params.len() > 2 || self.are_chunks_separated_multiline(&params, ", ");
        self.write_chunks_separated_with_paren(&params, ", ", params_multiline)?;

        Ok(())
    }

    fn visit_struct(&mut self, structure: &mut StructDefinition) -> VResult {
        write!(self.buf(), "struct {} ", &structure.name.name)?;

        if structure.fields.is_empty() {
            self.write_empty_brackets()?;
        } else {
            writeln!(self.buf(), "{{")?;

            self.indent(1);
            for field in structure.fields.iter_mut() {
                field.visit(self)?;
                writeln!(self.buf(), ";")?;
            }
            self.dedent(1);

            write!(self.buf(), "}}")?;
        }

        Ok(())
    }

    fn visit_type_definition(&mut self, def: &mut TypeDefinition) -> VResult {
        write!(self.buf(), "type {} is ", def.name.name)?;
        def.ty.visit(self)?;
        write!(self.buf(), ";")?;

        Ok(())
    }

    fn visit_stray_semicolon(&mut self) -> VResult {
        write!(self.buf(), ";")?;

        Ok(())
    }

    fn visit_block(
        &mut self,
        loc: Loc,
        unchecked: bool,
        statements: &mut Vec<Statement>,
    ) -> VResult {
        if unchecked {
            write!(self.buf(), "unchecked ")?;
        }

        if statements.is_empty() {
            self.write_empty_brackets()?;
            return Ok(())
        }

        let multiline = self.source[loc.start()..loc.end()].contains('\n');

        if multiline {
            writeln!(self.buf(), "{{")?;
            self.indent(1);
        } else {
            self.write_opening_bracket()?;
        }

        let mut statements_iter = statements.iter_mut().peekable();
        while let Some(stmt) = statements_iter.next() {
            stmt.visit(self)?;
            if multiline {
                writeln!(self.buf())?;
            }

            // If source has zero blank lines between statements, leave it as is. If one
            //  or more, separate statements with one blank line.
            if let Some(next_stmt) = statements_iter.peek() {
                if self.blank_lines(LineOfCode::loc(stmt), LineOfCode::loc(next_stmt)) > 1 {
                    writeln!(self.buf())?;
                }
            }
        }

        if multiline {
            self.dedent(1);
            write!(self.buf(), "}}")?;
        } else {
            self.write_closing_bracket()?;
        }

        Ok(())
    }

    fn visit_opening_paren(&mut self) -> VResult {
        write!(self.buf(), "(")?;

        Ok(())
    }

    fn visit_closing_paren(&mut self) -> VResult {
        write!(self.buf(), ")")?;

        Ok(())
    }

    fn visit_newline(&mut self) -> VResult {
        writeln!(self.buf())?;

        Ok(())
    }

    fn visit_event(&mut self, event: &mut EventDefinition) -> VResult {
        write!(self.buf(), "event {}", event.name.name)?;

        let params = event
            .fields
            .iter_mut()
            .map(|param| Ok((param.loc.end(), self.visit_to_string(param)?)))
            .collect::<Result<Vec<_>, VError>>()?;

        // TODO:
        let multiline = !self.will_it_fit(format!(
            "{}){};",
            params.iter().map(|p| p.1.to_owned()).join(", "),
            if event.anonymous { " anonymous" } else { "" }
        ));

        self.write_chunks_separated_with_paren(&params, ", ", multiline)?;

        if event.anonymous {
            write!(self.buf(), " anonymous")?;
        }

        write!(self.buf(), ";")?;

        Ok(())
    }

    fn visit_event_parameter(&mut self, param: &mut EventParameter) -> VResult {
        param.ty.visit(self)?;

        if param.indexed {
            write!(self.buf(), " indexed")?;
        }
        if let Some(name) = &param.name {
            write!(self.buf(), " {}", name.name)?;
        }

        Ok(())
    }

    fn visit_error(&mut self, error: &mut ErrorDefinition) -> VResult {
        write!(self.buf(), "error {}", error.name.name)?;

        let params = error
            .fields
            .iter_mut()
            .map(|param| Ok((param.loc.end(), self.visit_to_string(param)?)))
            .collect::<Result<Vec<_>, VError>>()?;

        let multiline = self.are_chunks_separated_multiline(&params, ", ");
        self.write_chunks_separated_with_paren(&params, ", ", multiline)?;
        self.visit_stray_semicolon()?;
        Ok(())
    }

    fn visit_error_parameter(&mut self, param: &mut ErrorParameter) -> VResult {
        param.ty.visit(self)?;

        if let Some(name) = &param.name {
            write!(self.buf(), " {}", name.name)?;
        }

        Ok(())
    }

    fn visit_using(&mut self, using: &mut Using) -> VResult {
        write!(self.buf(), "using ")?;

        match &mut using.list {
            UsingList::Library(library) => {
                self.visit_expr(LineOfCode::loc(library), library)?;
            }
            UsingList::Functions(funcs) => {
                let func_strs = funcs
                    .iter_mut()
                    .map(|func| Ok((func.loc().end(), self.visit_to_string(func)?)))
                    .collect::<Result<Vec<_>, VError>>()?;
                let multiline = self.are_chunks_separated_multiline(func_strs.iter(), ", ");
                self.write_opening_bracket()?;
                self.write_chunks_separated(&func_strs, ", ", multiline)?;
                self.write_closing_bracket()?;
            }
        }

        write!(self.buf(), " for ")?;

        if let Some(ty) = &mut using.ty {
            ty.visit(self)?;
        } else {
            write!(self.buf(), "*")?;
        }

        if let Some(global) = &mut using.global {
            write!(self.buf(), " {}", global.name)?;
        }

        write!(self.buf(), ";")?;

        Ok(())
    }

    fn visit_var_definition(&mut self, var: &mut VariableDefinition) -> VResult {
        var.ty.visit(self)?;

        // TODO write chunks to string in order and then do sort
        let attributes = var
            .attrs
            .iter_mut()
            .attr_sorted()
            .map(|attribute| match attribute {
                VariableAttribute::Visibility(visibility) => {
                    (visibility.loc().unwrap().end(), visibility.to_string())
                }
                VariableAttribute::Constant(loc) => (loc.end(), "constant".to_string()),
                VariableAttribute::Immutable(loc) => (loc.end(), "immutable".to_string()),
                VariableAttribute::Override(loc) => (loc.end(), "override".to_string()),
            })
            .collect::<Vec<_>>();

        let mut multiline = self.are_chunks_separated_multiline(&attributes, " ");

        if !var.attrs.is_empty() {
            if multiline {
                writeln_chunk!(self, var.loc.end())?;
                self.indent(1);
            } else {
                write_chunk!(self, var.loc.end(), " ")?;
            }

            self.write_chunks_separated(&attributes, " ", multiline)?;
        }

        let variable = self.visit_to_string(&mut var.name)?;

        if self.will_it_fit(&format!(" {}", variable)) {
            write!(self.buf(), " {}", variable)?;
        } else {
            if !multiline {
                multiline = true;
                self.indent(1);
            }
            write_chunk!(self, var.name.loc.end(), "\n{}", variable)?;
        }

        if let Some(init) = &mut var.initializer {
            let loc = LineOfCode::loc(init);

            // does assignment with equals fit?
            if self.will_chunk_fit(loc.start(), " =") {
                write!(self.buf(), " =")?;
                write_chunk!(self, loc.start())?;
            } else {
                writeln!(self.buf(), " =")?;
                // write comments on new line
                write_chunk!(self, loc.start())?
            }

            let formatted_init = self.visit_to_string(init)?;
            if self.will_it_fit(format!(" {}", formatted_init)) {
                write!(self.buf(), " {}", formatted_init)?;
            } else {
                writeln!(self.buf())?;
                if !multiline {
                    self.indent(1);
                }
                write!(self.buf(), "{}", formatted_init)?;
                if !multiline {
                    self.dedent(1);
                }
            }
        }

        write!(self.buf(), ";")?;

        if multiline {
            self.dedent(1);
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

                        if let Some((key, value)) = entry.unwrap().split_once('=') {
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

    fn test_formatter(
        filename: &str,
        config: FormatterConfig,
        source: &str,
        expected_source: &str,
    ) {
        #[derive(PartialEq, Eq)]
        struct PrettyString(String);

        impl std::fmt::Debug for PrettyString {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        let (mut source_pt, source_comments) = solang_parser::parse(source, 1).unwrap();
        let source_comments = Comments::new(source_comments, source);

        let (mut expected_pt, expected_comments) =
            solang_parser::parse(expected_source, 1).unwrap();
        if !source_pt.ast_eq(&expected_pt) {
            pretty_assertions::assert_eq!(
                source_pt,
                expected_pt,
                "(formatted Parse Tree == expected Parse Tree) in {}",
                filename
            );
        }
        let expected_comments = Comments::new(expected_comments, expected_source);

        let expected = PrettyString(expected_source.trim().to_string());

        let mut source_formatted = String::new();
        let mut f = Formatter::new(&mut source_formatted, source, source_comments, config.clone());
        source_pt.visit(&mut f).unwrap();
        let source_formatted = PrettyString(source_formatted);

        pretty_assertions::assert_eq!(
            source_formatted,
            expected,
            "(formatted == expected) in {}",
            filename
        );

        let mut expected_formatted = String::new();
        let mut f =
            Formatter::new(&mut expected_formatted, expected_source, expected_comments, config);
        expected_pt.visit(&mut f).unwrap();
        let expected_formatted = PrettyString(expected_formatted);

        pretty_assertions::assert_eq!(
            expected_formatted,
            expected,
            "(formatted == expected) in {}",
            filename
        );
    }

    macro_rules! test_directory {
        ($dir:ident) => {
            #[allow(non_snake_case)]
            #[test]
            fn $dir() {
                test_directory(stringify!($dir));
            }
        };
    }

    test_directory! { ConstructorDefinition }
    test_directory! { ContractDefinition }
    test_directory! { DocComments }
    test_directory! { EnumDefinition }
    test_directory! { ErrorDefinition }
    test_directory! { EventDefinition }
    test_directory! { FunctionDefinition }
    test_directory! { FunctionType }
    test_directory! { ImportDirective }
    test_directory! { ModifierDefinition }
    test_directory! { StatementBlock }
    test_directory! { StructDefinition }
    test_directory! { TypeDefinition }
    test_directory! { UsingDirective }
    test_directory! { VariableDefinition }
    test_directory! { SimpleComments }
}
