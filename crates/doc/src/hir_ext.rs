//! HIR-aware enrichments.
//!
//! Pure functions over `solar`'s HIR:
//! * `build_name_to_page`: maps contract names to their MDX page paths.
//! * `inheritance_links`: `**Inherits:**` line for a contract page.
//! * `resolve_inheritdoc`: pulls natspec from a base contract member.
//! * `replace_inline_links`: rewrites `{Ident}` to markdown links.

use solar::{
    ast::{ContractKind, DocComments, FunctionKind, NatSpecKind, ParameterList},
    interface::source_map::FileName,
    sema::{Gcx, hir},
};
use std::{
    collections::{HashMap, HashSet},
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
///
/// Only items whose source file is contained in `allowed_sources` (absolute paths)
/// are included, so cross-references cannot resolve to pages that won't be emitted.
pub fn build_name_to_page(
    gcx: Gcx<'_>,
    root: &Path,
    allowed_sources: &HashSet<PathBuf>,
) -> NameToPage {
    let mut map = NameToPage::new();

    for item_id in gcx.hir.item_ids() {
        let (name, source, contract, prefix) = match item_id {
            hir::ItemId::Contract(id) => {
                let c = gcx.hir.contract(id);
                let kind = match c.kind {
                    ContractKind::Contract => "contract",
                    ContractKind::AbstractContract => "abstract",
                    ContractKind::Interface => "interface",
                    ContractKind::Library => "library",
                };
                (c.name, c.source, None, kind)
            }
            hir::ItemId::Struct(id) => {
                let s = gcx.hir.strukt(id);
                (s.name, s.source, s.contract, "struct")
            }
            hir::ItemId::Enum(id) => {
                let e = gcx.hir.enumm(id);
                (e.name, e.source, e.contract, "enum")
            }
            hir::ItemId::Error(id) => {
                let e = gcx.hir.error(id);
                (e.name, e.source, e.contract, "error")
            }
            hir::ItemId::Event(id) => {
                let e = gcx.hir.event(id);
                (e.name, e.source, e.contract, "event")
            }
            hir::ItemId::Udvt(id) => {
                let u = gcx.hir.udvt(id);
                (u.name, u.source, u.contract, "type")
            }
            hir::ItemId::Function(_) | hir::ItemId::Variable(_) => continue,
        };

        // For non-contract items, skip those defined inside a contract (they appear on the
        // contract page, not their own page).
        if contract.is_some() && !matches!(item_id, hir::ItemId::Contract(_)) {
            continue;
        }

        if let Some((abs, rel)) = source_paths(gcx, source, root) {
            if !allowed_sources.contains(&abs) {
                continue;
            }
            let out_dir = rel.parent().unwrap_or(Path::new("")).to_owned();
            let page = out_dir.join(format!("{prefix}.{}.mdx", name.as_str()));
            map.insert(name.as_str().to_string(), page);
        }
    }

    map
}

fn source_paths(gcx: Gcx<'_>, source_id: hir::SourceId, root: &Path) -> Option<(PathBuf, PathBuf)> {
    let file = &gcx.hir.source(source_id).file;
    if let FileName::Real(p) = &file.name {
        let rel = p.strip_prefix(root).unwrap_or(p).to_owned();
        Some((p.clone(), rel))
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
/// `contract_id` (the current contract). Walks the linearized bases to find a
/// matching function and returns its natspec if found.
///
/// When `param_types` is `Some`, the resolver prefers a function whose parameter
/// type signature matches exactly; this disambiguates overloads. If no exact
/// signature match is found, it falls back to the first name match.
pub fn resolve_inheritdoc(
    gcx: Gcx<'_>,
    contract_id: hir::ContractId,
    fn_name: &str,
    base_name: &str,
    param_types: Option<&[String]>,
) -> Option<InheritedDoc> {
    let contract = gcx.hir.contract(contract_id);

    // Find the named base contract in the linearized hierarchy.
    let base_id = contract
        .linearized_bases
        .iter()
        .copied()
        .find(|&bid| gcx.hir.contract(bid).name.as_str() == base_name)?;

    // Collect all name-matching candidates.
    let base_contract = gcx.hir.contract(base_id);
    let mut name_matches: Vec<hir::FunctionId> = Vec::new();
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
                name_matches.push(fid);
            }
        }
    }

    // Prefer an exact signature match when overloads exist.
    if let Some(want) = param_types
        && name_matches.len() > 1
    {
        for &fid in &name_matches {
            if let Some(got) = function_param_types(gcx, fid)
                && got.len() == want.len()
                && got.iter().zip(want).all(|(a, b)| a == b)
            {
                return extract_inherited_doc(gcx, fid);
            }
        }
    }

    name_matches.first().copied().and_then(|fid| extract_inherited_doc(gcx, fid))
}

/// Extract the parameter type strings (in source order) for a function.
///
/// Returns `None` if the function's AST item cannot be located.
fn function_param_types(gcx: Gcx<'_>, fid: hir::FunctionId) -> Option<Vec<String>> {
    let f = gcx.hir.function(fid);
    let ast_source = gcx.sources.get(f.source)?;
    let ast = ast_source.ast.as_ref()?;
    let fn_span = f.span;

    let params = ast.items.iter().find_map(|item| match &item.kind {
        solar::ast::ItemKind::Function(func) if item.span == fn_span => {
            Some(&func.header.parameters)
        }
        solar::ast::ItemKind::Contract(c) => c.body.iter().find_map(|m| match &m.kind {
            solar::ast::ItemKind::Function(func) if m.span == fn_span => {
                Some(&func.header.parameters)
            }
            _ => None,
        }),
        _ => None,
    })?;

    Some(parameter_type_strings(gcx, params))
}

/// Compute the parameter type strings of a [`ParameterList`] from the source map.
pub fn parameter_type_strings(gcx: Gcx<'_>, params: &ParameterList<'_>) -> Vec<String> {
    let sm = gcx.sess.source_map();
    params
        .vars
        .iter()
        .map(|v| {
            sm.span_to_snippet(v.ty.span)
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
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
/// vocs uses root-relative links (starting with `/`). Forward slashes are
/// always used so the URL stays correct on Windows.
fn page_link(page: &Path, _current_page: &Path) -> String {
    use path_slash::PathExt;
    // Strip .mdx extension and produce an absolute path from the pages root.
    let without_ext = page.with_extension("");
    format!("/{}", without_ext.to_slash_lossy())
}
