use super::*;

use std::{
    collections::{hash_map::Entry, HashMap},
    fs,
    path::{Path, PathBuf},
};

/// A trait implemented by [`Source`] caches.
pub trait Cache<Id: ?Sized> {
    /// The type used to store the string data for this cache.
    ///
    /// Alternative types other than String can be used, but at the moment, the storage must be
    /// contiguous. A primary use case for this is to use a reference-counted string instead of
    /// copying the whole contents into a [`Source`].
    type Storage: AsRef<str>;

    /// Fetch the [`Source`] identified by the given ID, if possible.
    // TODO: Don't box
    fn fetch(&mut self, id: &Id) -> Result<&Source<Self::Storage>, Box<dyn fmt::Debug + '_>>;

    /// Display the given ID. as a single inline value.
    ///
    /// This function may make use of attributes from the [`Fmt`] trait.
    // TODO: Don't box
    fn display<'a>(&self, id: &'a Id) -> Option<Box<dyn fmt::Display + 'a>>;
}

impl<'b, C: Cache<Id>, Id: ?Sized> Cache<Id> for &'b mut C {
    type Storage = C::Storage;

    fn fetch(&mut self, id: &Id) -> Result<&Source<Self::Storage>, Box<dyn fmt::Debug + '_>> {
        C::fetch(self, id)
    }
    fn display<'a>(&self, id: &'a Id) -> Option<Box<dyn fmt::Display + 'a>> {
        C::display(self, id)
    }
}

impl<C: Cache<Id>, Id: ?Sized> Cache<Id> for Box<C> {
    type Storage = C::Storage;

    fn fetch(&mut self, id: &Id) -> Result<&Source<Self::Storage>, Box<dyn fmt::Debug + '_>> {
        C::fetch(self, id)
    }
    fn display<'a>(&self, id: &'a Id) -> Option<Box<dyn fmt::Display + 'a>> {
        C::display(self, id)
    }
}

/// A type representing a single line of a [`Source`].
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct Line {
    offset: usize,
    char_len: usize,
    byte_offset: usize,
    byte_len: usize,
}

impl Line {
    /// Get the offset of this line in the original [`Source`] (i.e: the number of characters that precede it).
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Get the character length of this line.
    pub fn len(&self) -> usize {
        self.char_len
    }

    /// Returns `true` if this line contains no characters.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the offset span of this line in the original [`Source`].
    pub fn span(&self) -> Range<usize> {
        self.offset..self.offset + self.char_len
    }

    /// Get the byte offset span of this line in the original [`Source`]. This can be used to
    /// directly slice into its source text.
    fn byte_span(&self) -> Range<usize> {
        self.byte_offset..self.byte_offset + self.byte_len
    }
}

/// A type representing a single source that may be referred to by [`Span`]s.
///
/// In most cases, a source is a single input file.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Source<I: AsRef<str> = String> {
    text: I,
    lines: Vec<Line>,
    len: usize,
    byte_len: usize,
    display_line_offset: usize,
}

impl<I: AsRef<str>> Source<I> {
    /// Get the full text of this source file.
    pub fn text(&self) -> &str {
        self.text.as_ref()
    }
}

impl<I: AsRef<str>> From<I> for Source<I> {
    /// Generate a [`Source`] from the given [`str`].
    ///
    /// Note that this function can be expensive for long strings. Use an implementor of [`Cache`] where possible.
    fn from(input: I) -> Self {
        // `input.split_inclusive()` will not iterate at all,
        // but an empty input still ought to count as a single empty line.
        if input.as_ref().is_empty() {
            return Self {
                text: input,
                lines: vec![Line {
                    offset: 0,
                    char_len: 0,
                    byte_offset: 0,
                    byte_len: 0,
                }],
                len: 0,
                byte_len: 0,
                display_line_offset: 0,
            };
        }

        let mut char_offset = 0;
        let mut byte_offset = 0;
        let mut lines = Vec::new();

        const SEPARATORS: [char; 7] = [
            '\r',       // Carriage return
            '\n',       // Line feed
            '\x0B',     // Vertical tab
            '\x0C',     // Form feed
            '\u{0085}', // Next line
            '\u{2028}', // Line separator
            '\u{2029}', // Paragraph separator
        ];
        let mut remaining = input.as_ref().split_inclusive(SEPARATORS).peekable();
        while let Some(line) = remaining.next() {
            let mut byte_len = line.len();
            let mut char_len = line.chars().count();
            // Handle CRLF as a single terminator.
            if line.ends_with('\r') && remaining.next_if_eq(&"\n").is_some() {
                byte_len += 1;
                char_len += 1;
            }
            lines.push(Line {
                offset: char_offset,
                char_len,
                byte_offset,
                byte_len,
            });

            char_offset += char_len;
            byte_offset += byte_len;
        }

        Self {
            text: input,
            lines,
            len: char_offset,
            byte_len: byte_offset,
            display_line_offset: 0,
        }
    }
}

