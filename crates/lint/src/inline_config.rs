use solar_parse::{ast::Span, lexer::token::RawTokenKind};
use std::{collections::HashMap, fmt, str::FromStr};

/// An inline config item
#[derive(Clone, Debug)]
pub enum InlineConfigItem {
    /// Disables the next code item regardless of newlines
    DisableNextItem(String),
    /// Disables formatting on the current line
    DisableLine(String),
    /// Disables formatting between the next newline and the newline after
    DisableNextLine(String),
    /// Disables formatting for any code that follows this and before the next "disable-end"
    DisableStart(String),
    /// Disables formatting for any code that precedes this and after the previous "disable-start"
    DisableEnd(String),
}

impl FromStr for InlineConfigItem {
    type Err = InvalidInlineConfigItem;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (disable, relevant) = s.split_once('(').unwrap_or((s, ""));
        let lint = if relevant.is_empty() || relevant == "all)" {
            "all".to_string()
        } else {
            match relevant.split_once(')') {
                Some((lint, _)) => lint.to_string(),
                None => return Err(InvalidInlineConfigItem(s.into())),
            }
        };

        let res = match disable {
            "disable-next-item" => Self::DisableNextItem(lint),
            "disable-line" => Self::DisableLine(lint),
            "disable-next-line" => Self::DisableNextLine(lint),
            "disable-start" => Self::DisableStart(lint),
            "disable-end" => Self::DisableEnd(lint),
            s => return Err(InvalidInlineConfigItem(s.into())),
        };

        Ok(res)
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
#[derive(Debug, Clone, Copy)]
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
    disabled_ranges: HashMap<String, Vec<DisabledRange>>,
}

impl InlineConfig {
    /// Build a new inline config with an iterator of inline config items and their locations in a
    /// source file.
    ///
    /// # Panics
    ///
    /// Panics if `items` is not sorted in ascending order of [`Span`]s.
    pub fn new(items: impl IntoIterator<Item = (Span, InlineConfigItem)>, src: &str) -> Self {
        let mut disabled_ranges: HashMap<String, Vec<DisabledRange>> = HashMap::new();
        let mut disabled_blocks: HashMap<String, (usize, usize)> = HashMap::new();
        let mut prev_sp = Span::DUMMY;
        for (sp, item) in items {
            if cfg!(debug_assertions) {
                assert!(sp >= prev_sp, "InlineConfig::new: unsorted items: {sp:?} < {prev_sp:?}");
                prev_sp = sp;
            }

            match item {
                InlineConfigItem::DisableNextItem(lint) => {
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
                        let range = DisabledRange { start, end, loose: true };
                        disabled_ranges
                            .entry(lint)
                            .and_modify(|r| r.push(range))
                            .or_insert(vec![range]);
                    }
                }
                InlineConfigItem::DisableLine(lint) => {
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
                    let range = DisabledRange { start, end, loose: false };
                    disabled_ranges
                        .entry(lint)
                        .and_modify(|r| r.push(range))
                        .or_insert(vec![range]);
                }
                InlineConfigItem::DisableNextLine(lint) => {
                    let offset = sp.hi().to_usize();
                    let mut char_indices =
                        src[offset..].char_indices().skip_while(|(_, ch)| *ch != '\n').skip(1);
                    if let Some((mut start, _)) = char_indices.next() {
                        start += offset;
                        let end = char_indices
                            .find(|(_, ch)| *ch == '\n')
                            .map(|(idx, _)| offset + idx + 1)
                            .unwrap_or(src.len());
                        let range = DisabledRange { start, end, loose: false };
                        disabled_ranges
                            .entry(lint)
                            .and_modify(|r| r.push(range))
                            .or_insert(vec![range]);
                    }
                }
                InlineConfigItem::DisableStart(lint) => {
                    disabled_blocks
                        .entry(lint)
                        .and_modify(|(_, depth)| *depth += 1)
                        .or_insert((sp.hi().to_usize(), 1));
                }
                InlineConfigItem::DisableEnd(lint) => {
                    if let Some((start, depth)) = disabled_blocks.get_mut(&lint) {
                        *depth = depth.saturating_sub(1);

                        if *depth == 0 {
                            let start = *start;
                            _ = disabled_blocks.remove(&lint);
                            let range =
                                DisabledRange { start, end: sp.lo().to_usize(), loose: false };
                            disabled_ranges
                                .entry(lint)
                                .and_modify(|r| r.push(range))
                                .or_insert(vec![range]);
                        }
                    }
                }
            }
        }

        for (lint, (start, _)) in disabled_blocks {
            let range = DisabledRange { start, end: src.len(), loose: false };
            disabled_ranges.entry(lint).and_modify(|r| r.push(range)).or_insert(vec![range]);
        }
        Self { disabled_ranges }
    }

    /// Check if the lint location is in a disabled range
    pub fn is_disabled(&self, span: Span, lint: &str) -> bool {
        if let Some(ranges) = self.disabled_ranges.get(lint) {
            return ranges.iter().any(|range| range.includes(span.to_range()));
        }

        if let Some(ranges) = self.disabled_ranges.get("all") {
            return ranges.iter().any(|range| range.includes(span.to_range()));
        }

        false
    }

    ///
    pub fn lints_ids(&self) -> impl Iterator<Item = &String> + '_ {
        self.disabled_ranges.iter().map(|(k, _)| k)
    }
}
