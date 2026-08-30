//! HIR-aware enrichments.
//!
//! Pure functions over `solar`'s HIR:
//! * `build_name_to_page`: maps contract names to their MDX page paths.
//! * `inheritance_links`: `**Inherits:**` line for a contract page.
//! * `natspec_doc`: resolves effective NatSpec for a callable item.
//! * `replace_inline_links`: rewrites `{Ident}` to markdown links.

use path_slash::PathBufExt;
use solar::{
    ast::{
        ContractKind, DataLocation, FunctionKind, ItemKind, NatSpecItem, NatSpecKind, Visibility,
    },
    interface::{Span, source_map::FileName},
    sema::{
        Gcx,
        hir::{ContractId, FunctionId, ItemId, SourceId, VariableId},
        ty::{Ty, TyAbiPrinter, TyAbiPrinterMode},
    },
};
use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
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

/// One rendered row of a public variable getter signature (its name, ABI type and the
/// inherited description).
pub struct GetterField {
    pub name: Option<String>,
    pub ty: String,
    pub description: String,
}

/// Effective NatSpec for an item, resolved by Solar and aligned with the item's callable
/// signature.
pub struct NatSpecDoc {
    pub notices: Vec<String>,
    pub devs: Vec<String>,
    pub params: Vec<String>,
    pub returns: Vec<String>,
    pub getter_params: Vec<GetterField>,
    pub getter_returns: Vec<GetterField>,
}

/// Resolves explicit NatSpec inheritance, or Foundry's conservative implicit inheritance policy.
pub fn natspec_doc(gcx: Gcx<'_>, item: ItemId, implicit: bool) -> Option<NatSpecDoc> {
    let mut doc = effective_natspec_doc(gcx, item, implicit, &mut HashSet::new())?;
    let callable = callable_function(gcx, item);
    if let Some(fid) = callable
        && matches!(item, ItemId::Variable(_))
    {
        let function = gcx.hir.function(fid);
        doc.getter_params = function
            .parameters
            .iter()
            .enumerate()
            .map(|(index, &parameter)| getter_field(gcx, parameter, &doc.params[index]))
            .collect();
        doc.getter_returns = function
            .returns
            .iter()
            .enumerate()
            .map(|(index, &return_)| getter_field(gcx, return_, &doc.returns[index]))
            .collect();
    }
    Some(doc)
}

fn effective_natspec_doc(
    gcx: Gcx<'_>,
    item: ItemId,
    implicit: bool,
    visited: &mut HashSet<ItemId>,
) -> Option<NatSpecDoc> {
    if !visited.insert(item) {
        return None;
    }

    let hir_item = gcx.hir.item(item);
    let raw = gcx.hir.doc(hir_item.doc()).ast_comments();
    let has_local = raw.iter().any(|comment| !comment.natspec.is_empty());
    if !has_local {
        if !implicit {
            return Some(empty_natspec_doc(gcx, item));
        }
        let bases = direct_base_items(gcx, item);
        let [base] = bases.as_slice() else { return None };
        if !implicit_edge_compatible(gcx, item, *base)
            || (!matches!(item, ItemId::Variable(_)) && !parameter_names_equal(gcx, item, *base))
        {
            return None;
        }
        return effective_natspec_doc(gcx, *base, true, visited);
    }

    let mut local = doc_from_view(gcx, item);
    let mut inheritdoc = None;
    let mut local_notice = false;
    let mut local_dev = false;
    let mut local_param = false;
    let mut local_return = false;
    let raw_items =
        raw.iter().flat_map(|comment| comment.natspec.iter().copied()).collect::<Vec<_>>();
    for entry in &raw_items {
        match entry.kind {
            NatSpecKind::Inheritdoc { contract } if inheritdoc.is_none() => {
                inheritdoc = Some(contract)
            }
            NatSpecKind::Notice if continuation_parent(gcx, &raw_items, entry).is_none() => {
                local_notice = true
            }
            NatSpecKind::Dev => local_dev = true,
            NatSpecKind::Param { .. } => local_param = true,
            NatSpecKind::Return { .. } => local_return = true,
            _ => {}
        }
    }
    let Some(alias) = inheritdoc else { return Some(local) };
    let contract = gcx.natspec_contract(alias.name, hir_item.source())?;
    let source = exact_override_item(gcx, item, contract, &mut HashSet::new())?;
    let inherited = effective_natspec_doc(gcx, source, true, visited)?;

    // Solar handles ordinary explicit inheritance, including positional remapping. The
    // recursive source fills only sections that Solar cannot see because they depend on
    // Foundry's implicit policy at the exact source declaration.
    if !local_notice {
        local.notices = inherited.notices;
    }
    if !local_dev {
        local.devs = inherited.devs;
    }
    if !local_param {
        for (description, inherited) in local.params.iter_mut().zip(inherited.params) {
            *description = inherited;
        }
    }
    if !local_return {
        for (description, inherited) in local.returns.iter_mut().zip(inherited.returns) {
            *description = inherited;
        }
    }
    Some(local)
}

