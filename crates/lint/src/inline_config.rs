use solar_ast::{Item, SourceUnit, visit::Visit};
use solar_parse::ast::Span;
use std::{collections::HashMap, fmt, marker::PhantomData, ops::ControlFlow};

/// An inline config item
#[derive(Clone, Debug)]
pub enum InlineConfigItem {
    /// Disables the next code (AST) item regardless of newlines
    DisableNextItem(Vec<String>),
    /// Disables formatting on the current line
    DisableLine(Vec<String>),
    /// Disables formatting between the next newline and the newline after
    DisableNextLine(Vec<String>),
    /// Disables formatting for any code that follows this and before the next "disable-end"
    DisableStart(Vec<String>),
    /// Disables formatting for any code that precedes this and after the previous "disable-start"
    DisableEnd(Vec<String>),
}

impl InlineConfigItem {
    /// Parse an inline config item from a string. Validates lint IDs against available lints.
    pub fn parse(s: &str, available_lints: &[&str]) -> Result<Self, InvalidInlineConfigItem> {
        let (disable, relevant) = s.split_once('(').unwrap_or((s, ""));
        let lints = if relevant.is_empty() || relevant == "all)" {
            vec!["all".to_string()]
        } else {
            match relevant.split_once(')') {
                Some((lint, _)) => lint.split(",").map(|s| s.trim().to_string()).collect(),
                None => return Err(InvalidInlineConfigItem::Syntax(s.into())),
            }
        };

        // Validate lint IDs
        let mut invalid_ids = Vec::new();
        'ids: for id in &lints {
            if id == "all" {
                continue;
            }
            for lint in available_lints {
                if *lint == id {
                    continue 'ids;
                }
            }
            invalid_ids.push(id.to_owned());
        }

        if !invalid_ids.is_empty() {
            return Err(InvalidInlineConfigItem::LintIds(invalid_ids));
        }

        let res = match disable {
            "disable-next-item" => Self::DisableNextItem(lints),
            "disable-line" => Self::DisableLine(lints),
            "disable-next-line" => Self::DisableNextLine(lints),
            "disable-start" => Self::DisableStart(lints),
            "disable-end" => Self::DisableEnd(lints),
            s => return Err(InvalidInlineConfigItem::Syntax(s.into())),
        };

        Ok(res)
    }
}

#[derive(Debug)]
pub enum InvalidInlineConfigItem {
    Syntax(String),
    LintIds(Vec<String>),
}

impl fmt::Display for InvalidInlineConfigItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(s) => write!(f, "invalid inline config item: {s}"),
            Self::LintIds(ids) => {
                write!(f, "unknown lint id: '{}'", ids.join("', '"))
            }
        }
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
    pub fn new<'ast>(
        items: impl IntoIterator<Item = (Span, InlineConfigItem)>,
        ast: &'ast SourceUnit<'ast>,
        src: &str,
    ) -> Self {
        let mut disabled_ranges: HashMap<String, Vec<DisabledRange>> = HashMap::new();
        let mut disabled_blocks: HashMap<String, (usize, usize)> = HashMap::new();

        let mut prev_sp = Span::DUMMY;
        for (sp, item) in items {
            if cfg!(debug_assertions) {
                assert!(sp >= prev_sp, "InlineConfig::new: unsorted items: {sp:?} < {prev_sp:?}");
                prev_sp = sp;
            }

            match item {
                InlineConfigItem::DisableNextItem(lints) => {
                    let comment_end = sp.hi().to_usize();

                    if let Some(next_item) = NextItemFinder::new(comment_end).find(ast) {
                        for lint in lints {
                            disabled_ranges.entry(lint).or_default().push(DisabledRange {
                                start: next_item.lo().to_usize(),
                                end: next_item.hi().to_usize(),
                                loose: false,
                            });
                        }
                    };
                }
                InlineConfigItem::DisableLine(lints) => {
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
                    for lint in lints {
                        disabled_ranges.entry(lint).or_default().push(DisabledRange {
                            start,
                            end,
                            loose: false,
                        })
                    }
                }
                InlineConfigItem::DisableNextLine(lints) => {
                    let offset = sp.hi().to_usize();
                    let mut char_indices =
                        src[offset..].char_indices().skip_while(|(_, ch)| *ch != '\n').skip(1);
                    if let Some((mut start, _)) = char_indices.next() {
                        start += offset;
                        let end = char_indices
                            .find(|(_, ch)| *ch == '\n')
                            .map(|(idx, _)| offset + idx + 1)
                            .unwrap_or(src.len());
                        for lint in lints {
                            disabled_ranges.entry(lint).or_default().push(DisabledRange {
                                start,
                                end,
                                loose: false,
                            })
                        }
                    }
                }
                InlineConfigItem::DisableStart(lints) => {
                    for lint in lints {
                        disabled_blocks
                            .entry(lint)
                            .and_modify(|(_, depth)| *depth += 1)
                            .or_insert((sp.hi().to_usize(), 1));
                    }
                }
                InlineConfigItem::DisableEnd(lints) => {
                    for lint in lints {
                        if let Some((start, depth)) = disabled_blocks.get_mut(&lint) {
                            *depth = depth.saturating_sub(1);

                            if *depth == 0 {
                                let start = *start;
                                _ = disabled_blocks.remove(&lint);

                                disabled_ranges.entry(lint).or_default().push(DisabledRange {
                                    start,
                                    end: sp.lo().to_usize(),
                                    loose: false,
                                })
                            }
                        }
                    }
                }
            }
        }

        for (lint, (start, _)) in disabled_blocks {
            disabled_ranges.entry(lint).or_default().push(DisabledRange {
                start,
                end: src.len(),
                loose: false,
            });
        }

        Self { disabled_ranges }
    }

    /// Check if the lint location is in a disabled range.
    #[inline]
    pub fn is_disabled(&self, span: Span, lint: &str) -> bool {
        if let Some(ranges) = self.disabled_ranges.get(lint) {
            return ranges.iter().any(|range| range.includes(span.to_range()));
        }

        if let Some(ranges) = self.disabled_ranges.get("all") {
            return ranges.iter().any(|range| range.includes(span.to_range()));
        }

        false
    }
}

/// An AST visitor that finds the first `Item` that starts after a given offset.
#[derive(Debug, Default)]
struct NextItemFinder<'ast> {
    /// The offset to search after.
    offset: usize,
    _pd: PhantomData<&'ast ()>,
}

impl<'ast> NextItemFinder<'ast> {
    fn new(offset: usize) -> Self {
        Self { offset, _pd: PhantomData }
    }

    /// Finds the next AST item which a span that begins after the `offset`.
    fn find(&mut self, ast: &'ast SourceUnit<'ast>) -> Option<Span> {
        match self.visit_source_unit(ast) {
            ControlFlow::Break(span) => Some(span),
            ControlFlow::Continue(()) => None,
        }
    }
}

impl<'ast> Visit<'ast> for NextItemFinder<'ast> {
    type BreakValue = Span;

    fn visit_item(&mut self, item: &'ast Item<'ast>) -> ControlFlow<Self::BreakValue> {
        // Check if this item starts after the offset.
        if item.span.lo().to_usize() > self.offset {
            return ControlFlow::Break(item.span);
        }

        // Otherwise, continue traversing inside this item.
        self.walk_item(item)
    }
}
