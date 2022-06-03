//! A Solidity formatter

use std::fmt::Write;

use indent_write::fmt::IndentWriter;
use itertools::Itertools;
use solang_parser::pt::*;

use crate::{
    comments::{CommentWithMetadata, Comments},
    helpers,
    solang_ext::*,
    visit::{VError, VResult, Visitable, Visitor},
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

#[derive(Default, Clone)]
struct IndentGroup {
    skip_line: bool,
}

struct FormatBuffer<W: Sized> {
    indents: Vec<IndentGroup>,
    tab_width: usize,
    line_length: usize,
    is_beginning_of_line: bool,
    last_indent: String,
    last_char: Option<char>,
    current_line_len: usize,
    w: W,
}

impl<W: Sized> FormatBuffer<W> {
    fn new(w: W, tab_width: usize, line_length: usize) -> Self {
        Self {
            w,
            tab_width,
            line_length,
            indents: vec![],
            current_line_len: 0,
            is_beginning_of_line: true,
            last_indent: String::new(),
            last_char: None,
        }
    }

    fn indent(&mut self, delta: usize) {
        self.indents.extend(std::iter::repeat(IndentGroup::default()).take(delta));
    }

    fn dedent(&mut self, delta: usize) {
        self.indents.truncate(self.indents.len() - delta);
    }

    fn level(&self) -> usize {
        self.indents.iter().filter(|i| !i.skip_line).count()
    }

    fn last_indent_group_skipped(&self) -> bool {
        self.indents.last().map(|i| i.skip_line).unwrap_or(false)
    }

    fn set_last_indent_group_skipped(&mut self, skip_line: bool) {
        if let Some(i) = self.indents.last_mut() {
            i.skip_line = skip_line
        }
    }

    fn line_len(&self) -> usize {
        self.line_length
    }

    fn last_indent_len(&self) -> usize {
        self.last_indent.len()
    }

    fn current_line_len(&self) -> usize {
        self.current_line_len
    }

    fn is_beginning_of_line(&self) -> bool {
        self.is_beginning_of_line
    }

    fn start_group(&mut self) {
        self.indents.push(IndentGroup { skip_line: true });
    }

    fn end_group(&mut self) {
        self.indents.pop();
    }

    fn last_char(&self) -> Option<char> {
        self.last_char
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

        let mut level = self.level();

        // TODO:
        // println!(
        //     "str: {}. line: {}. group: {}",
        //     s, self.is_beginning_of_line, self.is_beginning_of_group
        // );
        if self.is_beginning_of_line && !s.trim_start().is_empty() {
            // TODO: println!("str: {}. level: {}", s, level);
            let indent = " ".repeat(self.tab_width * level);
            self.write_raw(&indent)?;
            self.last_indent = indent;
        }

        if self.last_indent_group_skipped() {
            level += 1;
        }
        let indent = " ".repeat(self.tab_width * level);
        IndentWriter::new_skip_initial(&indent, &mut self.w).write_str(s)?;

        if let Some(last_char) = s.chars().next_back() {
            self.last_char = Some(last_char);
        }

        if s.contains('\n') {
            self.set_last_indent_group_skipped(false);
            self.last_indent = indent;
            self.is_beginning_of_line = s.ends_with('\n');
            if self.is_beginning_of_line {
                self.current_line_len = 0;
            } else {
                self.current_line_len = s.lines().last().unwrap().len();
            }
        } else {
            self.is_beginning_of_line = false;
            self.current_line_len += s.len();
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Chunk {
    postfixes_before: Vec<CommentWithMetadata>,
    prefixes: Vec<CommentWithMetadata>,
    content: String,
    postfixes: Vec<CommentWithMetadata>,
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
    ($self:ident, $loc:expr, $format_str:literal) => {{
        write_chunk!($self, $loc, $format_str,)
    }};
    ($self:ident, $loc:expr, $format_str:literal, $($arg:tt)*) => {{
        // println!("write_chunk[{}:{}]", file!(), line!());
        let chunk = $self.chunk_at($loc, format_args!($format_str, $($arg)*), None);
        $self.write_chunk(&chunk)
    }};
    ($self:ident, $loc:expr, $end_loc:expr, $format_str:literal) => {{
        write_chunk!($self, $loc, $end_loc, $format_str,)
    }};
    ($self:ident, $loc:expr, $end_loc:expr, $format_str:literal, $($arg:tt)*) => {{
        // println!("write_chunk[{}:{}]", file!(), line!());
        let chunk = $self.chunk_at($loc, format_args!($format_str, $($arg)*), Some($end_loc));
        $self.write_chunk(&chunk)
    }};
}

macro_rules! writeln_chunk {
    ($self:ident, $loc:expr) => {{
        writeln_chunk!($self, $loc, "")
    }};
    ($self:ident, $loc:expr, $format_str:literal) => {{
        writeln_chunk!($self, $loc, $format_str,)
    }};
    ($self:ident, $loc:expr, $format_str:literal, $($arg:tt)*) => {{
        write_chunk!($self, $loc, "{}\n", format_args!($format_str, $($arg)*))
    }};
    ($self:ident, $loc:expr, $end_loc:expr, $format_str:literal) => {{
        writeln_chunk!($self, $loc, $end_loc, $format_str,)
    }};
    ($self:ident, $loc:expr, $end_loc:expr, $format_str:literal, $($arg:tt)*) => {{
        write_chunk!($self, $loc, $end_loc, "{}\n", format_args!($format_str, $($arg)*))
    }};
}

macro_rules! format_chunk {
    ($self:ident, $loc:expr) => {{
        format_chunk!($self, $loc, "")
    }};
    ($self:ident, $loc:expr, $($arg:tt)*) => {{
        // println!("format_chunk[{}:{}]", file!(), line!());
        $self.format_chunk($loc, format_args!($($arg)*))
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
            buf: FormatBuffer::new(w, config.tab_width, config.line_length),
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

    fn push_temp_buf(&mut self) {
        let mut buffer = FormatBuffer::new(
            String::new(),
            self.config.tab_width,
            self.line_len().saturating_sub(self.last_indent_len()),
        );
        buffer.current_line_len = self.current_line_len();
        self.temp_bufs.push(buffer);
    }

    fn pop_temp_buf(&mut self) -> Option<FormatBuffer<String>> {
        self.temp_bufs.pop()
    }

    buf_fn! { fn indent(&mut self, delta: usize) }
    buf_fn! { fn dedent(&mut self, delta: usize) }
    buf_fn! { fn start_group(&mut self) }
    buf_fn! { fn end_group(&mut self) }
    buf_fn! { fn line_len(&self) -> usize }
    buf_fn! { fn current_line_len(&self) -> usize }
    buf_fn! { fn last_indent_len(&self) -> usize }
    buf_fn! { fn is_beginning_of_line(&self) -> bool }
    buf_fn! { fn last_char(&self) -> Option<char> }
    buf_fn! { fn last_indent_group_skipped(&self) -> bool }
    buf_fn! { fn set_last_indent_group_skipped(&mut self, skip: bool) }
    buf_fn! { fn write_raw(&mut self, s: impl AsRef<str>) -> std::fmt::Result }

    fn next_chunk_needs_space(&self, next_char: char) -> bool {
        if self.is_beginning_of_line() {
            return false
        }
        let last_char =
            if let Some(last_char) = self.last_char() { last_char } else { return false };
        if last_char.is_whitespace() {
            return false
        }
        if next_char.is_whitespace() {
            return false
        }
        match last_char {
            '{' | '[' => match next_char {
                '{' | '[' | '(' => false,
                _ => self.config.bracket_spacing,
            },
            '(' => false,
            _ => match next_char {
                '}' | ']' => self.config.bracket_spacing,
                ')' => false,
                _ => true,
            },
        }
    }

    /// Write opening bracket with respect to `config.bracket_spacing` setting:
    /// `"{ "` if `true`, `"{"` if `false`
    fn write_opening_bracket(&mut self) -> std::fmt::Result {
        let space = if self.next_chunk_needs_space('{') { " " } else { "" };
        write!(self.buf(), "{space}{{")
    }

    /// Write closing bracket with respect to `config.bracket_spacing` setting:
    /// `" }"` if `true`, `"}"` if `false`
    fn write_closing_bracket(&mut self) -> std::fmt::Result {
        let bracket = if self.config.bracket_spacing {
            if self.next_chunk_needs_space('}') {
                " }"
            } else {
                "}"
            }
        } else {
            "}"
        };
        write!(self.buf(), "{bracket}")
    }

    /// Write empty brackets with respect to `config.bracket_spacing` setting:
    /// `"{ }"` if `true`, `"{}"` if `false`
    fn write_empty_brackets(&mut self) -> std::fmt::Result {
        let space = if self.next_chunk_needs_space('{') { " " } else { "" };
        let brackets = if self.config.bracket_spacing { "{ }" } else { "{}" };
        write!(self.buf(), "{space}{brackets}")
    }

    /// Write semicolon to the buffer
    fn write_semicolon(&mut self) -> std::fmt::Result {
        write!(self.buf(), ";")
    }

    /// Write whitespace separator to the buffer
    /// `"\n"` if `multiline` is `true`, `" "` if `false`
    fn write_whitespace_separator(&mut self, multiline: bool) -> std::fmt::Result {
        if !self.is_beginning_of_line() {
            write!(self.buf(), "{}", if multiline { "\n" } else { " " })?;
        }
        Ok(())
    }

    /// Transform [Visitable] items to the list of chunks
    fn items_to_chunks<T, F, V>(
        &mut self,
        items: &mut [T],
        mapper: F,
    ) -> Result<Vec<(usize, String)>, VError>
    where
        F: Fn(&mut T) -> Result<(Loc, &mut V), VError>,
        V: Visitable,
    {
        items
            .iter_mut()
            .map(|i| {
                let (loc, vis) = mapper(i)?;
                Ok((loc.end(), self.visit_to_string(vis)?))
            })
            .collect::<Result<Vec<_>, VError>>()
    }

    /// Transform [Visitable] items to the list of chunks
    fn items_to_chunks2<T, F, V>(
        &mut self,
        items: &mut [T],
        next_byte_offset: Option<usize>,
        mapper: F,
    ) -> Result<Vec<Chunk>, VError>
    where
        F: Fn(&mut T) -> Result<(Loc, &mut V), VError>,
        V: Visitable,
    {
        let mut items = items
            .iter_mut()
            .map(mapper)
            .collect::<Result<Vec<_>, VError>>()?
            .into_iter()
            .peekable();
        let mut out = Vec::new();
        while let Some((loc, item)) = items.next() {
            let chunk_next_byte_offset =
                items.peek().map(|(loc, _)| loc.start()).or(next_byte_offset);
            out.push(self.visit_to_chunk(loc.start(), item, chunk_next_byte_offset)?);
        }
        Ok(out)
    }

    /// Transform [Visitable] items to the list of chunks
    fn items_to_chunks_sorted2<T, F, V>(
        &mut self,
        items: &mut [T],
        next_byte_offset: Option<usize>,
        mapper: F,
    ) -> Result<Vec<Chunk>, VError>
    where
        F: Fn(&mut T) -> Result<(Loc, &mut V), VError>,
        V: Visitable + AttrSortKey,
    {
        let mut items = items
            .iter_mut()
            .map(|i| {
                let (loc, vis) = mapper(i)?;
                Ok((vis.attr_sort_key(), loc, vis))
            })
            .collect::<Result<Vec<_>, VError>>()?
            .into_iter()
            .peekable();
        let mut out = Vec::new();
        while let Some((attr_sort_key, loc, item)) = items.next() {
            let chunk_next_byte_offset =
                items.peek().map(|(_, loc, _)| loc.start()).or(next_byte_offset);
            out.push((
                (attr_sort_key, loc),
                self.visit_to_chunk(loc.start(), item, chunk_next_byte_offset)?,
            ));
        }
        out.sort_by_key(|(k, _)| *k);
        Ok(out.into_iter().map(|(_, c)| c).collect_vec())
    }

    /// Is length of the `text` with respect to already written line <= `config.line_length`
    fn will_it_fit(&self, text: impl AsRef<str>) -> bool {
        if text.as_ref().contains('\n') {
            return false
        }
        self.line_len() >
            self.last_indent_len()
                .saturating_add(self.current_line_len())
                .saturating_add(text.as_ref().len())
    }

    fn are_chunks_separated_multiline<'b>(
        &self,
        items: impl IntoIterator<Item = &'b (usize, impl std::fmt::Display + 'b)> + 'b,
        separator: &str,
    ) -> bool {
        !self.will_it_fit(self.simulate_chunks_separated(items, separator))
    }

    // fn write_chunks<'b>(
    //     &mut self,
    //     items: impl IntoIterator<Item = &'b (usize, impl std::fmt::Display + 'b)> + 'b,
    //     multiline: bool,
    // ) -> std::fmt::Result {
    //     self.write_chunks_separated(items, "", multiline)
    // }

    /// Write `items` separated by `separator` with respect to `config.line_length` setting
    fn write_chunks_separated<'b>(
        &mut self,
        items: impl IntoIterator<Item = &'b (usize, impl std::fmt::Display + 'b)> + 'b,
        separator: &str,
        multiline: bool,
    ) -> std::fmt::Result {
        let mut items = items.into_iter().peekable();
        while let Some((byte_end, item)) = items.next() {
            write_chunk!(self, *byte_end, "{}", item)?;

            if let Some((next_byte_end, next_chunk)) = items.peek() {
                write!(self.buf(), "{}", separator)?;
                if multiline &&
                    next_chunk
                        .to_string()
                        .chars()
                        .next()
                        .map(|ch| !ch.is_whitespace())
                        .unwrap_or(false)
                {
                    writeln_chunk!(self, *next_byte_end)?;
                } else {
                    write_chunk!(self, *next_byte_end)?;
                }
            }
        }

        Ok(())
    }

    fn write_chunks_separated2<'b>(
        &mut self,
        chunks: impl IntoIterator<Item = &'b Chunk>,
        separator: &str,
        multiline: bool,
    ) -> std::fmt::Result {
        let mut chunks = chunks.into_iter().peekable();
        while let Some(chunk) = chunks.next() {
            let mut chunk = chunk.clone();

            // handle postfixes before and add newline if necessary
            let postfixes_before = std::mem::take(&mut chunk.postfixes_before);
            for comment in postfixes_before {
                self.write_comment(&comment)?;
            }
            if multiline && !self.is_beginning_of_line() {
                writeln!(self.buf())?;
            }

            // remove postfixes so we can add separator between
            let postfixes = std::mem::take(&mut chunk.postfixes);

            self.write_chunk(&chunk)?;

            // add separator
            if chunks.peek().is_some() {
                write!(self.buf(), "{}", separator)?;
                for comment in postfixes {
                    self.write_comment(&comment)?;
                }
                if multiline && !self.is_beginning_of_line() {
                    writeln!(self.buf())?;
                }
            } else {
                for comment in postfixes {
                    self.write_comment(&comment)?;
                }
            }
        }
        Ok(())
    }

    fn visit_to_string(&mut self, visitable: &mut impl Visitable) -> Result<String, VError> {
        self.push_temp_buf();
        visitable.visit(self)?;
        let buf = self.pop_temp_buf().unwrap();
        Ok(buf.w)
    }

    /// Returns number of blank lines between two LOCs
    fn blank_lines(&self, a: Loc, b: Loc) -> usize {
        self.source[a.end()..b.start()].matches('\n').count()
    }

    fn write_comment(&mut self, comment: &CommentWithMetadata) -> std::fmt::Result {
        if comment.is_prefix() {
            let last_indent_group_skipped = self.last_indent_group_skipped();
            if !self.is_beginning_of_line() {
                writeln!(self.buf())?;
            }
            writeln!(self.buf(), "{}", comment.comment)?;
            self.set_last_indent_group_skipped(last_indent_group_skipped);
        } else {
            let indented = self.is_beginning_of_line();
            if indented {
                self.indent(1);
            } else if self.next_chunk_needs_space('/') {
                write!(self.buf(), " ")?;
            }
            if comment.is_line() {
                writeln!(self.buf(), "{}", comment.comment)?;
            } else {
                write!(self.buf(), "{}", comment.comment)?;
            }
            if indented {
                self.dedent(1);
            }
        }
        Ok(())
    }

    fn write_postfix_comments_before(&mut self, byte_end: usize) -> std::fmt::Result {
        for postfix in self.comments.remove_postfixes_before(byte_end) {
            self.write_comment(&postfix)?;
        }
        Ok(())
    }

    fn write_prefix_comments_before(&mut self, byte_end: usize) -> std::fmt::Result {
        for prefix in self.comments.remove_prefixes_before(byte_end) {
            self.write_comment(&prefix)?;
        }
        Ok(())
    }

    fn chunk_at(
        &mut self,
        byte_offset: usize,
        content: impl std::fmt::Display,
        next_byte_offset: Option<usize>,
    ) -> Chunk {
        Chunk {
            postfixes_before: self.comments.remove_postfixes_before(byte_offset),
            prefixes: self.comments.remove_prefixes_before(byte_offset),
            content: content.to_string(),
            postfixes: next_byte_offset
                .map(|byte_offset| self.comments.remove_postfixes_before(byte_offset))
                .unwrap_or_default(),
        }
    }

    fn chunked(
        &mut self,
        byte_offset: usize,
        next_byte_offset: Option<usize>,
        mut fun: impl FnMut(&mut Self) -> Result<(), VError>,
    ) -> Result<Chunk, VError> {
        let postfixes_before = self.comments.remove_postfixes_before(byte_offset);
        let prefixes = self.comments.remove_prefixes_before(byte_offset);
        self.push_temp_buf();
        fun(self)?;
        let content = self.pop_temp_buf().unwrap().w;
        let postfixes = next_byte_offset
            .map(|byte_offset| self.comments.remove_postfixes_before(byte_offset))
            .unwrap_or_default();
        Ok(Chunk { postfixes_before, prefixes, content, postfixes })
    }

    fn visit_to_chunk(
        &mut self,
        byte_offset: usize,
        visitable: &mut impl Visitable,
        next_byte_offset: Option<usize>,
    ) -> Result<Chunk, VError> {
        self.chunked(byte_offset, next_byte_offset, |fmt| visitable.visit(fmt))
    }

    fn write_chunk(&mut self, chunk: &Chunk) -> std::fmt::Result {
        // handle comments before chunk
        for comment in &chunk.postfixes_before {
            self.write_comment(comment)?;
        }
        for comment in &chunk.prefixes {
            self.write_comment(comment)?;
        }

        // trim chunk start
        let content = if chunk.content.starts_with('\n') {
            let mut chunk = chunk.content.trim_start().to_string();
            chunk.insert(0, '\n');
            chunk
        } else if chunk.content.starts_with(' ') {
            let mut chunk = chunk.content.trim_start().to_string();
            chunk.insert(0, ' ');
            chunk
        } else {
            chunk.content.clone()
        };

        if !content.is_empty() {
            // add whitespace if necessary
            if self.next_chunk_needs_space(content.chars().next().unwrap()) {
                if self.will_it_fit(format!(" {content}")) {
                    write!(self.buf(), " ")?;
                } else {
                    writeln!(self.buf())?;
                }
            }

            // write chunk
            write!(self.buf(), "{content}")?;
        }

        // write any postfix comments
        for comment in &chunk.postfixes {
            self.write_comment(comment)?;
        }

        Ok(())
    }

    fn simulate_chunk(&self, byte_end: usize, chunk: impl std::fmt::Display) -> String {
        let mut string = chunk.to_string();
        for comment in self.comments.get_comments_before(byte_end) {
            string = format!("{} {}", string, comment.comment);
        }
        string
    }

    fn simulate_chunks_separated<'b>(
        &self,
        items: impl IntoIterator<Item = &'b (usize, impl std::fmt::Display + 'b)> + 'b,
        separator: &str,
    ) -> String {
        let separator = format!("{} ", separator.trim());
        let mut string = String::new();
        let mut items = items.into_iter().peekable();
        let mut max_byte_end: usize = 0;
        while let Some((byte_end, item)) = items.next() {
            // find end location of items
            max_byte_end = usize::max(*byte_end, max_byte_end);
            let item = item.to_string();
            // create separated string
            string.push_str(&item);
            if items.peek().is_some() {
                string.push_str(&separator);
            }
        }
        self.simulate_chunk(max_byte_end, string)
    }

    fn simulate_to_string(
        &mut self,
        mut fun: impl FnMut(&mut Self) -> Result<(), VError>,
    ) -> Result<String, VError> {
        let comments = self.comments.clone();
        self.push_temp_buf();
        fun(self)?;
        let buf = self.pop_temp_buf().unwrap();
        self.comments = comments;
        Ok(buf.w)
    }

    fn are_chunks_separated_multiline2<'b>(
        &mut self,
        format_string: &str,
        items: impl IntoIterator<Item = &'b Chunk>,
        separator: &str,
    ) -> Result<bool, VError> {
        let items = items.into_iter().collect_vec();
        let chunks = self.simulate_to_string(|fmt| {
            fmt.write_chunks_separated2(items.iter().copied(), separator, false)?;
            Ok(())
        })?;
        Ok(!self.will_it_fit(format_string.replacen("{}", &chunks, 1)))
    }

    fn format_chunk(
        &mut self,
        byte_end: usize,
        chunk: impl std::fmt::Display,
    ) -> Result<String, VError> {
        self.push_temp_buf();
        write_chunk!(self, byte_end, "{chunk}")?;
        let buf = self.pop_temp_buf().unwrap();
        Ok(buf.w)
    }

    fn indented(
        &mut self,
        delta: usize,
        fun: impl FnMut(&mut Self) -> Result<(), VError>,
    ) -> Result<(), VError> {
        self.indented_if(true, delta, fun)
    }

    fn indented_if(
        &mut self,
        condition: bool,
        delta: usize,
        mut fun: impl FnMut(&mut Self) -> Result<(), VError>,
    ) -> Result<(), VError> {
        if condition {
            self.indent(delta);
        }
        fun(self)?;
        if condition {
            self.dedent(delta);
        }
        Ok(())
    }

    fn grouped(
        &mut self,
        mut fun: impl FnMut(&mut Self) -> Result<(), VError>,
    ) -> Result<(), VError> {
        self.start_group();
        fun(self)?;
        self.end_group();
        Ok(())
    }

    fn surrounded(
        &mut self,
        byte_offset: usize,
        first_chunk: impl std::fmt::Display,
        last_chunk: impl std::fmt::Display,
        next_byte_end: Option<usize>,
        mut fun: impl FnMut(&mut Self) -> Result<(), VError>,
    ) -> Result<(), VError> {
        self.write_postfix_comments_before(byte_offset)?;

        write_chunk!(self, byte_offset, "{first_chunk}")?;

        self.push_temp_buf();
        fun(self)?;
        let contents = self.pop_temp_buf().unwrap().w;

        let multiline = !self.will_it_fit(format!("{contents}{last_chunk}"));
        if multiline {
            if contents.chars().next().map(|ch| !ch.is_whitespace()).unwrap_or(false) {
                writeln!(self.buf())?;
            }
            self.indent(1);
        }

        write_chunk!(self, byte_offset, "{contents}")?;

        if let Some(next_byte_end) = next_byte_end {
            self.write_postfix_comments_before(next_byte_end)?;
        }

        let last_chunk = last_chunk.to_string();
        if multiline {
            if !self.is_beginning_of_line() && !last_chunk.trim_start().is_empty() {
                writeln!(self.buf())?;
            }
            self.dedent(1);
        }
        write_chunk!(self, byte_offset, "{last_chunk}")?;

        Ok(())
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

        let comments = self.simulate_to_string(|fmt| {
            fmt.write_postfix_comments_before(fmt.source.len())?;
            fmt.write_prefix_comments_before(fmt.source.len())?;
            Ok(())
        })?;
        self.comments.remove_comments_before(self.source.len());
        write_chunk!(self, self.source.len(), "{}", comments.trim_end())?;

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

    fn visit_contract(&mut self, contract: &mut ContractDefinition) -> VResult {
        self.context.contract = Some(contract.clone());

        self.grouped(|fmt| {
            write_chunk!(fmt, contract.loc.start(), "{}", contract.ty)?;
            write_chunk!(fmt, contract.name.loc.end(), "{}", contract.name.name)?;
            if !contract.base.is_empty() {
                write_chunk!(
                    fmt,
                    contract.name.loc.end(),
                    contract.base.first().unwrap().loc.start(),
                    "is"
                )?;
            }
            Ok(())
        })?;

        if !contract.base.is_empty() {
            self.indented(1, |fmt| {
                let base_end = contract.parts.first().map(|part| part.loc().start());
                let bases = fmt
                    .items_to_chunks2(&mut contract.base, base_end, |base| Ok((base.loc, base)))?;
                let multiline = fmt.are_chunks_separated_multiline2("{}", &bases, ",")?;
                fmt.write_chunks_separated2(&bases, ",", multiline)?;
                fmt.write_whitespace_separator(multiline)?;
                Ok(())
            })?;
        }

        if contract.parts.is_empty() {
            self.write_empty_brackets()?;
            return Ok(())
        }

        self.write_opening_bracket()?;
        writeln!(self.buf())?;

        self.indented(1, |fmt| {
            let mut contract_parts_iter = contract.parts.iter_mut().peekable();
            while let Some(part) = contract_parts_iter.next() {
                part.visit(fmt)?;
                writeln!(fmt.buf())?;

                // If source has zero blank lines between parts and the current part is not a
                // function, leave it as is. If it has one or more blank lines or
                // the current part is a function, separate parts with one blank
                // line.
                if let Some(next_part) = contract_parts_iter.peek() {
                    let blank_lines = fmt.blank_lines(part.loc(), next_part.loc());
                    let is_function = match part {
                        ContractPart::FunctionDefinition(function_definition) => matches!(
                            **function_definition,
                            FunctionDefinition {
                                ty: FunctionTy::Function |
                                    FunctionTy::Receive |
                                    FunctionTy::Fallback,
                                ..
                            }
                        ),
                        _ => false,
                    };
                    if is_function && blank_lines > 0 || blank_lines > 1 {
                        writeln!(fmt.buf())?;
                    }
                }
            }
            Ok(())
        })?;

        self.write_closing_bracket()?;

        self.context.contract = None;

        Ok(())
    }

    fn visit_pragma(&mut self, ident: &mut Identifier, str: &mut StringLiteral) -> VResult {
        #[allow(clippy::if_same_then_else)]
        let pragma_descriptor = if ident.name == "solidity" {
            // There are some issues with parsing Solidity's versions with crates like `semver`:
            // 1. Ranges like `>=0.4.21<0.6.0` or `>=0.4.21 <0.6.0` are not parseable at all.
            // 2. Versions like `0.8.10` got transformed into `^0.8.10` which is not the same.
            // TODO: semver-solidity crate :D
            &str.string
        } else {
            &str.string
        };

        write_chunk!(self, str.loc.end(), "pragma {} {};", &ident.name, pragma_descriptor)?;

        Ok(())
    }

    fn visit_import_plain(&mut self, loc: Loc, import: &mut StringLiteral) -> VResult {
        self.grouped(|fmt| {
            write_chunk!(fmt, loc.start(), import.loc.start(), "import")?;
            write_chunk!(fmt, import.loc.start(), import.loc.end(), "\"{}\"", &import.string)?;
            fmt.write_semicolon()?;
            Ok(())
        })?;
        Ok(())
    }

    fn visit_import_global(
        &mut self,
        loc: Loc,
        global: &mut StringLiteral,
        alias: &mut Identifier,
    ) -> VResult {
        self.grouped(|fmt| {
            write_chunk!(fmt, loc.start(), global.loc.start(), "import")?;
            write_chunk!(fmt, global.loc.start(), global.loc.end(), "\"{}\"", &global.string)?;
            write_chunk!(fmt, loc.start(), alias.loc.start(), "as")?;
            alias.visit(fmt)?;
            fmt.write_semicolon()?;
            Ok(())
        })?;
        Ok(())
    }

    fn visit_import_renames(
        &mut self,
        loc: Loc,
        imports: &mut [(Identifier, Option<Identifier>)],
        from: &mut StringLiteral,
    ) -> VResult {
        if imports.is_empty() {
            self.grouped(|fmt| {
                write_chunk!(fmt, loc.start(), "import")?;
                fmt.write_empty_brackets()?;
                write_chunk!(fmt, loc.start(), from.loc.start(), "from")?;
                write_chunk!(fmt, from.loc.start(), from.loc.end(), "\"{}\"", &from.string)?;
                fmt.write_semicolon()?;
                Ok(())
            })?;
            return Ok(())
        }

        let imports_start = imports.first().unwrap().0.loc.start();

        write_chunk!(self, loc.start(), imports_start, "import")?;

        self.surrounded(imports_start, "{", "}", Some(from.loc.start()), |fmt| {
            let mut imports = imports.iter_mut().peekable();
            let mut import_chunks = Vec::new();
            while let Some((ident, alias)) = imports.next() {
                import_chunks.push(fmt.chunked(
                    ident.loc.start(),
                    imports.peek().map(|(ident, _)| ident.loc.start()),
                    |fmt| {
                        fmt.grouped(|fmt| {
                            ident.visit(fmt)?;
                            if let Some(alias) = alias {
                                write_chunk!(fmt, ident.loc.end(), alias.loc.start(), "as")?;
                                alias.visit(fmt)?;
                            }
                            Ok(())
                        })
                    },
                )?);
            }

            let multiline = fmt.are_chunks_separated_multiline2(
                &format!("{{}} }} from \"{}\";", from.string),
                &import_chunks,
                ",",
            )?;
            fmt.write_chunks_separated2(&import_chunks, ",", multiline)?;
            Ok(())
        })?;

        self.grouped(|fmt| {
            write_chunk!(fmt, imports_start, from.loc.start(), "from")?;
            write_chunk!(fmt, from.loc.start(), from.loc.end(), "\"{}\"", &from.string)?;
            fmt.write_semicolon()?;
            Ok(())
        })?;

        Ok(())
    }

    fn visit_enum(&mut self, enumeration: &mut EnumDefinition) -> VResult {
        let mut name = self.visit_to_chunk(
            enumeration.name.loc.start(),
            &mut enumeration.name,
            Some(enumeration.loc.end()),
        )?;
        name.content = format!("enum {}", name.content);
        self.write_chunk(&name)?;

        if enumeration.values.is_empty() {
            self.write_empty_brackets()?;
        } else {
            self.surrounded(
                enumeration.values.first().unwrap().loc.start(),
                "{",
                "}",
                Some(enumeration.loc.end()),
                |fmt| {
                    let values = fmt.items_to_chunks2(
                        &mut enumeration.values,
                        Some(enumeration.loc.end()),
                        |ident| Ok((ident.loc, ident)),
                    )?;
                    fmt.write_chunks_separated2(&values, ",", true)?;
                    Ok(())
                },
            )?;
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
        self.write_semicolon()?;

        Ok(())
    }

    fn visit_var_declaration(&mut self, var: &mut VariableDeclaration) -> VResult {
        var.ty.visit(self)?;

        if let Some(storage) = &var.storage {
            write_chunk!(self, storage.loc().end(), "{}", storage)?;
        }

        write_chunk!(self, var.name.loc.end(), "{}", var.name.name)?;

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

        self.write_postfix_comments_before(func.loc.start())?;
        self.write_prefix_comments_before(func.loc.start())?;

        let func_name = if let Some(ident) = &func.name {
            format!("{} {}", func.ty, ident.name)
        } else {
            func.ty.to_string()
        };

        // calculate locations of chunk groups
        let attrs_loc = func.attributes.first().map(|attr| attr.loc());
        let returns_loc = func.returns.first().map(|param| param.0);
        let body_loc = func.body.as_ref().map(LineOfCode::loc);

        // get function parameter chunks
        let params_end = attrs_loc
            .as_ref()
            .or(returns_loc.as_ref())
            .or(body_loc.as_ref())
            .map(|loc| loc.start());
        let params = self.items_to_chunks2(&mut func.params, params_end, |(loc, param)| {
            Ok((*loc, param.as_mut().unwrap()))
        })?;

        // get attribute chunks
        let attrs_end = returns_loc.as_ref().or(body_loc.as_ref()).map(|loc| loc.start());
        let attributes = self.items_to_chunks_sorted2(&mut func.attributes, attrs_end, |attr| {
            Ok((attr.loc(), attr))
        })?;

        // get returns parameter chunks
        let returns_end = body_loc.as_ref().map(|loc| loc.start());
        let returns = self.items_to_chunks2(&mut func.returns, returns_end, |(loc, param)| {
            Ok((*loc, param.as_mut().unwrap()))
        })?;

        // check if the parameters need to be multiline
        let simulated_func_def = self.simulate_to_string(|fmt| {
            write!(fmt.buf(), "{func_name}(")?;
            fmt.write_chunks_separated2(&params, ",", false)?;
            write!(fmt.buf(), ")")?;
            Ok(())
        })?;
        let params_multiline = !self.will_it_fit(&simulated_func_def);

        // check if the attributes need to be multiline
        let attrs_multiline = if params_multiline {
            true
        } else {
            let simulated_func_def_attrs = self.simulate_to_string(|fmt| {
                if !attributes.is_empty() {
                    write!(fmt.buf(), " ")?;
                    fmt.write_chunks_separated2(&attributes, "", false)?;
                }
                if !returns.is_empty() {
                    write!(fmt.buf(), " returns(")?;
                    fmt.write_chunks_separated2(&returns, "", false)?;
                    write!(fmt.buf(), ")")?;
                }
                write!(fmt.buf(), "{}", if func.body.is_some() { " {" } else { ";" })?;
                Ok(())
            })?;
            !self.will_it_fit(format!("{simulated_func_def}{simulated_func_def_attrs}"))
        };

        // write parameters
        self.surrounded(func.loc.start(), format!("{func_name}("), ")", params_end, |fmt| {
            fmt.write_chunks_separated2(&params, ",", params_multiline)?;
            Ok(())
        })?;

        // write attributes
        if !func.attributes.is_empty() {
            let byte_offset = attrs_loc.unwrap().start();
            self.write_postfix_comments_before(byte_offset)?;
            self.write_whitespace_separator(attrs_multiline)?;
            self.indented(1, |fmt| {
                fmt.write_chunks_separated2(&attributes, "", attrs_multiline)?;
                Ok(())
            })?;
        }

        // write returns
        if !func.returns.is_empty() {
            let byte_offset = returns_loc.unwrap().start();
            self.write_postfix_comments_before(byte_offset)?;
            self.write_whitespace_separator(attrs_multiline)?;
            self.indented(1, |fmt| {
                let returns_multiline = attrs_multiline &&
                    fmt.are_chunks_separated_multiline2("returns ({})", &returns, ",")?;
                fmt.surrounded(byte_offset, "returns (", ")", returns_end, |fmt| {
                    fmt.write_chunks_separated2(&returns, ",", returns_multiline)?;
                    Ok(())
                })?;
                Ok(())
            })?;
        }

        // write function body
        match &mut func.body {
            Some(body) => {
                let byte_offset = body_loc.unwrap().start();
                let formatted_body = self.visit_to_string(body)?;
                if attrs_multiline && !(func.attributes.is_empty() && func.returns.is_empty()) {
                    writeln_chunk!(self, byte_offset)?;
                } else {
                    write_chunk!(self, byte_offset, " ")?;
                }
                write!(self.buf(), "{formatted_body}")?;
            }
            None => self.write_semicolon()?,
        }

        self.context.function = None;

        Ok(())
    }

    fn visit_function_attribute(&mut self, attribute: &mut FunctionAttribute) -> VResult {
        match attribute {
            FunctionAttribute::Mutability(mutability) => {
                write_chunk!(self, mutability.loc().end(), "{mutability}")?
            }
            FunctionAttribute::Visibility(visibility) => {
                if let Some(loc) = visibility.loc() {
                    write_chunk!(self, loc.end(), "{visibility}")?
                } else {
                    write!(self.buf(), "{visibility}")?
                }
            }
            FunctionAttribute::Virtual(loc) => write_chunk!(self, loc.end(), "virtual")?,
            FunctionAttribute::Immutable(loc) => write_chunk!(self, loc.end(), "immutable")?,
            FunctionAttribute::Override(loc, args) => {
                write!(self.buf(), "override")?;
                if !args.is_empty() {
                    let args =
                        args.iter().map(|arg| (arg.loc.end(), &arg.name)).collect::<Vec<_>>();
                    let multiline = self.are_chunks_separated_multiline(&args, ", ");
                    self.surrounded(loc.start(), "(", ")", Some(loc.end()), |fmt| {
                        fmt.write_chunks_separated(&args, ",", multiline)?;
                        Ok(())
                    })?;
                }
            }
            FunctionAttribute::BaseOrModifier(loc, base) => {
                let is_contract_base = self.context.contract.as_ref().map_or(false, |contract| {
                    contract.base.iter().any(|contract_base| {
                        helpers::namespace_matches(&contract_base.name, &base.name)
                    })
                });

                if is_contract_base {
                    base.visit(self)?;
                } else {
                    let base_or_modifier = self.visit_to_string(base)?;
                    let base_or_modifier =
                        base_or_modifier.strip_suffix("()").unwrap_or(&base_or_modifier);
                    write_chunk!(self, loc.end(), "{base_or_modifier}")?;
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
            let args = self.items_to_chunks(args, |arg| Ok((arg.loc(), arg)))?;

            let multiline = self.are_chunks_separated_multiline(&args, ", ");

            if multiline {
                writeln!(self.buf())?;
            }

            self.indented_if(multiline, 1, |fmt| {
                fmt.write_chunks_separated(&args, ",", multiline)?;
                Ok(())
            })?;

            if multiline {
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
            write_chunk!(self, storage.loc().end(), "{storage}")?;
        }

        if let Some(name) = &parameter.name {
            write_chunk!(self, parameter.loc.end(), "{}", name.name)?;
        }

        Ok(())
    }

    fn visit_struct(&mut self, structure: &mut StructDefinition) -> VResult {
        write_chunk!(self, structure.name.loc.end(), "struct {} ", &structure.name.name)?;

        if structure.fields.is_empty() {
            self.write_empty_brackets()?;
        } else {
            writeln!(self.buf(), "{{")?;

            self.indented(1, |fmt| {
                for field in structure.fields.iter_mut() {
                    field.visit(fmt)?;
                    writeln!(fmt.buf(), ";")?;
                }
                Ok(())
            })?;

            write!(self.buf(), "}}")?;
        }

        Ok(())
    }

    fn visit_type_definition(&mut self, def: &mut TypeDefinition) -> VResult {
        write_chunk!(self, def.name.loc.end(), "type {} is ", def.name.name)?;
        def.ty.visit(self)?;
        self.write_semicolon()?;

        Ok(())
    }

    fn visit_stray_semicolon(&mut self) -> VResult {
        self.write_semicolon()?;

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
        } else {
            self.write_opening_bracket()?;
        }

        self.indented_if(multiline, 1, |fmt| {
            let mut statements_iter = statements.iter_mut().peekable();
            while let Some(stmt) = statements_iter.next() {
                stmt.visit(fmt)?;
                if multiline {
                    writeln!(fmt.buf())?;
                }

                // If source has zero blank lines between statements, leave it as is. If one
                //  or more, separate statements with one blank line.
                if let Some(next_stmt) = statements_iter.peek() {
                    if fmt.blank_lines(LineOfCode::loc(stmt), LineOfCode::loc(next_stmt)) > 1 {
                        writeln!(fmt.buf())?;
                    }
                }
            }
            Ok(())
        })?;

        if multiline {
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
        let event_name = format_chunk!(self, event.name.loc.end(), "event {}", event.name.name)?;

        let params = self.items_to_chunks(&mut event.fields, |param| Ok((param.loc, param)))?;

        let formatted = format!(
            "{event_name}({}){};",
            params.iter().map(|p| p.1.to_owned()).join(", "),
            if event.anonymous { " anonymous" } else { "" }
        );
        let multiline = !self.will_it_fit(formatted);

        self.surrounded(event.name.loc.end(), format!("{event_name}("), ")", None, |fmt| {
            fmt.write_chunks_separated(&params, ",", multiline)?;
            Ok(())
        })?;

        if event.anonymous {
            write_chunk!(self, event.loc.end(), "anonymous")?;
        }

        self.write_semicolon()?;

        Ok(())
    }

    fn visit_event_parameter(&mut self, param: &mut EventParameter) -> VResult {
        param.ty.visit(self)?;

        if param.indexed {
            write_chunk!(self, param.loc.start(), "indexed")?;
        }
        if let Some(name) = &param.name {
            write_chunk!(self, name.loc.end(), "{}", name.name)?;
        }

        Ok(())
    }

    fn visit_error(&mut self, error: &mut ErrorDefinition) -> VResult {
        let error_name = format_chunk!(self, error.name.loc.end(), "error {}", error.name.name)?;

        let params = self.items_to_chunks(&mut error.fields, |param| Ok((param.loc, param)))?;

        let formatted =
            format!("{error_name}({});", params.iter().map(|p| p.1.to_owned()).join(", "),);
        let multiline = !self.will_it_fit(formatted);

        self.surrounded(error.name.loc.end(), format!("{error_name}("), ")", None, |fmt| {
            fmt.write_chunks_separated(&params, ",", multiline)?;
            Ok(())
        })?;
        self.write_semicolon()?;

        Ok(())
    }

    fn visit_error_parameter(&mut self, param: &mut ErrorParameter) -> VResult {
        param.ty.visit(self)?;

        if let Some(name) = &param.name {
            write_chunk!(self, name.loc.end(), "{}", name.name)?;
        }

        Ok(())
    }

    fn visit_using(&mut self, using: &mut Using) -> VResult {
        write_chunk!(self, using.loc.start(), "using")?;

        match &mut using.list {
            UsingList::Library(library) => {
                self.visit_expr(LineOfCode::loc(library), library)?;
            }
            UsingList::Functions(funcs) => {
                let func_strs = self.items_to_chunks(funcs, |func| Ok((func.loc(), func)))?;
                let multiline = self.are_chunks_separated_multiline(func_strs.iter(), ", ");
                self.write_opening_bracket()?;
                self.write_chunks_separated(&func_strs, ",", multiline)?;
                self.write_closing_bracket()?;
            }
        }

        write_chunk!(self, using.loc.start(), "for")?;

        if let Some(ty) = &mut using.ty {
            ty.visit(self)?;
        } else {
            write_chunk!(self, using.loc.start(), "*")?;
        }

        if let Some(global) = &mut using.global {
            write_chunk!(self, global.loc.end(), "{}", global.name)?;
        }

        self.write_semicolon()?;

        Ok(())
    }

    fn visit_var_definition(&mut self, var: &mut VariableDefinition) -> VResult {
        self.grouped(|fmt| {
            var.ty.visit(fmt)?;

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

            if !var.attrs.is_empty() {
                let multiline = fmt.are_chunks_separated_multiline(&attributes, " ");
                if multiline {
                    writeln!(fmt.buf())?;
                }
                fmt.write_chunks_separated(&attributes, "", multiline)?;
            }

            if var.initializer.is_some() {
                write_chunk!(fmt, var.name.loc.end(), "{} =", var.name.name)?;
            } else {
                var.name.visit(fmt)?;
            }

            Ok(())
        })?;

        if let Some(init) = &mut var.initializer {
            // TODO check if this should actually be indented or not. Function calls, member access
            // and things may not need to be indented
            self.indented(1, |fmt| {
                init.visit(fmt)?;
                Ok(())
            })?;
        }

        self.write_semicolon()?;

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

        println!("{}", source_formatted);
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
    test_directory! { FunctionDefinitionWithComments }
}
