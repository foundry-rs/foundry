use solar::{
    interface::{BytePos, RelativeBytePos, SourceMap, Span},
    parse::ast::{self, Visit},
};
use std::{
    collections::{HashMap, hash_map::Entry},
    hash::Hash,
    ops::ControlFlow,
};

/// A disabled formatting range.
#[derive(Debug, Clone, Copy)]
struct DisabledRange<T = BytePos> {
    /// Start position, inclusive.
    lo: T,
    /// End position, inclusive.
    hi: T,
}

impl DisabledRange<BytePos> {
    fn includes(&self, span: Span) -> bool {
        span.lo() >= self.lo && span.hi() <= self.hi
    }
}

/// An inline config item
#[derive(Clone, Debug)]
pub enum InlineConfigItem<I> {
    /// Disables the next code (AST) item regardless of newlines
    DisableNextItem(I),
    /// Disables formatting on the current line
    DisableLine(I),
    /// Disables formatting between the next newline and the newline after
    DisableNextLine(I),
    /// Disables formatting for any code that follows this and before the next "disable-end"
    DisableStart(I),
    /// Disables formatting for any code that precedes this and after the previous "disable-start"
    DisableEnd(I),
}

impl InlineConfigItem<Vec<String>> {
    /// Parse an inline config item from a string. Validates IDs against available IDs.
    pub fn parse(s: &str, available_ids: &[&str]) -> Result<Self, InvalidInlineConfigItem> {
        let (disable, relevant) = s.split_once('(').unwrap_or((s, ""));
        let ids = if relevant.is_empty() || relevant == "all)" {
            vec!["all".to_string()]
        } else {
            match relevant.split_once(')') {
                Some((id_str, _)) => id_str.split(",").map(|s| s.trim().to_string()).collect(),
                None => return Err(InvalidInlineConfigItem::Syntax(s.into())),
            }
        };

        // Validate IDs
        let mut invalid_ids = Vec::new();
        'ids: for id in &ids {
            if id == "all" {
                continue;
            }
            for available_id in available_ids {
                if *available_id == id {
                    continue 'ids;
                }
            }
            invalid_ids.push(id.to_owned());
        }

        if !invalid_ids.is_empty() {
            return Err(InvalidInlineConfigItem::Ids(invalid_ids));
        }

        let res = match disable {
            "disable-next-item" => Self::DisableNextItem(ids),
            "disable-line" => Self::DisableLine(ids),
            "disable-next-line" => Self::DisableNextLine(ids),
            "disable-start" => Self::DisableStart(ids),
            "disable-end" => Self::DisableEnd(ids),
            s => return Err(InvalidInlineConfigItem::Syntax(s.into())),
        };

        Ok(res)
    }
}

impl std::str::FromStr for InlineConfigItem<()> {
    type Err = InvalidInlineConfigItem;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "disable-next-item" => Self::DisableNextItem(()),
            "disable-line" => Self::DisableLine(()),
            "disable-next-line" => Self::DisableNextLine(()),
            "disable-start" => Self::DisableStart(()),
            "disable-end" => Self::DisableEnd(()),
            s => return Err(InvalidInlineConfigItem::Syntax(s.into())),
        })
    }
}

#[derive(Debug)]
pub enum InvalidInlineConfigItem {
    Syntax(String),
    Ids(Vec<String>),
}

impl std::fmt::Display for InvalidInlineConfigItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syntax(s) => write!(f, "invalid inline config item: {s}"),
            Self::Ids(ids) => {
                write!(f, "unknown id: '{}'", ids.join("', '"))
            }
        }
    }
}

/// A trait for `InlineConfigItem` types that can be iterated over to produce keys for storage.
pub trait ItemIdIterator {
    type Item: Eq + Hash + Clone;
    fn into_iter(self) -> impl IntoIterator<Item = Self::Item>;
}

impl ItemIdIterator for () {
    type Item = ();
    fn into_iter(self) -> impl IntoIterator<Item = Self::Item> {
        std::iter::once(())
    }
}

impl ItemIdIterator for Vec<String> {
    type Item = String;
    fn into_iter(self) -> impl IntoIterator<Item = Self::Item> {
        self
    }
}

#[derive(Debug, Default)]
pub struct InlineConfig<I: ItemIdIterator> {
    disabled_ranges: HashMap<I::Item, Vec<DisabledRange>>,
}