fn empty_natspec_doc(gcx: Gcx<'_>, item: ItemId) -> NatSpecDoc {
    let (params, returns) = callable_function(gcx, item)
        .map(|id| {
            let function = gcx.hir.function(id);
            (
                vec![String::new(); function.parameters.len()],
                vec![String::new(); function.returns.len()],
            )
        })
        .unwrap_or_default();
    NatSpecDoc {
        notices: Vec::new(),
        devs: Vec::new(),
        params,
        returns,
        getter_params: Vec::new(),
        getter_returns: Vec::new(),
    }
}

fn doc_from_view(gcx: Gcx<'_>, item: ItemId) -> NatSpecDoc {
    let view = gcx.natspec_view(item);
    let mut doc = empty_natspec_doc(gcx, item);
    for natspec in view.items() {
        if continuation_parent(gcx, view.items(), natspec).is_some() {
            continue;
        }
        let content = positional_description(gcx, view.items(), std::slice::from_ref(natspec));
        match natspec.kind {
            NatSpecKind::Notice => doc.notices.push(content),
            NatSpecKind::Dev => doc.devs.push(content),
            _ => {}
        }
    }
    if let Some(fid) = callable_function(gcx, item) {
        let function = gcx.hir.function(fid);
        for (index, param) in doc.params[..function.parameters.len()].iter_mut().enumerate() {
            *param = positional_description(gcx, view.items(), view.parameter(index));
        }
        for (index, return_) in doc.returns[..function.returns.len()].iter_mut().enumerate() {
            *return_ = positional_description(gcx, view.items(), view.return_(index));
        }
    }
    doc
}

