//! HIR-aware enrichments.
//!
//! Pure functions over `solar`'s HIR:
//! * `build_name_to_page`: maps contract names to their MDX page paths.
//! * `inheritance_links`: `**Inherits:**` line for a contract page.
//! * `resolve_inheritdoc`: pulls natspec from a base contract member.
//! * `replace_inline_links`: rewrites `{Ident}` to markdown links.

use path_slash::PathBufExt;
use solar::{
    ast::{CommentKind, ContractKind, DocComments, FunctionKind, ItemKind, NatSpecKind},
    interface::source_map::FileName,
    sema::{
        Gcx,
        hir::{ContractId, FunctionId, ItemId, SourceId, VariableId},
        ty::{TyAbiPrinter, TyAbiPrinterMode},
    },
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use tracing::warn;

// ── name-to-page map ──────────────────────────────────────────────────────────

/// Maps Solidity identifiers and HIR ids to their output MDX page paths
/// relative to `pages/`.
#[derive(Debug, Default)]
pub struct NameToPage {
    by_name: HashMap<String, Vec<PathBuf>>,
    by_contract: HashMap<ContractId, PathBuf>,
}

impl NameToPage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Candidate pages defined for a top-level identifier, if any.
    pub fn get(&self, name: &str) -> Option<&Vec<PathBuf>> {
        self.by_name.get(name)
    }

    /// Exact page for a contract id, if it lives in an allowed source.
    pub fn get_contract(&self, id: ContractId) -> Option<&PathBuf> {
        self.by_contract.get(&id)
    }
}

/// Build the [`NameToPage`] index from HIR by re-deriving each item's output path.
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

    // Collect and sort by (source_path, name) so that last-insert-wins is deterministic
    // across platforms even when the HIR iteration order is unspecified.
    let mut item_ids: Vec<_> = gcx.hir.item_ids().collect();
    item_ids.sort_by_key(|id| {
        let (name, source) = match id {
            ItemId::Contract(id) => {
                let c = gcx.hir.contract(*id);
                (c.name.as_str().to_string(), c.source)
            }
            ItemId::Struct(id) => {
                let s = gcx.hir.strukt(*id);
                (s.name.as_str().to_string(), s.source)
            }
            ItemId::Enum(id) => {
                let e = gcx.hir.enumm(*id);
                (e.name.as_str().to_string(), e.source)
            }
            ItemId::Error(id) => {
                let e = gcx.hir.error(*id);
                (e.name.as_str().to_string(), e.source)
            }
            ItemId::Event(id) => {
                let e = gcx.hir.event(*id);
                (e.name.as_str().to_string(), e.source)
            }
            ItemId::Udvt(id) => {
                let u = gcx.hir.udvt(*id);
                (u.name.as_str().to_string(), u.source)
            }
            ItemId::Function(_) | ItemId::Variable(_) => {
                return (String::new(), String::new());
            }
        };
        let path = source_paths(gcx, source, root)
            .map(|(_, rel)| rel.to_string_lossy().into_owned())
            .unwrap_or_default();
        (path, name)
    });

    for item_id in item_ids {
        let (name, source, contract, prefix) = match item_id {
            ItemId::Contract(id) => {
                let c = gcx.hir.contract(id);
                let kind = match c.kind {
                    ContractKind::Contract => "contract",
                    ContractKind::AbstractContract => "abstract",
                    ContractKind::Interface => "interface",
                    ContractKind::Library => "library",
                };
                (c.name, c.source, None, kind)
            }
            ItemId::Struct(id) => {
                let s = gcx.hir.strukt(id);
                (s.name, s.source, s.contract, "struct")
            }
            ItemId::Enum(id) => {
                let e = gcx.hir.enumm(id);
                (e.name, e.source, e.contract, "enum")
            }
            ItemId::Error(id) => {
                let e = gcx.hir.error(id);
                (e.name, e.source, e.contract, "error")
            }
            ItemId::Event(id) => {
                let e = gcx.hir.event(id);
                (e.name, e.source, e.contract, "event")
            }
            ItemId::Udvt(id) => {
                let u = gcx.hir.udvt(id);
                (u.name, u.source, u.contract, "type")
            }
            ItemId::Function(_) | ItemId::Variable(_) => continue,
        };

        // For non-contract items, skip those defined inside a contract (they appear on the
        // contract page, not their own page).
        if contract.is_some() && !matches!(item_id, ItemId::Contract(_)) {
            continue;
        }

        if let Some((abs, rel)) = source_paths(gcx, source, root) {
            if !allowed_sources.contains(&abs) {
                continue;
            }
            let out_dir = rel.parent().unwrap_or(Path::new("")).to_owned();
            let page = out_dir.join(format!("{prefix}.{}.mdx", name.as_str()));
            let name_str = name.as_str().to_string();
            let entry = map.by_name.entry(name_str.clone()).or_default();
            if !entry.is_empty() {
                warn!(
                    "forge doc: duplicate top-level name `{name_str}`; \
                     cross-reference `{{{name_str}}}` will resolve by proximity to the referencing page"
                );
            }
            entry.push(page.clone());

            // Record exact contract -> page so inheritance / id-keyed lookups
            // don't go through the ambiguous name index.
            if let ItemId::Contract(cid) = item_id {
                map.by_contract.insert(cid, page);
            }
        }
    }

    map
}