impl<I: ItemIdIterator> InlineConfig<I> {
    /// Build a new inline config with an iterator of inline config items and their locations in a
    /// source file.
    ///
    /// # Panics
    ///
    /// Panics if `items` is not sorted in ascending order of [`Span`]s.
    pub fn from_ast<'ast>(
        items: impl IntoIterator<Item = (Span, InlineConfigItem<I>)>,
        ast: &'ast ast::SourceUnit<'ast>,
        source_map: &SourceMap,
    ) -> Self {
        Self::build(items, source_map, |offset| NextItemFinder::new(offset).find(ast))
    }

    fn build(
        items: impl IntoIterator<Item = (Span, InlineConfigItem<I>)>,
        source_map: &SourceMap,
        mut find_next_item: impl FnMut(BytePos) -> Option<Span>,
    ) -> Self {
        let mut cfg = Self::new();
        let mut disabled_blocks = HashMap::new();

        let mut prev_sp = Span::DUMMY;
        for (sp, item) in items {
            if cfg!(debug_assertions) {
                assert!(sp >= prev_sp, "InlineConfig::new: unsorted items: {sp:?} < {prev_sp:?}");
                prev_sp = sp;
            }

            cfg.disable_item(sp, item, source_map, &mut disabled_blocks, &mut find_next_item);
        }

        for (id, (_, lo, hi)) in disabled_blocks {
            cfg.disable(id, DisabledRange { lo, hi });
        }

        cfg
    }

    fn new() -> Self {
        Self { disabled_ranges: HashMap::new() }
    }

    fn disable_many(&mut self, ids: I, range: DisabledRange) {
        for id in ids.into_iter() {
            self.disable(id, range);
        }
    }

    fn disable(&mut self, id: I::Item, range: DisabledRange) {
        self.disabled_ranges.entry(id).or_default().push(range);
    }

    fn disable_item(
        &mut self,
        span: Span,
        item: InlineConfigItem<I>,
        source_map: &SourceMap,
        disabled_blocks: &mut HashMap<I::Item, (usize, BytePos, BytePos)>,
        find_next_item: &mut dyn FnMut(BytePos) -> Option<Span>,
    ) {
        let result = source_map.span_to_source(span).unwrap();
        let file = result.file;
        let comment_range = result.data;
        let src = file.src.as_str();

        match item {
            InlineConfigItem::DisableNextItem(ids) => {
                if let Some(next_item) = find_next_item(span.hi()) {
                    self.disable_many(
                        ids,
                        DisabledRange { lo: next_item.lo(), hi: next_item.hi() },
                    );
                }
            }
            InlineConfigItem::DisableLine(ids) => {
                let start = src[..comment_range.start].rfind('\n').map_or(0, |i| i);
                let end = src[comment_range.end..]
                    .find('\n')
                    .map_or(src.len(), |i| comment_range.end + i);
                self.disable_many(
                    ids,
                    DisabledRange {
                        lo: file.absolute_position(RelativeBytePos::from_usize(start)),
                        hi: file.absolute_position(RelativeBytePos::from_usize(end)),
                    },
                );
            }
            InlineConfigItem::DisableNextLine(ids) => {
                if let Some(offset) = src[comment_range.end..].find('\n') {
                    let next_line = comment_range.end + offset + 1;
                    if next_line < src.len() {
                        let end = src[next_line..].find('\n').map_or(src.len(), |i| next_line + i);
                        self.disable_many(
                            ids,
                            DisabledRange {
                                lo: file.absolute_position(RelativeBytePos::from_usize(
                                    comment_range.start,
                                )),
                                hi: file.absolute_position(RelativeBytePos::from_usize(end)),
                            },
                        );
                    }
                }
            }

            InlineConfigItem::DisableStart(ids) => {
                for id in ids.into_iter() {
                    disabled_blocks.entry(id).and_modify(|(depth, _, _)| *depth += 1).or_insert((
                        1,
                        span.lo(),
                        // Use file end as fallback for unclosed blocks
                        file.absolute_position(RelativeBytePos::from_usize(src.len())),
                    ));
                }
            }
            InlineConfigItem::DisableEnd(ids) => {
                for id in ids.into_iter() {
                    if let Entry::Occupied(mut entry) = disabled_blocks.entry(id) {
                        let (depth, lo, _) = entry.get_mut();
                        *depth = depth.saturating_sub(1);

                        if *depth == 0 {
                            let lo = *lo;
                            let (id, _) = entry.remove_entry();

                            self.disable(id, DisabledRange { lo, hi: span.hi() });
                        }
                    }
                }
            }
        }
    }
}

impl InlineConfig<()> {
    /// Checks if a span is disabled (only applicable when inline config doesn't require an id).
    pub fn is_disabled(&self, span: Span) -> bool {
        if let Some(ranges) = self.disabled_ranges.get(&()) {
            return ranges.iter().any(|range| range.includes(span));
        }
        false
    }
}

impl<I: ItemIdIterator> InlineConfig<I>
where
    I::Item: std::borrow::Borrow<str>,
{
    /// Checks if a span is disabled for a specific id. Also checks against "all", which disables
    /// all rules.
    pub fn is_id_disabled(&self, span: Span, id: &str) -> bool {
        self.is_id_disabled_inner(span, id)
            || (id != "all" && self.is_id_disabled_inner(span, "all"))
    }

    fn is_id_disabled_inner(&self, span: Span, id: &str) -> bool {
        if let Some(ranges) = self.disabled_ranges.get(id)
            && ranges.iter().any(|range| range.includes(span))
        {
            return true;
        }

        false
    }
}