fn positional_description(
    gcx: Gcx<'_>,
    all_items: &[NatSpecItem],
    items: &[NatSpecItem],
) -> String {
    items
        .iter()
        .map(|item| {
            let mut content = normalized_natspec_content(gcx, item);
            for continuation in all_items.iter().filter(|candidate| {
                continuation_parent(gcx, all_items, candidate) == Some(Some(item.span))
            }) {
                content.push('\n');
                content.push_str(&normalized_natspec_content(gcx, continuation));
            }
            content
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalized_natspec_content(gcx: Gcx<'_>, item: &NatSpecItem) -> String {
    let source_map = gcx.sess.source_map();
    let snippet = source_map.span_to_snippet(item.span).ok();
    if snippet.as_deref().is_some_and(|snippet| snippet.starts_with("///")) {
        return item.content().trim().to_string();
    }
    if snippet.as_deref().is_some_and(|snippet| snippet.starts_with("/**")) {
        return clean_block_doc_content(item.content()).trim().to_string();
    }
    item.content()
        .lines()
        .enumerate()
        .map(
            |(index, line)| {
                if index == 0 { line.to_string() } else { clean_block_doc_content(line) }
            },
        )
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Returns the original tagged parent of a synthetic line-comment notice. The outer `Option`
/// distinguishes a continuation from a standalone untagged notice; the inner value is `None` when
/// Solar's resolved view omitted the parent because a local section replaced it.
fn continuation_parent(
    gcx: Gcx<'_>,
    all_items: &[NatSpecItem],
    item: &NatSpecItem,
) -> Option<Option<Span>> {
    if !matches!(item.kind, NatSpecKind::Notice)
        || !gcx.sess.source_map().span_to_snippet(item.span).ok()?.starts_with("///")
    {
        return None;
    }

    let source_map = gcx.sess.source_map();
    let location = source_map.lookup_char_pos(item.span.lo());
    let mut previous_line = location.line.checked_sub(2)?;
    loop {
        let line = location.file.get_line(previous_line)?.trim_start();
        let content = line.strip_prefix("///")?;
        if content.trim().is_empty() {
            return None;
        }
        let Some(tag) = content.trim_start().strip_prefix('@') else {
            previous_line = previous_line.checked_sub(1)?;
            continue;
        };
        if !matches!(tag.split_whitespace().next(), Some("notice" | "dev" | "param" | "return")) {
            return None;
        }
        return Some(
            all_items
                .iter()
                .find(|candidate| {
                    let candidate_location = source_map.lookup_char_pos(candidate.span.lo());
                    candidate_location.line == previous_line + 1
                        && std::sync::Arc::ptr_eq(&candidate_location.file, &location.file)
                })
                .map(|parent| parent.span),
        );
    }
}

fn getter_field(gcx: Gcx<'_>, variable: VariableId, description: &str) -> GetterField {
    GetterField {
        name: gcx.hir.variable(variable).name.map(|name| name.as_str().to_string()),
        ty: render_ty(gcx, gcx.type_of_item(variable.into())),
        description: description.to_string(),
    }
}

/// Renders a resolved type as its ABI signature string.
fn render_ty<'a>(gcx: Gcx<'a>, ty: Ty<'a>) -> String {
    let mut out = String::new();
    let _ = TyAbiPrinter::new(gcx, &mut out, TyAbiPrinterMode::Signature).print(ty);
    out
}

/// Extracts function parameter types exactly as spelled in source for link anchors.
fn function_source_param_types(gcx: Gcx<'_>, fid: FunctionId) -> Option<Vec<String>> {
    let f = gcx.hir.function(fid);
    let ast = gcx.sources.get(f.source)?.ast.as_ref()?;
    let params = ast.items.iter().find_map(|item| match &item.kind {
        ItemKind::Function(func) if item.span == f.span => Some(&func.header.parameters),
        ItemKind::Contract(c) => c.body.iter().find_map(|member| match &member.kind {
            ItemKind::Function(func) if member.span == f.span => Some(&func.header.parameters),
            _ => None,
        }),
        _ => None,
    })?;
    let sm = gcx.sess.source_map();
    Some(
        params
            .vars
            .iter()
            .map(|variable| {
                sm.span_to_snippet(variable.ty.span).unwrap_or_default().trim().to_string()
            })
            .collect(),
    )
}

/// Canonicalizes Solidity type aliases for generated link anchors.
fn normalize_sol_type(t: &str) -> String {
    let bytes = t.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + 8);
    let mut i = 0;
    while i < len {
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

fn callable_function(gcx: Gcx<'_>, item: ItemId) -> Option<FunctionId> {
    match item {
        ItemId::Function(id) => Some(id),
        ItemId::Variable(id) => gcx.hir.variable(id).getter,
        _ => None,
    }
}

fn exact_override_item(
    gcx: Gcx<'_>,
    item: ItemId,
    owner: ContractId,
    visited: &mut HashSet<ItemId>,
) -> Option<ItemId> {
    if !visited.insert(item) {
        return None;
    }
    if gcx.hir.item(item).contract() == Some(owner) {
        return Some(item);
    }
    for &base in gcx.base_override_items(item) {
        if let Some(found) = exact_override_item(gcx, base, owner, visited) {
            return Some(found);
        }
    }
    None
}

fn direct_base_items(gcx: Gcx<'_>, item: ItemId) -> Vec<ItemId> {
    let mut bases = Vec::new();
    let mut visited = HashSet::new();
    for &base in gcx.base_override_items(item) {
        push_non_yul_base_items(gcx, base, &mut bases, &mut visited);
    }
    bases
}

fn push_non_yul_base_items(
    gcx: Gcx<'_>,
    item: ItemId,
    bases: &mut Vec<ItemId>,
    visited: &mut HashSet<ItemId>,
) {
    if !visited.insert(item) {
        return;
    }
    if matches!(item, ItemId::Function(id) if gcx.hir.function(id).is_yul) {
        for &base in gcx.base_override_items(item) {
            push_non_yul_base_items(gcx, base, bases, visited);
        }
    } else {
        bases.push(item);
    }
}

fn parameter_names_equal(gcx: Gcx<'_>, target: ItemId, base: ItemId) -> bool {
    let names = |item| {
        callable_function(gcx, item).map(|id| {
            gcx.hir
                .function(id)
                .parameters
                .iter()
                .map(|&id| gcx.hir.variable(id).name.map(|name| name.name))
                .collect::<Vec<_>>()
        })
    };
    names(target) == names(base)
}

fn implicit_edge_compatible(gcx: Gcx<'_>, target: ItemId, base: ItemId) -> bool {
    let (Some(target_id), Some(base_id)) =
        (callable_function(gcx, target), callable_function(gcx, base))
    else {
        return false;
    };
    let target_fn = gcx.hir.function(target_id);
    let base_fn = gcx.hir.function(base_id);
    if !base_fn.virtual_
        || target_fn.visibility == Visibility::Private
        || base_fn.visibility == Visibility::Private
        || (target_fn.body.is_none() && base_fn.body.is_some())
    {
        return false;
    }
    let visibility_ok = if matches!(target, ItemId::Variable(_)) {
        base_fn.visibility == Visibility::External
    } else {
        target_fn.visibility == base_fn.visibility
            || (base_fn.visibility == Visibility::External
                && target_fn.visibility == Visibility::Public)
    };
    let target_mutability = match target {
        ItemId::Variable(id) if gcx.hir.variable(id).is_constant() => {
            solar::ast::StateMutability::Pure
        }
        _ => target_fn.state_mutability,
    };
    let mutability_ok = target_mutability == base_fn.state_mutability
        || matches!(
            (target_mutability, base_fn.state_mutability),
            (
                solar::ast::StateMutability::Pure,
                solar::ast::StateMutability::View | solar::ast::StateMutability::NonPayable
            ) | (solar::ast::StateMutability::View, solar::ast::StateMutability::NonPayable)
        );
    if !visibility_ok || !mutability_ok {
        return false;
    }
    let types_equal = |left: &[VariableId], right: &[VariableId], locations: bool| {
        left.len() == right.len()
            && left.iter().zip(right).all(|(&left, &right)| {
                let left = gcx.type_of_item(left.into());
                let right = gcx.type_of_item(right.into());
                left.peel_refs() == right.peel_refs() && (!locations || left.loc() == right.loc())
            })
    };
    let external_types_equal = |left: &[VariableId], right: &[VariableId]| {
        let normalize = |location| match location {
            Some(DataLocation::Calldata) => Some(DataLocation::Memory),
            location => location,
        };
        left.len() == right.len()
            && left.iter().zip(right).all(|(&left, &right)| {
                let left_variable = gcx.hir.variable(left);
                let right_variable = gcx.hir.variable(right);
                let left = gcx.type_of_item(left.into());
                let right = gcx.type_of_item(right.into());
                let left_location = left_variable.data_location.or_else(|| left.loc());
                let right_location = right_variable.data_location.or_else(|| right.loc());
                left.peel_refs() == right.peel_refs()
                    && normalize(left_location) == normalize(right_location)
            })
    };
    let target_returns =
        gcx.type_of_item(target_id.into()).as_externally_callable_function(false, gcx).returns();
    let base_returns =
        gcx.type_of_item(base_id.into()).as_externally_callable_function(false, gcx).returns();
    if target_returns != base_returns || !external_types_equal(target_fn.returns, base_fn.returns) {
        return false;
    }
    if target_fn.kind == FunctionKind::Modifier {
        return types_equal(target_fn.parameters, base_fn.parameters, false);
    }
    if base_fn.visibility == Visibility::External {
        external_types_equal(target_fn.parameters, base_fn.parameters)
    } else {
        types_equal(target_fn.parameters, base_fn.parameters, true)
            && types_equal(target_fn.returns, base_fn.returns, true)
    }
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
    /// Heading and exact signature anchors rendered on the current page.
    anchors: HashSet<String>,
    /// Effective inherited member names and their optional documentation pages.
    inherited: HashMap<String, Option<PathBuf>>,
    /// Inherited contracts and the members rendered on their exact pages.
    inherited_contracts: HashMap<String, InheritedContract>,
}

#[derive(Debug)]
enum InheritedContract {
    Unique { id: ContractId, page: Option<PathBuf>, anchors: HashSet<String> },
    Ambiguous,
}

/// Record the heading and exact signature anchor for a rendered Solidity function.
fn insert_function_anchors(
    gcx: Gcx<'_>,
    id: FunctionId,
    anchors: &mut HashSet<String>,
) -> Option<String> {
    let function = gcx.hir.function(id);
    if function.is_yul || function.is_getter() {
        return None;
    }
    let params = function_source_param_types(gcx, id)?;
    let name = match function.kind {
        FunctionKind::Constructor => "constructor".to_string(),
        FunctionKind::Fallback => "fallback".to_string(),
        FunctionKind::Receive => "receive".to_string(),
        FunctionKind::Function | FunctionKind::Modifier => function.name?.as_str().to_string(),
    };
    anchors.insert(slug_anchor_segment(&name));
    anchors.insert(function_signature_anchor(&name, &params));
    Some(name)
}

impl LocalMembers {
    /// Create an empty member set for the contract `name`.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            members: HashSet::new(),
            anchors: HashSet::new(),
            inherited: HashMap::new(),
            inherited_contracts: HashMap::new(),
        }
    }

    /// Create a member set populated with members declared by base contracts.
    pub fn for_contract(gcx: Gcx<'_>, contract_id: ContractId, name_to_page: &NameToPage) -> Self {
        let contract = gcx.hir.contract(contract_id);
        let mut this = Self::new(contract.name.as_str());

        // Solidity's linearization lists the current contract first, followed by bases in
        // resolution order. Reserve the first inherited declaration even if its page is not
        // rendered so a farther declaration cannot produce a confidently incorrect link.
        for &base_id in contract.linearized_bases.iter().filter(|&&id| id != contract_id) {
            let base = gcx.hir.contract(base_id);
            let page = name_to_page.get_contract(base_id).cloned();
            let mut anchors = HashSet::new();

            for &item_id in base.items {
                let (name, is_inherited) = match item_id {
                    ItemId::Function(id) => {
                        let function = gcx.hir.function(id);
                        (
                            insert_function_anchors(gcx, id, &mut anchors),
                            function.visibility != Visibility::Private
                                && function.kind != FunctionKind::Constructor,
                        )
                    }
                    ItemId::Variable(id) => {
                        let variable = gcx.hir.variable(id);
                        (
                            variable.name.map(|name| name.as_str().to_string()),
                            variable.visibility != Some(Visibility::Private),
                        )
                    }
                    ItemId::Struct(id) => {
                        (Some(gcx.hir.strukt(id).name.as_str().to_string()), true)
                    }
                    ItemId::Enum(id) => (Some(gcx.hir.enumm(id).name.as_str().to_string()), true),
                    ItemId::Error(id) => (Some(gcx.hir.error(id).name.as_str().to_string()), true),
                    ItemId::Event(id) => (Some(gcx.hir.event(id).name.as_str().to_string()), true),
                    ItemId::Udvt(id) => (Some(gcx.hir.udvt(id).name.as_str().to_string()), true),
                    ItemId::Contract(_) => (None, false),
                };
                if let Some(name) = name {
                    anchors.insert(slug_anchor_segment(&name));
                    if is_inherited {
                        this.inherited.entry(name).or_insert_with(|| page.clone());
                    }
                }
            }

            match this.inherited_contracts.entry(base.name.as_str().to_string()) {
                Entry::Vacant(entry) => {
                    entry.insert(InheritedContract::Unique { id: base_id, page, anchors });
                }
                Entry::Occupied(mut entry) => {
                    if matches!(entry.get(), InheritedContract::Unique { id, .. } if *id != base_id)
                    {
                        entry.insert(InheritedContract::Ambiguous);
                    }
                }
            }
        }

        this
    }

    /// Record a member that is rendered as a `### member` heading on the page.
    pub fn insert(&mut self, member: &str) {
        self.members.insert(member.to_string());
        self.anchors.insert(slug_anchor_segment(member));
    }

    /// Record an exact signature anchor rendered on the current page.
    pub fn insert_anchor(&mut self, anchor: String) {
        self.anchors.insert(anchor);
    }

    /// Anchor for a bare `{member}` reference, if `member` is documented on this page.
    ///
    /// Overloads share the base heading slug; the first heading owns it.
    fn member_anchor(&self, member: &str) -> Option<String> {
        self.members.contains(member).then(|| slug_anchor_segment(member))
    }

    /// Anchor for a qualified `{Contract-member[-params...]}` reference, if `member` is
    /// documented on this page.
    fn xref_member_anchor(&self, part: &str) -> Option<String> {
        let anchor = xref_part_anchor(part);
        self.anchors.contains(&anchor).then_some(anchor)
    }

    /// Page and anchor for a bare inherited-member reference.
    ///
    /// The outer option indicates whether the name is inherited; the inner option is absent when
    /// the effective declaration has no rendered page.
    fn inherited_member_link(&self, member: &str, current_page: &Path) -> Option<Option<String>> {
        let page = self.inherited.get(member)?;
        Some(page.as_ref().map(|page| {
            format!("{}#{}", page_link(page, current_page), slug_anchor_segment(member))
        }))
    }

    /// Exact page and anchor for a qualified inherited-contract member reference.
    ///
    /// The outer option indicates whether the contract is an inherited base; the inner option is
    /// absent when that base has no rendered page or the named member has no rendered heading.
    fn inherited_contract_member_link(
        &self,
        contract: &str,
        part: &str,
        current_page: &Path,
    ) -> Option<Option<String>> {
        let base = self.inherited_contracts.get(contract)?;
        let InheritedContract::Unique { page, anchors, .. } = base else {
            return Some(None);
        };
        let anchor = xref_part_anchor(part);
        Some(page.as_ref().and_then(|page| {
            anchors.contains(&anchor).then(|| format!("{}#{anchor}", page_link(page, current_page)))
        }))
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
                    let local_anchor = match part {
                        None => local.member_anchor(lookup_name).map(Some),
                        Some(member) if lookup_name == local.name => {
                            Some(local.xref_member_anchor(member))
                        }
                        Some(_) => None,
                    };
                    if let Some(anchor) = local_anchor {
                        if let Some(anchor) = anchor {
                            let default_display = match part {
                                Some(member) => format!("{lookup_name}.{member}"),
                                None => lookup_name.to_string(),
                            };
                            let display = escape_link_label(label.unwrap_or(&default_display));
                            out.push_str(&format!("[{display}](#{anchor})"));
                        } else {
                            let safe_name = lookup_name.replace('`', "'");
                            out.push_str(&format!("`{safe_name}`"));
                        }
                        i += end;
                        continue;
                    }

                    let inherited_link = match part {
                        None => local.inherited_member_link(lookup_name, current_page),
                        Some(member) => {
                            local.inherited_contract_member_link(lookup_name, member, current_page)
                        }
                    };
                    if let Some(link) = inherited_link {
                        if let Some(link) = link {
                            let default_display = match part {
                                Some(member) => format!("{lookup_name}.{member}"),
                                None => lookup_name.to_string(),
                            };
                            let display = escape_link_label(label.unwrap_or(&default_display));
                            out.push_str(&format!("[{display}]({link})"));
                        } else {
                            let safe_name = lookup_name.replace('`', "'");
                            out.push_str(&format!("`{safe_name}`"));
                        }
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
        } else if ch != '$' && !last_was_dash && !out.is_empty() {
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

        // Unknown qualified self-reference should not create a broken same-page anchor.
        let out = replace_inline_links(
            "See {ECDSA-doesNotExist}.",
            &name_to_page,
            Path::new("src/library.ECDSA.mdx"),
            Some(&local),
        );
        assert_eq!(out, "See `ECDSA`.");
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
