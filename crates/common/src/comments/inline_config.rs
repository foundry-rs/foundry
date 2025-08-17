use solar_interface::{SourceMap, Span};
use solar_parse::ast::{Item, SourceUnit, visit::Visit as VisitAst};
use solar_sema::hir::{self, Visit as VisitHir};
use std::{collections::HashMap, hash::Hash, marker::PhantomData, ops::ControlFlow};

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
        ast: &'ast SourceUnit<'ast>,
        source_map: &SourceMap,
    ) -> Self {
        Self::build(items, source_map, |offset| NextItemFinderAst::new(offset).find(ast))
    }

    /// Build a new inline config with an iterator of inline config items and their locations in a
    /// source file.
    ///
    /// # Panics
    ///
    /// Panics if `items` is not sorted in ascending order of [`Span`]s.
    pub fn from_hir<'hir>(
        items: impl IntoIterator<Item = (Span, InlineConfigItem<I>)>,
        hir: &'hir hir::Hir<'hir>,
        source_id: hir::SourceId,
        source_map: &SourceMap,
    ) -> Self {
        Self::build(items, source_map, |offset| NextItemFinderHir::new(offset, hir).find(source_id))
    }

    fn build(
        items: impl IntoIterator<Item = (Span, InlineConfigItem<I>)>,
        source_map: &SourceMap,
        mut find_next_item: impl FnMut(usize) -> Option<Span>,
    ) -> Self {
        let mut disabled_ranges: HashMap<I::Item, Vec<DisabledRange>> = HashMap::new();
        let mut disabled_blocks: HashMap<I::Item, (usize, usize, usize)> = HashMap::new();

        let mut prev_sp = Span::DUMMY;
        for (sp, item) in items {
            if cfg!(debug_assertions) {
                assert!(sp >= prev_sp, "InlineConfig::new: unsorted items: {sp:?} < {prev_sp:?}");
                prev_sp = sp;
            }

            let Ok((file, comment_range)) = source_map.span_to_source(sp) else { continue };
            let src = file.src.as_str();
            match item {
                InlineConfigItem::DisableNextItem(ids) => {
                    if let Some(next_item) = find_next_item(sp.hi().to_usize()) {
                        for id in ids.into_iter() {
                            disabled_ranges.entry(id).or_default().push(DisabledRange {
                                start: next_item.lo().to_usize(),
                                end: next_item.hi().to_usize(),
                                loose: false,
                            });
                        }
                    }
                }
                InlineConfigItem::DisableLine(ids) => {
                    let start = src[..comment_range.start].rfind('\n').map_or(0, |i| i);
                    let end = src[comment_range.end..]
                        .find('\n')
                        .map_or(src.len(), |i| comment_range.end + i);

                    for id in ids.into_iter() {
                        disabled_ranges.entry(id).or_default().push(DisabledRange {
                            start: start + file.start_pos.to_usize(),
                            end: end + file.start_pos.to_usize(),
                            loose: false,
                        })
                    }
                }
                InlineConfigItem::DisableNextLine(ids) => {
                    if let Some(offset) = src[comment_range.end..].find('\n') {
                        let next_line = comment_range.end + offset + 1;
                        if next_line < src.len() {
                            let end =
                                src[next_line..].find('\n').map_or(src.len(), |i| next_line + i);
                            for id in ids.into_iter() {
                                disabled_ranges.entry(id).or_default().push(DisabledRange {
                                    start: comment_range.start + file.start_pos.to_usize(),
                                    end: end + file.start_pos.to_usize(),
                                    loose: false,
                                })
                            }
                        }
                    }
                }
                InlineConfigItem::DisableStart(ids) => {
                    for id in ids.into_iter() {
                        disabled_blocks
                            .entry(id)
                            .and_modify(|(_, depth, _)| *depth += 1)
                            .or_insert((
                                sp.lo().to_usize(),
                                1,
                                // Use file end as fallback for unclosed blocks
                                file.start_pos.to_usize() + src.len(),
                            ));
                    }
                }
                InlineConfigItem::DisableEnd(ids) => {
                    for id in ids.into_iter() {
                        if let Some((start, depth, _)) = disabled_blocks.get_mut(&id) {
                            *depth = depth.saturating_sub(1);

                            if *depth == 0 {
                                let start = *start;
                                _ = disabled_blocks.remove(&id);

                                disabled_ranges.entry(id).or_default().push(DisabledRange {
                                    start,
                                    end: sp.hi().to_usize(),
                                    loose: false,
                                })
                            }
                        }
                    }
                }
            }
        }

        for (id, (start, _, file_end)) in disabled_blocks {
            disabled_ranges.entry(id).or_default().push(DisabledRange {
                start,
                end: file_end,
                loose: false,
            });
        }

        Self { disabled_ranges }
    }
}

