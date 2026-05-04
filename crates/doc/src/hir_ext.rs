//! HIR-aware enrichments.
//!
//! Pure functions over `solar`'s HIR:
//! * `build_name_to_page`: maps contract names to their MDX page paths.
//! * `inheritance_links`: `**Inherits:**` line for a contract page.
//! * `resolve_inheritdoc`: pulls natspec from a base contract member.
//! * `replace_inline_links`: rewrites `{Ident}` to markdown links.

use solar::{
    ast::{ContractKind, DocComments, FunctionKind, NatSpecKind},
    interface::source_map::FileName,
    sema::{Gcx, hir},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

// ── name-to-page map ──────────────────────────────────────────────────────────

/// Maps a Solidity identifier (contract / struct / enum / error / event / UDVT)
/// to its output MDX page path relative to `pages/`.
pub type NameToPage = HashMap<String, PathBuf>;

/// Build the `NameToPage` map from HIR by re-deriving each item's output path.
///
/// This mirrors the path computation in `render::source` so links can be resolved
/// before rendering begins.
pub fn build_name_to_page(gcx: Gcx<'_>, root: &Path) -> NameToPage {
    let mut map = NameToPage::new();

    // Contracts.
    for id in gcx.hir.contract_ids() {
        let c = gcx.hir.contract(id);
        if let Some(rel) = source_rel_path(gcx, c.source, root) {
            let out_dir = rel.parent().unwrap_or(Path::new("")).to_owned();
            let kind = match c.kind {
                ContractKind::Contract => "contract",
                ContractKind::AbstractContract => "abstract",
                ContractKind::Interface => "interface",
                ContractKind::Library => "library",
            };
            let page = out_dir.join(format!("{kind}.{}.mdx", c.name.as_str()));
            map.insert(c.name.as_str().to_string(), page);
        }
    }

    // Structs.
    for id in gcx.hir.strukt_ids() {
        let s = gcx.hir.strukt(id);
        if s.contract.is_none()
            && let Some(rel) = source_rel_path(gcx, s.source, root)
        {
            let out_dir = rel.parent().unwrap_or(Path::new("")).to_owned();
            let page = out_dir.join(format!("struct.{}.mdx", s.name.as_str()));
            map.insert(s.name.as_str().to_string(), page);
        }
    }

    // Enums.
    for id in gcx.hir.enumm_ids() {
        let e = gcx.hir.enumm(id);
        if e.contract.is_none()
            && let Some(rel) = source_rel_path(gcx, e.source, root)
        {
            let out_dir = rel.parent().unwrap_or(Path::new("")).to_owned();
            let page = out_dir.join(format!("enum.{}.mdx", e.name.as_str()));
            map.insert(e.name.as_str().to_string(), page);
        }
    }

    // Errors.
    for id in gcx.hir.error_ids() {
        let e = gcx.hir.error(id);
        if e.contract.is_none()
            && let Some(rel) = source_rel_path(gcx, e.source, root)
        {
            let out_dir = rel.parent().unwrap_or(Path::new("")).to_owned();
            let page = out_dir.join(format!("error.{}.mdx", e.name.as_str()));
            map.insert(e.name.as_str().to_string(), page);
        }
    }

    // Events.
    for id in gcx.hir.event_ids() {
        let e = gcx.hir.event(id);
        if e.contract.is_none()
            && let Some(rel) = source_rel_path(gcx, e.source, root)
        {
            let out_dir = rel.parent().unwrap_or(Path::new("")).to_owned();
            let page = out_dir.join(format!("event.{}.mdx", e.name.as_str()));
            map.insert(e.name.as_str().to_string(), page);
        }
    }

    map
}

fn source_rel_path(gcx: Gcx<'_>, source_id: hir::SourceId, root: &Path) -> Option<PathBuf> {
    let file = &gcx.hir.source(source_id).file;
    if let FileName::Real(p) = &file.name {
        let rel = p.strip_prefix(root).unwrap_or(p);
        Some(rel.to_owned())
    } else {
        None
    }
}

// ── inheritance links ─────────────────────────────────────────────────────────

/// Returns the `**Inherits:**` markdown string for a contract, or `None` if it has no bases.
///
/// Each base is either a bare name (when no page is known) or a markdown link.
pub fn inheritance_links(
    gcx: Gcx<'_>,
    contract_id: hir::ContractId,
    name_to_page: &NameToPage,
    current_page: &Path,
) -> Option<String> {
    let contract = gcx.hir.contract(contract_id);
    if contract.bases.is_empty() {
        return None;
    }

    let parts: Vec<String> = contract
        .bases
        .iter()
        .map(|&base_id| {
            let base = gcx.hir.contract(base_id);
            let name = base.name.as_str();
            if let Some(page) = name_to_page.get(name) {
                let link = page_link(page, current_page);
                format!("[{name}]({link})")
            } else {
                name.to_string()
            }
        })
        .collect();

    Some(format!("**Inherits:** {}", parts.join(", ")))
}

// ── inheritdoc resolution ─────────────────────────────────────────────────────

/// Collected natspec tags from an inherited base member.
pub struct InheritedDoc {
    pub notices: Vec<String>,
    pub devs: Vec<String>,
    pub params: Vec<(String, String)>,
    pub returns: Vec<(String, String)>,
}

/// Resolve `@inheritdoc BaseContract` for a function named `fn_name` inside
/// `contract_id` (the current contract). Walks the linearized bases to find the
/// first matching function and returns its natspec if found.
pub fn resolve_inheritdoc(
    gcx: Gcx<'_>,
    contract_id: hir::ContractId,
    fn_name: &str,
    base_name: &str,
) -> Option<InheritedDoc> {
    let contract = gcx.hir.contract(contract_id);

    // Find the named base contract in the linearized hierarchy.
    let base_id = contract
        .linearized_bases
        .iter()
        .copied()
        .find(|&bid| gcx.hir.contract(bid).name.as_str() == base_name)?;

    // Walk the base's own items to find a function with matching name.
    let base_contract = gcx.hir.contract(base_id);
    for &item_id in base_contract.items {
        if let hir::ItemId::Function(fid) = item_id {
            let f = gcx.hir.function(fid);
            let matches = match f.kind {
                FunctionKind::Constructor => fn_name == "constructor",
                FunctionKind::Fallback => fn_name == "fallback",
                FunctionKind::Receive => fn_name == "receive",
                _ => f.name.map(|n| n.as_str() == fn_name).unwrap_or(false),
            };
            if matches {
                return extract_inherited_doc(gcx, fid);
            }
        }
    }

    None
}

fn extract_inherited_doc(gcx: Gcx<'_>, fid: hir::FunctionId) -> Option<InheritedDoc> {
    // HIR functions store a span; we need the AST doc comments.
    // The AST source has the doc comments on the Item.
    // We find the source file for this function and look up the AST item by span.
    let f = gcx.hir.function(fid);
    let ast_source = gcx.sources.get(f.source)?;
    let ast = ast_source.ast.as_ref()?;

    let fn_span = f.span;
    // Walk the AST to find the Item whose span matches.
    let docs = ast.items.iter().find_map(|item| {
        if item.span == fn_span {
            return Some(&item.docs);
        }
        // Also search inside contracts.
        if let solar::ast::ItemKind::Contract(c) = &item.kind {
            for member in c.body.iter() {
                if member.span == fn_span {
                    return Some(&member.docs);
                }
            }
        }
        None
    })?;

    Some(collect_inherited_doc(docs))
}

fn collect_inherited_doc(docs: &DocComments<'_>) -> InheritedDoc {
    let mut result = InheritedDoc {
        notices: Vec::new(),
        devs: Vec::new(),
        params: Vec::new(),
        returns: Vec::new(),
    };
    for doc in docs.iter() {
        for item in doc.natspec.iter() {
            let content = doc.natspec_content(item).trim().to_string();
            if content.is_empty() {
                continue;
            }
            match item.kind {
                NatSpecKind::Notice => result.notices.push(content),
                NatSpecKind::Dev => result.devs.push(content),
                NatSpecKind::Param { name } => {
                    result.params.push((name.as_str().to_string(), content))
                }
                NatSpecKind::Return { name } => {
                    result.returns.push((name.as_str().to_string(), content))
                }
                _ => {}
            }
        }
    }
    result
}

// ── inline link replacement ───────────────────────────────────────────────────

/// Replace `{Ident}` and `{xref-Ident}` with markdown links using `name_to_page`.
///
/// Matches the legacy pattern: `{[xref-]Ident[-part]}[label]` where `label` defaults
/// to `Ident`.
pub fn replace_inline_links(text: &str, name_to_page: &NameToPage, current_page: &Path) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'{' {
            // Try to parse {[xref-]Ident[-part]}[optional label].
            if let Some((end, ident, _part, label)) = parse_inline_link(&text[i..]) {
                // Strip the leading `xref-` prefix if present.
                let lookup_name = ident.strip_prefix("xref-").unwrap_or(ident);
                let lookup_name = if let Some(pos) = lookup_name.find('-') {
                    &lookup_name[..pos]
                } else {
                    lookup_name
                };

                if let Some(page) = name_to_page.get(lookup_name) {
                    let link = page_link(page, current_page);
                    let display = label.unwrap_or(lookup_name);
                    out.push_str(&format!("[{display}]({link})"));
                    i += end;
                    continue;
                }

                // Unresolved {Ident}, emit as inline code to avoid MDX treating it as a
                // JS expression.
                out.push_str(&format!("`{lookup_name}`"));
                i += end;
                continue;
            }
            // Bare `{` with no matching `}`, escape it.
            out.push_str("&#123;");
            i += 1;
            continue;
        }

        if bytes[i] == b'<' {
            // Escape `<` that would be parsed as a JSX/HTML tag by MDX.
            // A `<` is safe only when it's already part of a markdown link `<url>` or
            // a standard HTML entity. We unconditionally escape to `&lt;` here
            // since Solidity natspec does not produce markdown autolinks.
            out.push_str("&lt;");
            i += 1;
            continue;
        }

        // Advance by the full UTF-8 character to avoid corrupting multi-byte sequences.
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

/// Parse `{[xref-]Ident[-part]}[label]` starting at offset 0 in `s`.
///
/// Returns `(consumed_bytes, ident, part, label)` on success.
fn parse_inline_link(s: &str) -> Option<(usize, &str, Option<&str>, Option<&str>)> {
    let s = s.strip_prefix('{')?;
    let close = s.find('}')?;
    let inner = &s[..close];

    // inner = "[xref-]Ident[-part]"
    let (raw_ident, raw_part) = if let Some(dash) = inner.rfind('-') {
        // Heuristic: only split on dash if the suffix looks like a member name.
        let candidate_ident = &inner[..dash];
        let candidate_part = &inner[dash + 1..];
        if candidate_ident.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            && !candidate_part.is_empty()
        {
            (candidate_ident, Some(candidate_part))
        } else {
            (inner, None)
        }
    } else {
        (inner, None)
    };

    let mut consumed = 1 + close + 1; // '{' + inner + '}'

    // Optional label: `[label]`
    let rest = &s[close + 1..];
    let label = if rest.starts_with('[') {
        if let Some(end) = rest.find(']') {
            let lbl = &rest[1..end];
            consumed += end + 1;
            Some(lbl)
        } else {
            None
        }
    } else {
        None
    };

    Some((consumed, raw_ident, raw_part, label))
}

// ── path helpers ──────────────────────────────────────────────────────────────

/// Produce a vocs-style link from `page` relative to `current_page`.
///
/// vocs uses root-relative links (starting with `/`).
fn page_link(page: &Path, _current_page: &Path) -> String {
    // Strip .mdx extension and produce an absolute path from the pages root.
    let without_ext = page.with_extension("");
    format!("/{}", without_ext.display())
}
