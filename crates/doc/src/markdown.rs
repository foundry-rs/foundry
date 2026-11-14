//! Shared Markdown code-region and MDX ESM handling.
//!
//! README and NatSpec escaping retain their own policies for malformed fences and prose.

use markdown::{ParseOptions, mdast::Node, to_mdast};
use std::ops::Range;

/// Byte ranges that Markdown parses as code under `options`. An HTML entity would render literally
/// inside these ranges, so neutralization skips them. If malformed MDX cannot be parsed, returning
/// no ranges favors neutralizing possible ESM over preserving an invalid code example
/// byte-for-byte.
pub(crate) fn code_regions(text: &str, options: &ParseOptions) -> Vec<Range<usize>> {
    let Ok(tree) = to_mdast(text, options) else { return Vec::new() };
    let mut regions = Vec::new();
    collect_code_regions(&tree, &mut regions);
    regions
}

/// Collect fenced and inline code positions from the MDX-aware syntax tree.
fn collect_code_regions(node: &Node, regions: &mut Vec<Range<usize>>) {
    if matches!(node, Node::Code(_) | Node::InlineCode(_))
        && let Some(position) = node.position()
    {
        regions.push(position.start.offset..position.end.offset);
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_code_regions(child, regions);
        }
    }
}

/// Logical lines and their byte offsets in the original text. CRLF is one separator; lone CR and
/// LF are separators too. The separator bytes are excluded from the returned slices and preserved
/// in the source string.
pub(crate) fn logical_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let bytes = text.as_bytes();
    let mut offset = 0;
    std::iter::from_fn(move || {
        if offset >= bytes.len() {
            return None;
        }
        let start = offset;
        let end = bytes[start..]
            .iter()
            .position(|&byte| byte == b'\n' || byte == b'\r')
            .map_or(bytes.len(), |position| start + position);
        offset = if end == bytes.len() {
            end
        } else if bytes[end] == b'\r' && bytes.get(end + 1) == Some(&b'\n') {
            end + 2
        } else {
            end + 1
        };
        Some((start, &text[start..end]))
    })
}

/// Check a position against sorted, merged ranges while advancing monotonically.
pub(crate) fn region_contains(
    regions: &[Range<usize>],
    cursor: &mut usize,
    position: usize,
) -> bool {
    while regions.get(*cursor).is_some_and(|region| region.end <= position) {
        *cursor += 1;
    }
    regions.get(*cursor).is_some_and(|region| region.start <= position)
}

/// Neutralize any line MDX would parse as an ESM statement (`import ` or `export ` at column one):
/// the keyword's prefix becomes HTML entities, so the line renders the same but no longer
/// begins with an ESM token. NatSpec text can be inherited from a dependency via `@inheritdoc`, so
/// this must run wherever displayed prose is assembled. A keyword that falls inside a Markdown
/// code span or fenced code block is left untouched (see `code_regions`): the entity would render
/// literally and corrupt the example, and MDX would not execute it there.
pub(crate) fn neutralize_esm(text: &str) -> String {
    let regions = code_regions(text, &ParseOptions::mdx());
    let mut region_cursor = 0;
    let mut copied = 0;
    let mut out = String::with_capacity(text.len());

    for (line_start, line) in logical_lines(text) {
        let replacement = if line.starts_with("import ") {
            Some(("&#105;&#109;", 2))
        } else if line.starts_with("export ") {
            Some(("&#101;", 1))
        } else {
            None
        };
        let Some((entity, prefix_len)) = replacement else { continue };
        if region_contains(&regions, &mut region_cursor, line_start) {
            continue;
        }
        out.push_str(&text[copied..line_start]);
        out.push_str(entity);
        copied = line_start + prefix_len;
    }

    out.push_str(&text[copied..]);
    out
}