impl<I: AsRef<str>> Source<I> {
    /// Add an offset to the printed line numbers
    pub fn with_display_line_offset(mut self, offset: usize) -> Self {
        self.display_line_offset = offset;
        self
    }

    /// Get the offset added to printed line numbers
    pub fn display_line_offset(&self) -> usize {
        self.display_line_offset
    }

    /// Get the length of the total number of characters in the source.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if this source contains no characters.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return an iterator over the characters in the source.
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.text.as_ref().chars()
    }

    /// Get access to a specific, zero-indexed [`Line`].
    pub fn line(&self, idx: usize) -> Option<Line> {
        self.lines.get(idx).copied()
    }

    /// Return an iterator over the [`Line`]s in this source.
    pub fn lines(&self) -> impl ExactSizeIterator<Item = Line> + '_ {
        self.lines.iter().copied()
    }

    /// Get the line that the given offset appears on, and the line/column numbers of the offset.
    ///
    /// Note that the line/column numbers are zero-indexed.
    pub fn get_offset_line(&self, offset: usize) -> Option<(Line, usize, usize)> {
        if offset <= self.len {
            let idx = self
                .lines
                .binary_search_by_key(&offset, |line| line.offset)
                .unwrap_or_else(|idx| idx.saturating_sub(1));
            let line = self.line(idx)?;
            assert!(
                offset >= line.offset,
                "offset = {}, line.offset = {}",
                offset,
                line.offset
            );
            Some((line, idx, offset - line.offset))
        } else {
            None
        }
    }

    /// Get the line that the given byte offset appears on, and the line/byte column numbers of the offset.
    ///
    /// Note that the line/column numbers are zero-indexed.
    pub fn get_byte_line(&self, byte_offset: usize) -> Option<(Line, usize, usize)> {
        if byte_offset <= self.byte_len {
            let idx = self
                .lines
                .binary_search_by_key(&byte_offset, |line| line.byte_offset)
                .unwrap_or_else(|idx| idx.saturating_sub(1));
            let line = self.line(idx)?;
            assert!(
                byte_offset >= line.byte_offset,
                "byte_offset = {}, line.byte_offset = {}",
                byte_offset,
                line.byte_offset
            );
            Some((line, idx, byte_offset - line.byte_offset))
        } else {
            None
        }
    }

    /// Get the range of lines that this span runs across.
    ///
    /// The resulting range is guaranteed to contain valid line indices (i.e: those that can be used for
    /// [`Source::line`]).
    pub fn get_line_range<S: Span>(&self, span: &S) -> Range<usize> {
        let start = self.get_offset_line(span.start()).map_or(0, |(_, l, _)| l);
        let end = self
            .get_offset_line(span.end().saturating_sub(1).max(span.start()))
            .map_or(self.lines.len(), |(_, l, _)| l + 1);
        start..end
    }

    /// Get the source text for a line, includes trailing whitespace and the newline
    pub fn get_line_text(&self, line: Line) -> Option<&'_ str> {
        self.text.as_ref().get(line.byte_span())
    }
}

impl<I: AsRef<str>> Cache<()> for Source<I> {
    type Storage = I;

    fn fetch(&mut self, _: &()) -> Result<&Source<I>, Box<dyn fmt::Debug + '_>> {
        Ok(self)
    }
    fn display(&self, _: &()) -> Option<Box<dyn fmt::Display>> {
        None
    }
}

impl<I: AsRef<str>, Id: fmt::Display + Eq> Cache<Id> for (Id, Source<I>) {
    type Storage = I;

    fn fetch(&mut self, id: &Id) -> Result<&Source<I>, Box<dyn fmt::Debug + '_>> {
        if id == &self.0 {
            Ok(&self.1)
        } else {
            Err(Box::new(format!("Failed to fetch source '{}'", id)))
        }
    }
    fn display<'a>(&self, id: &'a Id) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new(id))
    }
}

/// A [`Cache`] that fetches [`Source`]s from the filesystem.
#[derive(Default, Debug, Clone)]
pub struct FileCache {
    files: HashMap<PathBuf, Source>,
}

impl Cache<Path> for FileCache {
    type Storage = String;

    fn fetch(&mut self, path: &Path) -> Result<&Source, Box<dyn fmt::Debug + '_>> {
        Ok(match self.files.entry(path.to_path_buf()) {
            // TODO: Don't allocate here
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(Source::from(
                fs::read_to_string(path).map_err(|e| Box::new(e) as _)?,
            )),
        })
    }
    fn display<'a>(&self, path: &'a Path) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new(path.display()))
    }
}

