use solar::{
    interface::{BytePos, RelativeBytePos, SourceMap, Span},
    parse::ast::{self, Visit},
};
use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    hash::Hash,
    ops::ControlFlow,
    sync::atomic::{AtomicBool, Ordering},
};

/// An inline suppression and its optional target range.
#[derive(Debug)]
struct DisabledRange<T = BytePos> {
    /// Disabled range, or `None` if the directive has no target.
    range: Option<(T, T)>,
    /// Whether the range stems from a `disable-start`/`disable-end` block.
    block: bool,
    /// Span of the directive comment that created this range, used to report unused suppressions.
    directive: Span,
    /// Whether this range suppressed at least one diagnostic during the run.
    used: AtomicBool,
}

impl DisabledRange<BytePos> {
    fn includes(&self, span: Span) -> bool {
        self.range.is_some_and(|(lo, hi)| span.lo() >= lo && span.hi() <= hi)
    }

    /// Marks the range as having suppressed a diagnostic and reports whether it includes `span`.
    fn mark_if_includes(&self, span: Span) -> bool {
        if self.includes(span) {
            self.used.store(true, Ordering::Relaxed);
            return true;
        }
        false
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
        let mut ids = if relevant.is_empty() || relevant == "all)" {
            vec!["all".to_string()]
        } else {
            match relevant.split_once(')') {
                Some((id_str, _)) => id_str.split(',').map(|s| s.trim().to_string()).collect(),
                None => return Err(InvalidInlineConfigItem::Syntax(s.into())),
            }
        };
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));

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
        let mut disabled_blocks = HashMap::<I::Item, Vec<(BytePos, BytePos, Span)>>::new();

        let mut prev_sp = Span::DUMMY;
        for (sp, item) in items {
            if cfg!(debug_assertions) {
                assert!(sp >= prev_sp, "InlineConfig::new: unsorted items: {sp:?} < {prev_sp:?}");
                prev_sp = sp;
            }

            cfg.disable_item(sp, item, source_map, &mut disabled_blocks, &mut find_next_item);
        }

        for (id, blocks) in disabled_blocks {
            for (lo, hi, directive) in blocks {
                cfg.disable(id.clone(), Some((lo, hi)), true, directive);
            }
        }

        cfg
    }

    fn new() -> Self {
        Self { disabled_ranges: HashMap::new() }
    }

    fn disable_many(
        &mut self,
        ids: I,
        range: Option<(BytePos, BytePos)>,
        block: bool,
        directive: Span,
    ) {
        for id in ids.into_iter() {
            self.disable(id, range, block, directive);
        }
    }

    fn disable(
        &mut self,
        id: I::Item,
        range: Option<(BytePos, BytePos)>,
        block: bool,
        directive: Span,
    ) {
        self.disabled_ranges.entry(id).or_default().push(DisabledRange {
            range,
            block,
            directive,
            used: AtomicBool::new(false),
        });
    }

    fn disable_item(
        &mut self,
        span: Span,
        item: InlineConfigItem<I>,
        source_map: &SourceMap,
        disabled_blocks: &mut HashMap<I::Item, Vec<(BytePos, BytePos, Span)>>,
        find_next_item: &mut dyn FnMut(BytePos) -> Option<Span>,
    ) {
        let result = source_map.span_to_source(span).unwrap();
        let file = result.file;
        let comment_range = result.data;
        let src = file.src.as_str();

        #[allow(clippy::collapsible_match)]
        match item {
            InlineConfigItem::DisableNextItem(ids) => {
                let range = find_next_item(span.hi()).map(|item| (item.lo(), item.hi()));
                self.disable_many(ids, range, false, span);
            }
            InlineConfigItem::DisableLine(ids) => {
                let start = src[..comment_range.start].rfind('\n').unwrap_or(0);
                let end = src[comment_range.end..]
                    .find('\n')
                    .map_or(src.len(), |i| comment_range.end + i);
                self.disable_many(
                    ids,
                    Some((
                        file.absolute_position(RelativeBytePos::from_usize(start)),
                        file.absolute_position(RelativeBytePos::from_usize(end)),
                    )),
                    false,
                    span,
                );
            }
            InlineConfigItem::DisableNextLine(ids) => {
                let range = src[comment_range.end..].find('\n').and_then(|offset| {
                    let next_line = comment_range.end + offset + 1;
                    (next_line < src.len()).then(|| {
                        let end = src[next_line..].find('\n').map_or(src.len(), |i| next_line + i);
                        (
                            file.absolute_position(RelativeBytePos::from_usize(
                                comment_range.start,
                            )),
                            file.absolute_position(RelativeBytePos::from_usize(end)),
                        )
                    })
                });
                self.disable_many(ids, range, false, span);
            }

            InlineConfigItem::DisableStart(ids) => {
                for id in ids.into_iter() {
                    disabled_blocks.entry(id).or_default().push((
                        span.lo(),
                        // Use file end as fallback for unclosed blocks
                        file.absolute_position(RelativeBytePos::from_usize(src.len())),
                        span,
                    ));
                }
            }
            InlineConfigItem::DisableEnd(ids) => {
                for id in ids.into_iter() {
                    // An unmatched end closes no suppression and is ignored.
                    if let Some(blocks) = disabled_blocks.get_mut(&id)
                        && let Some((lo, _, directive)) = blocks.pop()
                    {
                        self.disable(id, Some((lo, span.hi())), true, directive);
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

    /// Checks if a span is disabled by a `disable-start`/`disable-end` block, as opposed to a
    /// line-based directive such as `disable-line`.
    pub fn is_disabled_block(&self, span: Span) -> bool {
        if let Some(ranges) = self.disabled_ranges.get(&()) {
            return ranges.iter().any(|range| range.block && range.includes(span));
        }
        false
    }
}

impl<I: ItemIdIterator> InlineConfig<I>
where
    I::Item: Borrow<str>,
{
    /// Checks if a span is disabled for a specific id. Also checks against "all", which disables
    /// all rules.
    pub fn is_id_disabled(&self, span: Span, id: &str) -> bool {
        let id_disabled = self.is_id_disabled_inner(span, id);
        let all_disabled = id != "all" && self.is_id_disabled_inner(span, "all");
        id_disabled || all_disabled
    }

    fn is_id_disabled_inner(&self, span: Span, id: &str) -> bool {
        let Some(ranges) = self.disabled_ranges.get(id) else { return false };
        // Mark every matching range as used, not just the first, so overlapping suppressions are
        // all credited when reporting unused ones.
        let mut disabled = false;
        for range in ranges {
            disabled |= range.mark_if_includes(span);
        }
        disabled
    }

    /// Returns the directive span and lint id of each suppression that never suppressed a
    /// diagnostic during the run.
    ///
    /// Only suppressions for ids in `active` (or the catch-all `"all"`) are reported, so severity
    /// filters and excluded lints do not produce false positives. Results are sorted by directive
    /// location for stable output.
    pub fn unused_suppressions(&self, active: &[&str]) -> Vec<(Span, String)> {
        if active.is_empty() {
            return Vec::new();
        }
        let mut unused = Vec::new();
        for (id, ranges) in &self.disabled_ranges {
            let id = id.borrow();
            if id != "all" && !active.contains(&id) {
                continue;
            }
            for range in ranges {
                if !range.used.load(Ordering::Relaxed) {
                    unused.push((range.directive, id.to_string()));
                }
            }
        }
        unused.sort_by(|(a, ida), (b, idb)| a.lo().cmp(&b.lo()).then_with(|| ida.cmp(idb)));
        unused
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
    const fn new(offset: BytePos) -> Self {
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
        fn to_byte_pos(&self) -> DisabledRange<BytePos> {
            DisabledRange::<BytePos> {
                range: self
                    .range
                    .map(|(lo, hi)| (BytePos::from_usize(lo), BytePos::from_usize(hi))),
                block: self.block,
                directive: Span::DUMMY,
                used: AtomicBool::new(false),
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
        let strict = DisabledRange {
            range: Some((10, 20)),
            block: false,
            directive: Span::DUMMY,
            used: AtomicBool::new(false),
        };
        assert!(strict.includes(10..20));
        assert!(strict.includes(12..18));
        assert!(!strict.includes(5..15)); // Partial overlap fails
    }

    #[test]
    fn test_unused_suppressions_credits_all_overlapping_ranges() {
        let mut config = InlineConfig::<Vec<String>>::new();
        config.disable(
            "lint1".to_string(),
            Some((BytePos::from_usize(10), BytePos::from_usize(20))),
            false,
            Span::new(BytePos::from_usize(1), BytePos::from_usize(2)),
        );
        config.disable(
            "all".to_string(),
            Some((BytePos::from_usize(5), BytePos::from_usize(25))),
            true,
            Span::new(BytePos::from_usize(3), BytePos::from_usize(4)),
        );
        config.disable(
            "lint1".to_string(),
            Some((BytePos::from_usize(5), BytePos::from_usize(25))),
            true,
            Span::new(BytePos::from_usize(5), BytePos::from_usize(6)),
        );

        assert!(
            config.is_id_disabled(
                Span::new(BytePos::from_usize(12), BytePos::from_usize(18)),
                "lint1"
            )
        );
        assert!(config.unused_suppressions(&["lint1"]).is_empty());
    }

    #[test]
    fn test_unused_suppressions_tracks_missing_targets_and_active_lints() {
        let mut config = InlineConfig::<Vec<String>>::new();
        let directive = Span::new(BytePos::from_usize(1), BytePos::from_usize(2));
        config.disable_many(vec!["lint1".to_string(), "all".to_string()], None, false, directive);

        assert!(config.unused_suppressions(&[]).is_empty());
        assert_eq!(
            config.unused_suppressions(&["lint1"]),
            vec![(directive, "all".to_string()), (directive, "lint1".to_string())]
        );
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

        // Duplicate lint IDs are normalized within a directive.
        match InlineConfigItem::parse("disable-line(lint1, lint1)", &lint_ids).unwrap() {
            InlineConfigItem::DisableLine(lints) => assert_eq!(lints, vec!["lint1"]),
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
