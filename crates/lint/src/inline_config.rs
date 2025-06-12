use solar_parse::{ast::Span, lexer::token::RawTokenKind};
use std::{fmt, str::FromStr};

/// An inline config item
#[derive(Clone, Copy, Debug)]
pub enum InlineConfigItem {
    /// Disables the next code item regardless of newlines
    DisableNextItem,
    /// Disables formatting on the current line
    DisableLine,
    /// Disables formatting between the next newline and the newline after
    DisableNextLine,
    /// Disables formatting for any code that follows this and before the next "disable-end"
    DisableStart,
    /// Disables formatting for any code that precedes this and after the previous "disable-start"
    DisableEnd,
}

impl FromStr for InlineConfigItem {
    type Err = InvalidInlineConfigItem;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "disable-next-item" => Self::DisableNextItem,
            "disable-line" => Self::DisableLine,
            "disable-next-line" => Self::DisableNextLine,
            "disable-start" => Self::DisableStart,
            "disable-end" => Self::DisableEnd,
            s => return Err(InvalidInlineConfigItem(s.into())),
        })
    }
}

#[derive(Debug)]
pub struct InvalidInlineConfigItem(String);

impl fmt::Display for InvalidInlineConfigItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid inline config item: {}", self.0)
    }
}

/// A disabled formatting range. `loose` designates that the range includes any loc which
/// may start in between start and end, whereas the strict version requires that
/// `range.start >= loc.start <=> loc.end <= range.end`
#[derive(Debug)]
struct DisabledRange {
    start: usize,
    end: usize,
    loose: bool,
}

impl DisabledRange {
    fn includes(&self, range: std::ops::Range<usize>) -> bool {
        range.start >= self.start && (if self.loose { range.start } else { range.end } <= self.end)
    }
}

/// An inline config. Keeps track of ranges which should not be formatted.
#[derive(Debug, Default)]
pub struct InlineConfig {
    disabled_ranges: Vec<DisabledRange>,
}

impl InlineConfig {
    /// Build a new inline config with an iterator of inline config items and their locations in a
    /// source file.
    ///
    /// # Panics
    ///
    /// Panics if `items` is not sorted in ascending order of [`Span`]s.
    pub fn new(items: impl IntoIterator<Item = (Span, InlineConfigItem)>, src: &str) -> Self {
        let mut disabled_ranges = vec![];
        let mut disabled_range_start = None;
        let mut disabled_depth = 0usize;
        let mut prev_sp = Span::DUMMY;
        for (sp, item) in items {
            if cfg!(debug_assertions) {
                assert!(sp >= prev_sp, "InlineConfig::new: unsorted items: {sp:?} < {prev_sp:?}");
                prev_sp = sp;
            }

            match item {
                InlineConfigItem::DisableNextItem => {
                    use RawTokenKind::*;
                    let offset = sp.hi().to_usize();
                    let mut idx = offset;
                    let mut tokens = solar_parse::Cursor::new(&src[offset..])
                        .map(|token| {
                            let start = idx;
                            idx += token.len as usize;
                            (start, token)
                        })
                        .filter(|(_, t)| {
                            !matches!(t.kind, LineComment { .. } | BlockComment { .. })
                        })
                        .skip_while(|(_, t)| matches!(t.kind, Whitespace));
                    if let Some((mut start, _)) = tokens.next() {
                        start += offset;
                        let end = tokens
                            .find(|(_, t)| !matches!(t.kind, Whitespace))
                            .map(|(idx, _)| idx)
                            .unwrap_or(src.len());
                        disabled_ranges.push(DisabledRange { start, end, loose: true });
                    }
                }
                InlineConfigItem::DisableLine => {
                    let mut prev_newline = src[..sp.lo().to_usize()]
                        .char_indices()
                        .rev()
                        .skip_while(|(_, ch)| *ch != '\n');
                    let start = prev_newline.next().map(|(idx, _)| idx).unwrap_or_default();

                    let end_offset = sp.hi().to_usize();
                    let mut next_newline =
                        src[end_offset..].char_indices().skip_while(|(_, ch)| *ch != '\n');
                    let end =
                        end_offset + next_newline.next().map(|(idx, _)| idx).unwrap_or_default();

                    disabled_ranges.push(DisabledRange { start, end, loose: false });
                }
                InlineConfigItem::DisableNextLine => {
                    let offset = sp.hi().to_usize();
                    let mut char_indices =
                        src[offset..].char_indices().skip_while(|(_, ch)| *ch != '\n').skip(1);
                    if let Some((mut start, _)) = char_indices.next() {
                        start += offset;
                        let end = char_indices
                            .find(|(_, ch)| *ch == '\n')
                            .map(|(idx, _)| offset + idx + 1)
                            .unwrap_or(src.len());
                        disabled_ranges.push(DisabledRange { start, end, loose: false });
                    }
                }
                InlineConfigItem::DisableStart => {
                    if disabled_depth == 0 {
                        disabled_range_start = Some(sp.hi().to_usize());
                    }
                    disabled_depth += 1;
                }
                InlineConfigItem::DisableEnd => {
                    disabled_depth = disabled_depth.saturating_sub(1);
                    if disabled_depth == 0 {
                        if let Some(start) = disabled_range_start.take() {
                            disabled_ranges.push(DisabledRange {
                                start,
                                end: sp.lo().to_usize(),
                                loose: false,
                            })
                        }
                    }
                }
            }
        }
        if let Some(start) = disabled_range_start.take() {
            disabled_ranges.push(DisabledRange { start, end: src.len(), loose: false })
        }
        Self { disabled_ranges }
    }

    /// Check if the location is in a disabled range
    pub fn is_disabled(&self, span: Span) -> bool {
        self.disabled_ranges.iter().any(|range| range.includes(span.to_range()))
    }
}