impl<I> InlineConfig<I>
where
    I: ItemIdIterator,
    I::Item: Clone + Eq + Hash,
{
    /// Checks if a span is disabled (only applicable when inline config doesn't require an id).
    pub fn is_disabled(&self, span: Span) -> bool
    where
        I: ItemIdIterator<Item = ()>,
    {
        if let Some(ranges) = self.disabled_ranges.get(&()) {
            return ranges.iter().any(|range| range.includes(span.to_range()));
        }
        false
    }

    /// Checks if a span is disabled for a specific id. Also checks against "all", which disables
    /// all rules.
    pub fn is_disabled_with_id(&self, span: Span, id: &str) -> bool
    where
        I::Item: std::borrow::Borrow<str>,
    {
        if let Some(ranges) = self.disabled_ranges.get(id) {
            if ranges.iter().any(|range| range.includes(span.to_range())) {
                return true;
            }
        }

        if let Some(ranges) = self.disabled_ranges.get("all") {
            if ranges.iter().any(|range| range.includes(span.to_range())) {
                return true;
            }
        }

        false
    }
}

/// An AST visitor that finds the first `Item` that starts after a given offset.
#[derive(Debug, Default)]
struct NextItemFinderAst<'ast> {
    /// The offset to search after.
    offset: usize,
    _pd: PhantomData<&'ast ()>,
}

impl<'ast> NextItemFinderAst<'ast> {
    fn new(offset: usize) -> Self {
        Self { offset, _pd: PhantomData }
    }

    /// Finds the next AST item or statement which a span that begins after the `offset`.
    fn find(&mut self, ast: &'ast SourceUnit<'ast>) -> Option<Span> {
        match self.visit_source_unit(ast) {
            ControlFlow::Break(span) => Some(span),
            ControlFlow::Continue(()) => None,
        }
    }
}

impl<'ast> VisitAst<'ast> for NextItemFinderAst<'ast> {
    type BreakValue = Span;

    fn visit_item(&mut self, item: &'ast Item<'ast>) -> ControlFlow<Self::BreakValue> {
        // Check if this item starts after the offset.
        if item.span.lo().to_usize() > self.offset {
            return ControlFlow::Break(item.span);
        }

        // Otherwise, continue traversing inside this item.
        self.walk_item(item)
    }

    fn visit_stmt(
        &mut self,
        stmt: &'ast solar_sema::ast::Stmt<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        // Check if this stmt starts after the offset.
        if stmt.span.lo().to_usize() > self.offset {
            return ControlFlow::Break(stmt.span);
        }

        // Otherwise, continue traversing inside this stmt.
        self.walk_stmt(stmt)
    }
}

/// A HIR visitor that finds the first `Item` that starts after a given offset.
#[derive(Debug)]
struct NextItemFinderHir<'hir> {
    hir: &'hir hir::Hir<'hir>,
    /// The offset to search after.
    offset: usize,
}

impl<'hir> NextItemFinderHir<'hir> {
    fn new(offset: usize, hir: &'hir hir::Hir<'hir>) -> Self {
        Self { offset, hir }
    }

    /// Finds the next HIR item which a span that begins after the `offset`.
    fn find(&mut self, id: hir::SourceId) -> Option<Span> {
        match self.visit_nested_source(id) {
            ControlFlow::Break(span) => Some(span),
            ControlFlow::Continue(()) => None,
        }
    }
}

impl<'hir> VisitHir<'hir> for NextItemFinderHir<'hir> {
    type BreakValue = Span;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_item(&mut self, item: hir::Item<'hir, 'hir>) -> ControlFlow<Self::BreakValue> {
        // Check if this item starts after the offset.
        if item.span().lo().to_usize() > self.offset {
            return ControlFlow::Break(item.span());
        }

        // If the item is before the offset, skip traverse.
        if item.span().hi().to_usize() < self.offset {
            return ControlFlow::Continue(());
        }

        // Otherwise, continue traversing inside this item.
        self.walk_item(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_range_includes() {
        // Strict mode - requires full containment
        let strict = DisabledRange { start: 10, end: 20, loose: false };
        assert!(strict.includes(10..20));
        assert!(strict.includes(12..18));
        assert!(!strict.includes(5..15)); // Partial overlap fails
        
        // Loose mode - only checks start position
        let loose = DisabledRange { start: 10, end: 20, loose: true };
        assert!(loose.includes(10..25)); // Start in range
        assert!(!loose.includes(5..15));  // Start before range
    }

    #[test]
    fn test_inline_config_item_from_str() {
        assert!(matches!("disable-next-item".parse::<InlineConfigItem<()>>().unwrap(), InlineConfigItem::DisableNextItem(())));
        assert!(matches!("disable-line".parse::<InlineConfigItem<()>>().unwrap(), InlineConfigItem::DisableLine(())));
        assert!(matches!("disable-start".parse::<InlineConfigItem<()>>().unwrap(), InlineConfigItem::DisableStart(())));
        assert!(matches!("disable-end".parse::<InlineConfigItem<()>>().unwrap(), InlineConfigItem::DisableEnd(())));
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