macro_rules! find_next_item {
    ($self:expr, $x:expr, $span:expr, $walk:ident) => {{
        let span = $span;
        // If the item is *entirely* before the offset, skip traversing it.
        if span.hi() < $self.offset {
            return ControlFlow::Continue(());
        }
        // Check if this item starts after the offset.
        if span.lo() > $self.offset {
            return ControlFlow::Break(span);
        }
        // Otherwise, continue traversing inside this item.
        $self.$walk($x)
    }};
}

/// An AST visitor that finds the first `Item` that starts after a given offset.
#[derive(Debug)]
struct NextItemFinder {
    /// The offset to search after.
    offset: BytePos,
}

impl NextItemFinder {
    fn new(offset: BytePos) -> Self {
        Self { offset }
    }

    /// Finds the next AST item or statement which a span that begins after the `offset`.
    fn find<'ast>(&mut self, ast: &'ast ast::SourceUnit<'ast>) -> Option<Span> {
        match self.visit_source_unit(ast) {
            ControlFlow::Break(span) => Some(span),
            ControlFlow::Continue(()) => None,
        }
    }
}

impl<'ast> ast::Visit<'ast> for NextItemFinder {
    type BreakValue = Span;

    fn visit_item(&mut self, item: &'ast ast::Item<'ast>) -> ControlFlow<Self::BreakValue> {
        find_next_item!(self, item, item.span, walk_item)
    }

    fn visit_stmt(&mut self, stmt: &'ast ast::Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        find_next_item!(self, stmt, stmt.span, walk_stmt)
    }

    fn visit_yul_stmt(
        &mut self,
        stmt: &'ast ast::yul::Stmt<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        find_next_item!(self, stmt, stmt.span, walk_yul_stmt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl DisabledRange<usize> {
        fn to_byte_pos(self) -> DisabledRange<BytePos> {
            DisabledRange::<BytePos> {
                lo: BytePos::from_usize(self.lo),
                hi: BytePos::from_usize(self.hi),
            }
        }

        fn includes(&self, range: std::ops::Range<usize>) -> bool {
            self.to_byte_pos().includes(Span::new(
                BytePos::from_usize(range.start),
                BytePos::from_usize(range.end),
            ))
        }
    }

    #[test]
    fn test_disabled_range_includes() {
        let strict = DisabledRange { lo: 10, hi: 20 };
        assert!(strict.includes(10..20));
        assert!(strict.includes(12..18));
        assert!(!strict.includes(5..15)); // Partial overlap fails
    }

    #[test]
    fn test_inline_config_item_from_str() {
        assert!(matches!(
            "disable-next-item".parse::<InlineConfigItem<()>>().unwrap(),
            InlineConfigItem::DisableNextItem(())
        ));
        assert!(matches!(
            "disable-line".parse::<InlineConfigItem<()>>().unwrap(),
            InlineConfigItem::DisableLine(())
        ));
        assert!(matches!(
            "disable-start".parse::<InlineConfigItem<()>>().unwrap(),
            InlineConfigItem::DisableStart(())
        ));
        assert!(matches!(
            "disable-end".parse::<InlineConfigItem<()>>().unwrap(),
            InlineConfigItem::DisableEnd(())
        ));
        assert!("invalid".parse::<InlineConfigItem<()>>().is_err());
    }

    #[test]
    fn test_inline_config_item_parse_with_lints() {
        let lint_ids = vec!["lint1", "lint2"];

        // No lints = "all"
        match InlineConfigItem::parse("disable-line", &lint_ids).unwrap() {
            InlineConfigItem::DisableLine(lints) => assert_eq!(lints, vec!["all"]),
            _ => panic!("Wrong type"),
        }

        // Valid single lint
        match InlineConfigItem::parse("disable-start(lint1)", &lint_ids).unwrap() {
            InlineConfigItem::DisableStart(lints) => assert_eq!(lints, vec!["lint1"]),
            _ => panic!("Wrong type"),
        }

        // Multiple lints with spaces
        match InlineConfigItem::parse("disable-end(lint1, lint2)", &lint_ids).unwrap() {
            InlineConfigItem::DisableEnd(lints) => assert_eq!(lints, vec!["lint1", "lint2"]),
            _ => panic!("Wrong type"),
        }

        // Invalid lint ID
        assert!(matches!(
            InlineConfigItem::parse("disable-line(unknown)", &lint_ids),
            Err(InvalidInlineConfigItem::Ids(_))
        ));

        // Malformed syntax
        assert!(matches!(
            InlineConfigItem::parse("disable-line(lint1", &lint_ids),
            Err(InvalidInlineConfigItem::Syntax(_))
        ));
    }
}