fn source_paths(gcx: Gcx<'_>, source_id: SourceId, root: &Path) -> Option<(PathBuf, PathBuf)> {
    let file = &gcx.hir.source(source_id).file;
    if let FileName::Real(p) = &file.name {
        let rel = if let Ok(r) = p.strip_prefix(root) {
            r.to_path_buf()
        } else {
            // Outside-root files (e.g. absolute lib paths) get a synthetic
            // `lib/<tail>` path that matches what builder.rs emits.
            let comps: Vec<_> = p.components().collect();
            let start = comps.len().saturating_sub(3);
            let tail: PathBuf = comps[start..].iter().collect();
            PathBuf::from("lib").join(tail)
        };
        Some((p.clone(), rel))
    } else {
        None
    }
}

/// Pick the best candidate page for a given cross-reference lookup.
///
/// When only one candidate exists the choice is trivial. When multiple files define
/// the same top-level name the page whose *directory* shares the longest common
/// path prefix with `current_page` wins; ties fall back to the first entry (which
/// is deterministic because `build_name_to_page` sorts before inserting).
fn resolve_page<'a>(candidates: &'a [PathBuf], current_page: &Path) -> &'a PathBuf {
    if candidates.len() == 1 {
        return &candidates[0];
    }
    let current_dir = current_page.parent().unwrap_or(Path::new(""));
    candidates
        .iter()
        .max_by_key(|page| {
            let page_dir = page.parent().unwrap_or(Path::new(""));
            current_dir.components().zip(page_dir.components()).take_while(|(a, b)| a == b).count()
        })
        .unwrap_or(&candidates[0])
}

// ── inheritance links ─────────────────────────────────────────────────────────