/// A [`Cache`] that fetches [`Source`]s using the provided function.
#[derive(Debug, Clone)]
pub struct FnCache<Id, F, I>
where
    I: AsRef<str>,
{
    sources: HashMap<Id, Source<I>>,
    get: F,
}

impl<Id, F, I> FnCache<Id, F, I>
where
    I: AsRef<str>,
{
    /// Create a new [`FnCache`] with the given fetch function.
    pub fn new(get: F) -> Self {
        Self {
            sources: HashMap::default(),
            get,
        }
    }

    /// Pre-insert a selection of [`Source`]s into this cache.
    pub fn with_sources(mut self, sources: HashMap<Id, Source<I>>) -> Self
    where
        Id: Eq + Hash,
    {
        self.sources.reserve(sources.len());
        for (id, src) in sources {
            self.sources.insert(id, src);
        }
        self
    }
}

impl<Id: fmt::Display + Hash + PartialEq + Eq + Clone, F, I> Cache<Id> for FnCache<Id, F, I>
where
    I: AsRef<str>,
    F: for<'a> FnMut(&'a Id) -> Result<I, Box<dyn fmt::Debug>>,
{
    type Storage = I;

    fn fetch(&mut self, id: &Id) -> Result<&Source<I>, Box<dyn fmt::Debug + '_>> {
        Ok(match self.sources.entry(id.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(Source::from((self.get)(id)?)),
        })
    }
    fn display<'a>(&self, id: &'a Id) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new(id))
    }
}

/// Create a [`Cache`] from a collection of ID/strings, where each corresponds to a [`Source`].
pub fn sources<Id, S, I>(iter: I) -> impl Cache<Id>
where
    Id: fmt::Display + Hash + PartialEq + Eq + Clone + 'static,
    I: IntoIterator<Item = (Id, S)>,
    S: AsRef<str>,
{
    FnCache::new(
        (move |id| Err(Box::new(format!("Failed to fetch source '{}'", id)) as _)) as fn(&_) -> _,
    )
    .with_sources(
        iter.into_iter()
            .map(|(id, s)| (id, Source::from(s)))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use std::iter::zip;
    use std::sync::Arc;

    use super::Source;

    fn test_with_lines(lines: Vec<&str>) {
        let source: String = lines.iter().copied().collect();
        let source = Source::from(source);

        assert_eq!(source.lines.len(), lines.len());

        let mut offset = 0;
        for (source_line, raw_line) in zip(source.lines.iter().copied(), lines.into_iter()) {
            assert_eq!(source_line.offset, offset);
            assert_eq!(source_line.char_len, raw_line.chars().count());
            assert_eq!(source.get_line_text(source_line).unwrap(), raw_line);
            offset += source_line.char_len;
        }

        assert_eq!(source.len, offset);
    }

    #[test]
    fn source_from_empty() {
        test_with_lines(vec![""]); // Empty string
    }

    #[test]
    fn source_from_single() {
        test_with_lines(vec!["Single line"]);
        test_with_lines(vec!["Single line with LF\n"]);
        test_with_lines(vec!["Single line with CRLF\r\n"]);
    }

    #[test]
    fn source_from_multi() {
        test_with_lines(vec!["Two\r\n", "lines\n"]);
        test_with_lines(vec!["Some\n", "more\r\n", "lines"]);
        test_with_lines(vec!["\n", "\r\n", "\n", "Empty Lines"]);
    }

    #[test]
    fn source_from_trims_trailing_spaces() {
        test_with_lines(vec!["Trailing spaces  \n", "are trimmed\t"]);
    }

    #[test]
    fn source_from_alternate_line_endings() {
        // Line endings other than LF or CRLF
        test_with_lines(vec![
            "CR\r",
            "VT\x0B",
            "FF\x0C",
            "NEL\u{0085}",
            "LS\u{2028}",
            "PS\u{2029}",
        ]);
    }

    #[test]
    fn source_from_other_string_types() {
        let raw = r#"A raw string
            with multiple
            lines behind
            an Arc"#;
        let arc = Arc::from(raw);
        let source = Source::from(arc);

        assert_eq!(source.lines.len(), 4);

        let mut offset = 0;
        for (source_line, raw_line) in zip(source.lines.iter().copied(), raw.split_inclusive('\n'))
        {
            assert_eq!(source_line.offset, offset);
            assert_eq!(source_line.char_len, raw_line.chars().count());
            assert_eq!(source.get_line_text(source_line).unwrap(), raw_line);
            offset += source_line.char_len;
        }

        assert_eq!(source.len, offset);
    }

    #[test]
    fn source_from_reference() {
        let raw = r#"A raw string
            with multiple
            lines"#;

        fn non_owning_source(input: &str) -> Source<&str> {
            Source::from(input)
        }

        let source = non_owning_source(raw);
        assert_eq!(source.lines.len(), 3);
    }
}
