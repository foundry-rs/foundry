use solar_ast::{Item, SourceUnit, visit::Visit as VisitAst};
use solar_interface::SourceMap;
use solar_parse::ast::Span;
use solar_sema::hir::{self, Visit as VisitHir};
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
    pub fn parse(s: &str, lint_ids: &[&str]) -> Result<Self, InvalidInlineConfigItem> {
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
            for lint in lint_ids {
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
    pub fn from_ast<'ast>(
        items: impl IntoIterator<Item = (Span, InlineConfigItem)>,
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
        items: impl IntoIterator<Item = (Span, InlineConfigItem)>,
        hir: &'hir hir::Hir<'hir>,
        source_id: hir::SourceId,
        source_map: &SourceMap,
    ) -> Self {
        Self::build(items, source_map, |offset| NextItemFinderHir::new(offset, hir).find(source_id))
    }

    fn build(
        items: impl IntoIterator<Item = (Span, InlineConfigItem)>,
        source_map: &SourceMap,
        mut find_next_item: impl FnMut(usize) -> Option<Span>,
    ) -> Self {
        let mut disabled_ranges: HashMap<String, Vec<DisabledRange>> = HashMap::new();
        let mut disabled_blocks: HashMap<String, (usize, usize, usize)> = HashMap::new();

        let mut prev_sp = Span::DUMMY;
        for (sp, item) in items {
            if cfg!(debug_assertions) {
                assert!(sp >= prev_sp, "InlineConfig::new: unsorted items: {sp:?} < {prev_sp:?}");
                prev_sp = sp;
            }

            let Ok((file, comment_range)) = source_map.span_to_source(sp) else { continue };
            let src = file.src.as_str();
            match item {
                InlineConfigItem::DisableNextItem(lints) => {
                    if let Some(next_item) = find_next_item(sp.hi().to_usize()) {
                        for lint in lints {
                            disabled_ranges.entry(lint).or_default().push(DisabledRange {
                                start: next_item.lo().to_usize(),
                                end: next_item.hi().to_usize(),
                                loose: false,
                            });
                        }
                    }
                }
                InlineConfigItem::DisableLine(lints) => {
                    let start = src[..comment_range.start].rfind('\n').map_or(0, |i| i);
                    let end = src[comment_range.end..]
                        .find('\n')
                        .map_or(src.len(), |i| comment_range.end + i);

                    for lint in lints {
                        disabled_ranges.entry(lint).or_default().push(DisabledRange {
                            start: start + file.start_pos.to_usize(),
                            end: end + file.start_pos.to_usize(),
                            loose: false,
                        })
                    }
                }
                InlineConfigItem::DisableNextLine(lints) => {
                    if let Some(offset) = src[comment_range.end..].find('\n') {
                        let start = comment_range.end + offset + 1;
                        if start < src.len() {
                            let end = src[start..].find('\n').map_or(src.len(), |i| start + i);
                            for lint in lints {
                                disabled_ranges.entry(lint).or_default().push(DisabledRange {
                                    start: start + file.start_pos.to_usize(),
                                    end: end + file.start_pos.to_usize(),
                                    loose: false,
                                })
                            }
                        }
                    }
                }
                InlineConfigItem::DisableStart(lints) => {
                    for lint in lints {
                        disabled_blocks
                            .entry(lint)
                            .and_modify(|(_, depth, _)| *depth += 1)
                            .or_insert((
                                sp.hi().to_usize(),
                                1,
                                // Use file end as fallback for unclosed blocks
                                file.start_pos.to_usize() + src.len(),
                            ));
                    }
                }
                InlineConfigItem::DisableEnd(lints) => {
                    for lint in lints {
                        if let Some((start, depth, _)) = disabled_blocks.get_mut(&lint) {
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

        for (lint, (start, _, file_end)) in disabled_blocks {
            disabled_ranges.entry(lint).or_default().push(DisabledRange {
                start,
                end: file_end,
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
struct NextItemFinderAst<'ast> {
    /// The offset to search after.
    offset: usize,
    _pd: PhantomData<&'ast ()>,
}

impl<'ast> NextItemFinderAst<'ast> {
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
    fn parse_all_without_parens_defaults_to_all() {
        let ids: [&str; 1] = ["some_lint"];
        let item = InlineConfigItem::parse("disable-line", &ids).expect("should parse");
        match item {
            InlineConfigItem::DisableLine(lints) => assert_eq!(lints, vec!["all".to_string()]),
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn parse_all_with_parens() {
        let ids: [&str; 0] = [];
        let item = InlineConfigItem::parse("disable-next-item(all)", &ids).expect("should parse");
        match item {
            InlineConfigItem::DisableNextItem(lints) => assert_eq!(lints, vec!["all".to_string()]),
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn parse_specific_lints_are_trimmed_and_preserved() {
        let ids = ["foo", "bar", "baz"]; // available lint ids
        let item = InlineConfigItem::parse("disable-next-line(foo, bar)", &ids).expect("should parse");
        match item {
            InlineConfigItem::DisableNextLine(lints) => {
                assert_eq!(lints, vec!["foo".to_string(), "bar".to_string()]);
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn parse_invalid_lint_ids_are_reported() {
        let ids = ["foo"]; // only foo is valid
        let err = InlineConfigItem::parse("disable-line(foo, qux, nope)", &ids)
            .expect_err("should error for unknown ids");
        match err {
            InvalidInlineConfigItem::LintIds(mut unknown) => {
                unknown.sort();
                assert_eq!(unknown, vec!["nope".to_string(), "qux".to_string()]);
            }
            _ => panic!("expected LintIds error"),
        }
    }

    #[test]
    fn parse_syntax_errors_unknown_directive() {
        let ids: [&str; 0] = [];
        let err = InlineConfigItem::parse("disable-something(all)", &ids)
            .expect_err("should error for unknown directive");
        match err {
            InvalidInlineConfigItem::Syntax(s) => assert_eq!(s, "disable-something"),
            _ => panic!("expected Syntax error"),
        }
    }

    #[test]
    fn parse_syntax_errors_missing_closing_paren() {
        let ids: [&str; 0] = [];
        let err = InlineConfigItem::parse("disable-line(all", &ids)
            .expect_err("should error for missing closing paren");
        match err {
            InvalidInlineConfigItem::Syntax(s) => assert_eq!(s, "disable-line(all"),
            _ => panic!("expected Syntax error"),
        }
    }

    #[test]
    fn invalid_inline_config_item_display() {
        let s = InvalidInlineConfigItem::Syntax("oops".into()).to_string();
        assert_eq!(s, "invalid inline config item: oops");

        let ids = InvalidInlineConfigItem::LintIds(vec!["a".into(), "b".into()]).to_string();
        assert_eq!(ids, "unknown lint id: 'a', 'b'");
    }

    #[test]
    fn disabled_range_includes_strict() {
        let range = DisabledRange { start: 10, end: 20, loose: false };
        // inside
        assert!(range.includes(12..18));
        // boundaries
        assert!(range.includes(10..20));
        assert!(range.includes(10..10));
        assert!(range.includes(20..20));
        // starts before
        assert!(!range.includes(9..12));
        // ends after
        assert!(!range.includes(12..21));
        // both out
        assert!(!range.includes(0..9));
    }

    #[test]
    fn disabled_range_includes_loose() {
        let range = DisabledRange { start: 10, end: 20, loose: true };
        // start inside, end can go beyond
        assert!(range.includes(12..100));
        // start exactly at end still counts if >= start and <= end
        assert!(range.includes(20..100));
        // start before start is not included
        assert!(!range.includes(0..15));
        // start after end is not included
        assert!(!range.includes(21..30));
    }
}