/// Returns the `**Inherits:**` markdown string for a contract, or `None` if it has no bases.
///
/// Each base is either a bare name (when no page is known) or a markdown link.
pub fn inheritance_links(
    gcx: Gcx<'_>,
    contract_id: ContractId,
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
            // Prefer the exact base id; only fall back to the ambiguous name
            // index when the base has no rendered page of its own.
            if let Some(page) = name_to_page.get_contract(base_id) {
                let link = page_link(page, current_page);
                format!("[{name}]({link})")
            } else if let Some(candidates) = name_to_page.get(name) {
                let page = resolve_page(candidates, current_page);
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
/// type signature matches exactly; this disambiguates overloads. Ambiguous
/// overloads are never matched by name alone.
pub fn resolve_inheritdoc(
    gcx: Gcx<'_>,
    contract_id: ContractId,
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

    // Search the named base and then its own linearized chain so that `@inheritdoc Base`
    // resolves even when `Base` itself inherits the member without redeclaring NatSpec.
    // We prefer the first level that has both a name match AND non-empty documentation.
    let base_contract = gcx.hir.contract(base_id);

    let search_contracts: Vec<ContractId> = std::iter::once(base_id)
        .chain(base_contract.linearized_bases.iter().copied().filter(|&id| id != base_id))
        .collect();

    for search_id in &search_contracts {
        let search_contract = gcx.hir.contract(*search_id);
        let mut name_matches: Vec<FunctionId> = Vec::new();
        for &item_id in search_contract.items {
            if let ItemId::Function(fid) = item_id {
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
        if name_matches.is_empty() {
            continue;
        }

        // Prefer an exact signature match when overloads exist.
        if let Some(want) = param_types
            && name_matches.len() > 1
        {
            // Try to find a signature-exact match with docs; fall through only for
            // non-overloaded name matches below.
            for &fid in &name_matches {
                if let Some(got) = function_param_types(gcx, fid)
                    && got.len() == want.len()
                    && got.iter().zip(want).all(|(a, b)| a == b)
                    && let Some(doc) = extract_inherited_doc(gcx, fid)
                    && (!doc.notices.is_empty()
                        || !doc.devs.is_empty()
                        || !doc.params.is_empty()
                        || !doc.returns.is_empty())
                {
                    return Some(doc);
                }
            }
        }

        if name_matches.len() > 1 {
            continue;
        }

        // Return the first candidate that has actual documentation; if none have docs
        // at this inheritance level continue walking up the chain.
        for &fid in &name_matches {
            if let Some(doc) = extract_inherited_doc(gcx, fid)
                && (!doc.notices.is_empty()
                    || !doc.devs.is_empty()
                    || !doc.params.is_empty()
                    || !doc.returns.is_empty())
            {
                return Some(doc);
            }
        }
    }

    None
}

/// Resolve `@inheritdoc BaseContract` for a **state variable** named `var_name`
/// inside `contract_id`. Walks the linearised bases to find a matching public
/// variable and returns its natspec if found.
pub fn resolve_inheritdoc_var(
    gcx: Gcx<'_>,
    contract_id: ContractId,
    var_name: &str,
    base_name: &str,
) -> Option<InheritedDoc> {
    let contract = gcx.hir.contract(contract_id);
    let base_id = contract
        .linearized_bases
        .iter()
        .copied()
        .find(|&bid| gcx.hir.contract(bid).name.as_str() == base_name)?;

    let base_contract = gcx.hir.contract(base_id);
    let search_contracts: Vec<ContractId> = std::iter::once(base_id)
        .chain(base_contract.linearized_bases.iter().copied().filter(|&id| id != base_id))
        .collect();

    for search_id in &search_contracts {
        let search_contract = gcx.hir.contract(*search_id);
        for &item_id in search_contract.items {
            match item_id {
                ItemId::Variable(vid) => {
                    let v = gcx.hir.variable(vid);
                    if v.name.map(|n| n.as_str() == var_name).unwrap_or(false)
                        && let Some(doc) = extract_inherited_doc_var(gcx, vid)
                        && (!doc.notices.is_empty() || !doc.devs.is_empty())
                    {
                        return Some(doc);
                    }
                }
                // A public state variable can implement an interface getter declared as a
                // zero-arg function (e.g. `function totalSupply() external view returns
                // (uint256)`). Fall back to matching a same-name zero-parameter function so
                // `@inheritdoc IERC20` on `uint256 public totalSupply` picks up the
                // interface's notice/return docs.
                ItemId::Function(fid) => {
                    let f = gcx.hir.function(fid);
                    if f.name.map(|n| n.as_str() == var_name).unwrap_or(false)
                        && function_param_types(gcx, fid).map(|p| p.is_empty()).unwrap_or(false)
                        && let Some(doc) = extract_inherited_doc(gcx, fid)
                        && (!doc.notices.is_empty()
                            || !doc.devs.is_empty()
                            || !doc.returns.is_empty())
                    {
                        return Some(doc);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

fn extract_inherited_doc_var(gcx: Gcx<'_>, vid: VariableId) -> Option<InheritedDoc> {
    let v = gcx.hir.variable(vid);
    let ast_source = gcx.sources.get(v.source)?;
    let ast = ast_source.ast.as_ref()?;
    let var_span = v.span;

    let docs = ast.items.iter().find_map(|item| {
        if item.span == var_span {
            return Some(&item.docs);
        }
        if let ItemKind::Contract(c) = &item.kind {
            for member in c.body.iter() {
                if member.span == var_span {
                    return Some(&member.docs);
                }
            }
        }
        None
    })?;

    Some(collect_inherited_doc(docs))
}

/// Extract the canonical ABI parameter type strings (in source order) for a function.
///
/// Non-ABI-printable internal parameter types, such as mappings, fall back to their
/// source spelling so inherited docs can still resolve without panicking.
pub(crate) fn function_param_types(gcx: Gcx<'_>, fid: FunctionId) -> Option<Vec<String>> {
    let f = gcx.hir.function(fid);
    let source_types = gcx
        .sources
        .get(f.source)
        .and_then(|source| source.ast.as_ref())
        .and_then(|ast| {
            ast.items.iter().find_map(|item| match &item.kind {
                ItemKind::Function(func) if item.span == f.span => Some(&func.header.parameters),
                ItemKind::Contract(c) => c.body.iter().find_map(|m| match &m.kind {
                    ItemKind::Function(func) if m.span == f.span => Some(&func.header.parameters),
                    _ => None,
                }),
                _ => None,
            })
        })
        .map(|params| {
            let sm = gcx.sess.source_map();
            params
                .vars
                .iter()
                .map(|v| {
                    normalize_sol_type(sm.span_to_snippet(v.ty.span).unwrap_or_default().trim())
                })
                .collect::<Vec<_>>()
        });

    f.parameters
        .iter()
        .enumerate()
        .map(|(idx, &param)| {
            let ty = gcx.type_of_item(param.into());
            if ty.can_be_exported(gcx) {
                let mut out = String::new();
                TyAbiPrinter::new(gcx, &mut out, TyAbiPrinterMode::Signature)
                    .print(ty)
                    .expect("writing ABI signature type to a String cannot fail");
                Some(out)
            } else {
                source_types.as_ref().and_then(|types| types.get(idx).cloned())
            }
        })
        .collect()
}

/// Canonicalize Solidity type aliases for generated anchors and source fallbacks.
///
/// Replaces every occurrence of the bare alias tokens `uint` / `int` (not
/// followed by a digit) with their canonical ABI equivalents `uint256` /
/// `int256`.
fn normalize_sol_type(t: &str) -> String {
    // Walk char-by-char and replace `uint` / `int` that are not followed by a
    // digit (i.e. are bare aliases, not `uint8`, `uint256`, etc.).
    let bytes = t.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + 8);
    let mut i = 0;
    while i < len {
        // Try to match the longer alias first (`uint` before `int`) to avoid
        // a prefix match of `int` inside `uint`.
        if bytes[i..].starts_with(b"uint")
            && !bytes.get(i + 4).copied().map(|b| b.is_ascii_digit()).unwrap_or(false)
        {
            out.push_str("uint256");
            i += 4;
        } else if bytes[i..].starts_with(b"int")
            && !bytes.get(i + 3).copied().map(|b| b.is_ascii_digit()).unwrap_or(false)
        {
            out.push_str("int256");
            i += 3;
        } else if let Some(ch) = t[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn extract_inherited_doc(gcx: Gcx<'_>, fid: FunctionId) -> Option<InheritedDoc> {
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
    let mut prev_doc_was_blank = false;
    #[derive(Clone, Copy)]
    enum LastSection {
        Notice,
        Dev,
        Param,
        Return,
    }
    let mut last_section: Option<LastSection> = None;

    for doc in docs.iter() {
        if doc.natspec.is_empty() {
            prev_doc_was_blank = true;
            continue;
        }
        for item in doc.natspec.iter() {
            let raw = doc.natspec_content(item);
            // For /** */ block comments Solar preserves raw ` * ` line decorations; strip them.
            let raw: &str =
                if doc.kind == CommentKind::Block { &clean_block_doc_content(raw) } else { raw };
            // Solar emits lines without a `@` tag as synthetic @notice with leading whitespace.
            let is_continuation = matches!(item.kind, NatSpecKind::Notice)
                && raw.starts_with(|c: char| c.is_whitespace());
            let content = raw.trim().to_string();
            if content.is_empty() {
                prev_doc_was_blank = true;
                continue;
            }
            if is_continuation && !prev_doc_was_blank {
                let last: Option<&mut String> = match last_section {
                    Some(LastSection::Notice) => result.notices.last_mut(),
                    Some(LastSection::Dev) => result.devs.last_mut(),
                    Some(LastSection::Param) => result.params.last_mut().map(|(_, d)| d),
                    Some(LastSection::Return) => result.returns.last_mut().map(|(_, d)| d),
                    None => None,
                };
                if let Some(last) = last {
                    last.push('\n');
                    last.push_str(&content);
                    continue;
                }
            }
            prev_doc_was_blank = false;
            match item.kind {
                NatSpecKind::Notice => {
                    result.notices.push(content);
                    last_section = Some(LastSection::Notice);
                }
                NatSpecKind::Dev => {
                    result.devs.push(content);
                    last_section = Some(LastSection::Dev);
                }
                NatSpecKind::Param { name } => {
                    result.params.push((name.as_str().to_string(), content));
                    last_section = Some(LastSection::Param);
                }
                NatSpecKind::Return { name } => {
                    result.returns.push((
                        name.map(|name| name.as_str().to_string()).unwrap_or_default(),
                        content,
                    ));
                    last_section = Some(LastSection::Return);
                }
                _ => {}
            }
        }
    }
    result
}

/// Strip the ` * ` block-comment line decoration from each line of a `/** */` NatSpec item's
/// content. Solar preserves raw source bytes, so continuation lines look like ` * text` and blank
/// separator lines look like ` *`. This normalises them to plain text / empty lines.
pub(crate) fn clean_block_doc_content(raw: &str) -> String {
    raw.lines()
        .map(|line| {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix('*') {
                rest.strip_prefix(' ').unwrap_or(rest)
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── inline link replacement ───────────────────────────────────────────────────

/// Members of the contract page currently being rendered.
///
/// Used to resolve `{member}` and `{Contract-member}` references lexically:
/// a name that belongs to the current contract links to its heading anchor on
/// the same page instead of going through the global name index (which only
/// contains top-level items and could otherwise resolve to an unrelated page).
#[derive(Debug)]
pub struct LocalMembers {
    /// The current contract's name.
    name: String,
    /// Member names with a heading (and thus an anchor) on the current page.
    members: HashSet<String>,
}

impl LocalMembers {
    /// Create an empty member set for the contract `name`.
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), members: HashSet::new() }
    }

    /// Record a member that is rendered as a `### member` heading on the page.
    pub fn insert(&mut self, member: &str) {
        self.members.insert(member.to_string());
    }

    /// Anchor for a bare `{member}` reference, if `member` is documented on this page.
    ///
    /// Overloads share the base heading slug; the first heading owns it.
    fn member_anchor(&self, member: &str) -> Option<String> {
        self.members.contains(member).then(|| slug_anchor_segment(member))
    }
}

/// Escape a string for use as a markdown link label.
///
/// Prevents MDX from treating user-controlled NatSpec label text as JSX or
/// breaking the surrounding markdown link syntax.
fn escape_link_label(s: &str) -> String {
    s.replace('{', "&#123;").replace('<', "&lt;").replace('[', "\\[").replace(']', "\\]")
}

/// Replace `{Ident}` and `{xref-Ident}` with markdown links using `name_to_page`.
///
/// Matches the legacy pattern: `{[xref-]Ident[-part]}[label]` where `label` defaults
/// to `Ident`.
///
/// Resolution prefers lexical proximity: a reference naming a member of the current
/// contract (`{member}`, or `{Contract-member}` where `Contract` is the current
/// contract) becomes an anchor-only link within the page; everything else goes
/// through the global `name_to_page` index.
pub fn replace_inline_links(
    text: &str,
    name_to_page: &NameToPage,
    current_page: &Path,
    local: Option<&LocalMembers>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'{' {
            // Try to parse {[xref-]Ident[-part]}[optional label].
            if let Some((end, ident, part, label)) = parse_inline_link(&text[i..]) {
                // Strip the leading `xref-` prefix if present.
                let lookup_name = ident.strip_prefix("xref-").unwrap_or(ident);
                let lookup_name = if let Some(pos) = lookup_name.find('-') {
                    &lookup_name[..pos]
                } else {
                    lookup_name
                };

                // Same-contract references resolve to anchor-only links: a bare
                // `{member}` documented on this page, or `{Contract-member}` where
                // `Contract` is the contract being rendered.
                if let Some(local) = local {
                    let anchor = match part {
                        None => local.member_anchor(lookup_name),
                        Some(member) if lookup_name == local.name => Some(xref_part_anchor(member)),
                        Some(_) => None,
                    };
                    if let Some(anchor) = anchor
                        && !anchor.is_empty()
                    {
                        let default_display = match part {
                            Some(member) => format!("{lookup_name}.{member}"),
                            None => lookup_name.to_string(),
                        };
                        let display = escape_link_label(label.unwrap_or(&default_display));
                        out.push_str(&format!("[{display}](#{anchor})"));
                        i += end;
                        continue;
                    }
                }

                if let Some(candidates) = name_to_page.get(lookup_name) {
                    let page = resolve_page(candidates, current_page);
                    let mut link = page_link(page, current_page);
                    // Append the member anchor when the pattern is `{Type-member}`.
                    // Sanitize to ASCII alphanumerics and `_` only, Solidity identifiers
                    // never contain other characters, so this drops any injection attempt.
                    if let Some(member) = part {
                        let safe_member = xref_part_anchor(member);
                        if !safe_member.is_empty() {
                            link.push('#');
                            link.push_str(&safe_member);
                        }
                    }
                    let default_display = if let Some(member) = part {
                        // default display: "Type.member"
                        format!("{lookup_name}.{member}")
                    } else {
                        lookup_name.to_string()
                    };
                    let display = escape_link_label(label.unwrap_or(&default_display));
                    out.push_str(&format!("[{display}]({link})"));
                    i += end;
                    continue;
                }

                // Unresolved {Ident}, emit as inline code to avoid MDX treating it as a
                // JS expression. Strip backticks to avoid breaking the fence.
                let safe_name = lookup_name.replace('`', "'");
                out.push_str(&format!("`{safe_name}`"));
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

pub(crate) fn function_signature_anchor(name: &str, params: &[String]) -> String {
    let mut anchor = slug_anchor_segment(name);
    for param in params {
        let param = slug_anchor_segment(&normalize_sol_type(param));
        if !param.is_empty() {
            anchor.push('-');
            anchor.push_str(&param);
        }
    }
    anchor
}

fn xref_part_anchor(part: &str) -> String {
    let mut pieces = part.split('-').filter(|piece| !piece.is_empty());
    let Some(member) = pieces.next() else {
        return String::new();
    };
    let params = pieces.map(|piece| piece.to_string()).collect::<Vec<_>>();
    function_signature_anchor(member, &params)
}

fn slug_anchor_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_dash = false;

    for ch in s.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
            last_was_dash = false;
        } else if !last_was_dash && !out.is_empty() {
            out.push('-');
            last_was_dash = true;
        }
    }

    if last_was_dash {
        out.pop();
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
    let (raw_ident, raw_part) = if let Some(rest) = inner.strip_prefix("xref-") {
        if let Some(dash) = rest.find('-') {
            (&inner[..("xref-".len() + dash)], Some(&rest[dash + 1..]))
        } else {
            (inner, None)
        }
    } else if let Some(dash) = inner.find('-') {
        let candidate_ident = &inner[..dash];
        let candidate_part = &inner[dash + 1..];
        if candidate_ident.chars().all(|c| c.is_alphanumeric() || c == '_')
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
    // Strip .mdx extension and produce an absolute path from the pages root.
    let without_ext = page.with_extension("");
    format!("/{}", without_ext.to_slash_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_signature_xref_links_member_anchor() {
        let mut name_to_page = NameToPage::new();
        name_to_page
            .by_name
            .insert("ERC721".to_string(), vec![PathBuf::from("src/contract.ERC721.mdx")]);

        let out = replace_inline_links(
            "See {xref-ERC721-_safeMint-address-uint256-}.",
            &name_to_page,
            Path::new("src/contract.Child.mdx"),
            None,
        );

        assert_eq!(
            out,
            "See [ERC721._safeMint-address-uint256-](/src/contract.ERC721#_safemint-address-uint256)."
        );
    }

    #[test]
    fn same_contract_member_links_anchor_only() {
        let name_to_page = NameToPage::new();
        let mut local = LocalMembers::new("ECDSA");
        local.insert("toEthSignedMessageHash");
        local.insert("tryRecover");

        // Bare member reference -> anchor-only link.
        let out = replace_inline_links(
            "then calling {toEthSignedMessageHash} on it.",
            &name_to_page,
            Path::new("src/library.ECDSA.mdx"),
            Some(&local),
        );
        assert_eq!(out, "then calling [toEthSignedMessageHash](#toethsignedmessagehash) on it.");

        // `{Contract-member}` self-reference -> anchor-only link.
        let out = replace_inline_links(
            "Overload of {ECDSA-tryRecover} that ...",
            &name_to_page,
            Path::new("src/library.ECDSA.mdx"),
            Some(&local),
        );
        assert_eq!(out, "Overload of [ECDSA.tryRecover](#tryrecover) that ...");

        // Unknown member still falls back to inline code.
        let out = replace_inline_links(
            "See {unknownMember}.",
            &name_to_page,
            Path::new("src/library.ECDSA.mdx"),
            Some(&local),
        );
        assert_eq!(out, "See `unknownMember`.");
    }

    #[test]
    fn local_member_wins_over_global_name() {
        // A top-level item elsewhere shares the member's name; lexical
        // proximity resolves to the same-page anchor, not the other page.
        let mut name_to_page = NameToPage::new();
        name_to_page
            .by_name
            .insert("transfer".to_string(), vec![PathBuf::from("src/other/contract.transfer.mdx")]);
        let mut local = LocalMembers::new("Token");
        local.insert("transfer");

        let out = replace_inline_links(
            "Calls {transfer}.",
            &name_to_page,
            Path::new("src/contract.Token.mdx"),
            Some(&local),
        );
        assert_eq!(out, "Calls [transfer](#transfer).");

        // Without local context the global index still resolves.
        let out = replace_inline_links(
            "Calls {transfer}.",
            &name_to_page,
            Path::new("src/contract.Token.mdx"),
            None,
        );
        assert_eq!(out, "Calls [transfer](/src/other/contract.transfer).");
    }
}
