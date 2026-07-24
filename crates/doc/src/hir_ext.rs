//! HIR-aware enrichments.
//!
//! Pure functions over `solar`'s HIR:
//! * `build_name_to_page`: maps contract names to their MDX page paths.
//! * `inheritance_links`: `**Inherits:**` line for a contract page.
//! * `resolve_inheritdoc`: pulls natspec from a base contract member.
//! * `replace_inline_links`: rewrites `{Ident}` to markdown links.

use path_slash::PathBufExt;
use solar::{
    ast::{
        CommentKind, ContractKind, DocComments, FunctionKind, ItemKind, NatSpecKind, Visibility,
    },
    interface::source_map::FileName,
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

/// Collected natspec tags from an inherited base member.
pub struct InheritedDoc {
    pub notices: Vec<String>,
    pub devs: Vec<String>,
    pub params: Vec<(String, String)>,
    pub returns: Vec<(String, String)>,
    /// When inherited by a public state variable, the getter's input rows.
    pub getter_params: Vec<GetterField>,
    /// When inherited by a public state variable, the getter's output rows.
    pub getter_returns: Vec<GetterField>,
}

impl InheritedDoc {
    const fn is_renderable(&self) -> bool {
        !self.notices.is_empty()
            || !self.devs.is_empty()
            || !self.params.is_empty()
            || !self.returns.is_empty()
    }
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
    resolve_inheritdoc_source(gcx, contract_id, fn_name, base_name, param_types).map(|(_, doc)| doc)
}

fn resolve_inheritdoc_source(
    gcx: Gcx<'_>,
    contract_id: ContractId,
    fn_name: &str,
    base_name: &str,
    param_types: Option<&[String]>,
) -> Option<(FunctionId, InheritedDoc)> {
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
                    return Some((fid, doc));
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
                return Some((fid, doc));
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

// ── implicit inheritance ──────────────────────────────────────────────────────

/// Whether `fid` is the function named `fn_name` (handling the special receiver kinds).
fn function_name_matches(gcx: Gcx<'_>, fid: FunctionId, fn_name: &str) -> bool {
    let f = gcx.hir.function(fid);
    match f.kind {
        FunctionKind::Constructor => fn_name == "constructor",
        FunctionKind::Fallback => fn_name == "fallback",
        FunctionKind::Receive => fn_name == "receive",
        _ => f.name.map(|n| n.as_str() == fn_name).unwrap_or(false),
    }
}

/// The member name under which `fid` takes part in overriding: `None` for constructors and
/// unnamed functions.
fn function_member_name(gcx: Gcx<'_>, fid: FunctionId) -> Option<String> {
    let f = gcx.hir.function(fid);
    match f.kind {
        FunctionKind::Constructor => None,
        FunctionKind::Fallback => Some("fallback".to_string()),
        FunctionKind::Receive => Some("receive".to_string()),
        _ => f.name.map(|n| n.as_str().to_string()),
    }
}

/// Whether `target` and `base` have identical parameter names in order. Solidity refuses
/// automatic inheritance across an override that renames parameters.
fn param_names_match(gcx: Gcx<'_>, target: FunctionId, base: FunctionId) -> bool {
    let names = |fid: FunctionId| -> Vec<Option<String>> {
        gcx.hir
            .function(fid)
            .parameters
            .iter()
            .map(|&vid| gcx.hir.variable(vid).name.map(|n| n.as_str().to_string()))
            .collect()
    };
    names(target) == names(base)
}

/// The `@inheritdoc Base` contract name on `docs`, if any.
fn inheritdoc_base_name(docs: &DocComments<'_>) -> Option<String> {
    docs.iter().find_map(|doc| {
        doc.natspec.iter().find_map(|item| match item.kind {
            NatSpecKind::Inheritdoc { contract } => Some(contract.as_str().to_string()),
            _ => None,
        })
    })
}

/// The AST doc comments of `fid`, located by span.
fn function_doc_comments<'gcx>(gcx: Gcx<'gcx>, fid: FunctionId) -> Option<&'gcx DocComments<'gcx>> {
    let f = gcx.hir.function(fid);
    let ast = gcx.sources.get(f.source)?.ast.as_ref()?;
    ast.items.iter().find_map(|item| {
        if item.span == f.span {
            return Some(&item.docs);
        }
        if let ItemKind::Contract(c) = &item.kind {
            c.body.iter().find(|member| member.span == f.span).map(|member| &member.docs)
        } else {
            None
        }
    })
}

/// Merge base documentation into a partial local doc: a tag kind present locally wins whole,
/// a kind absent locally is copied from the base (solc's `@inheritdoc` merge rule).
fn merge_missing(local: InheritedDoc, base: InheritedDoc) -> InheritedDoc {
    fn pick<T>(local: Vec<T>, base: Vec<T>) -> Vec<T> {
        if local.is_empty() { base } else { local }
    }
    InheritedDoc {
        notices: pick(local.notices, base.notices),
        devs: pick(local.devs, base.devs),
        params: pick(local.params, base.params),
        returns: pick(local.returns, base.returns),
        getter_params: Vec::new(),
        getter_returns: Vec::new(),
    }
}

/// Resolve natspec inherited implicitly (without `@inheritdoc`) for a function named `fn_name`
/// inside `contract_id`: the function inherits the effective documentation of the single base
/// function it directly overrides, and only when their parameter names match.
pub fn resolve_implicit_inheritdoc(
    gcx: Gcx<'_>,
    _contract_id: ContractId,
    fn_name: &str,
    target: Option<FunctionId>,
) -> Option<InheritedDoc> {
    let target = target?;
    let base = single_direct_base(gcx, target, fn_name, false)?;
    if !param_names_match(gcx, target, base) {
        return None;
    }
    let mut doc = implicit_function_doc(gcx, base)?;
    remap_inherited_return_names(gcx, target, &mut doc);
    Some(doc)
}

/// The effective documentation of `fid`, resolved level by level like solc: local tags are
/// terminal, but a local `@inheritdoc` is resolved and merged so its documentation still
/// propagates downward; a member with no local NatSpec inherits from its single direct base.
fn implicit_function_doc(gcx: Gcx<'_>, fid: FunctionId) -> Option<InheritedDoc> {
    if let Some(docs) = function_doc_comments(gcx, fid)
        && docs.iter().any(|doc| !doc.natspec.is_empty())
    {
        let mut local = collect_inherited_doc(docs);
        resolve_return_names(gcx, fid, &mut local);
        if let Some(base_name) = inheritdoc_base_name(docs)
            && let Some(cid) = gcx.hir.function(fid).contract
            && let Some(fn_name) = function_member_name(gcx, fid)
        {
            let base_doc = resolve_inheritdoc_source(
                gcx,
                cid,
                &fn_name,
                &base_name,
                function_param_types(gcx, fid).as_deref(),
            );
            return match base_doc {
                Some((source, mut base_doc)) => {
                    resolve_return_names(gcx, source, &mut base_doc);
                    remap_inherited_return_names(gcx, fid, &mut base_doc);
                    Some(merge_missing(local, base_doc))
                }
                None => Some(local).filter(InheritedDoc::is_renderable),
            };
        }
        return Some(local).filter(InheritedDoc::is_renderable);
    }

    let fn_name = function_member_name(gcx, fid)?;
    let base = single_direct_base(gcx, fid, &fn_name, false)?;
    if !param_names_match(gcx, fid, base) {
        return None;
    }
    let mut doc = implicit_function_doc(gcx, base)?;
    remap_inherited_return_names(gcx, fid, &mut doc);
    Some(doc)
}

#[derive(Clone, Copy)]
struct OverrideDomain {
    kind: FunctionKind,
    external_only: bool,
}

/// The single function directly overridden by `target`, if unambiguous. Mirrors solc's
/// `baseFunctions` set: each direct-base branch contributes its nearest same-signature
/// declaration (shadowing within the branch); distinct survivors on different branches make
/// it ambiguous. `external_declared_only` restricts to external non-getter functions (the
/// public state-variable getter case).
fn single_direct_base(
    gcx: Gcx<'_>,
    target: FunctionId,
    fn_name: &str,
    external_declared_only: bool,
) -> Option<FunctionId> {
    let f = gcx.hir.function(target);
    if matches!(f.kind, FunctionKind::Constructor) {
        return None;
    }
    let contract = f.contract?;
    let want = function_param_types(gcx, target)?;
    let domain = OverrideDomain { kind: f.kind, external_only: external_declared_only };
    match direct_base_functions(gcx, contract, fn_name, &want, domain)[..] {
        [base] => Some(base),
        _ => None,
    }
}

fn direct_base_functions(
    gcx: Gcx<'_>,
    contract: ContractId,
    fn_name: &str,
    want: &[String],
    domain: OverrideDomain,
) -> Vec<FunctionId> {
    let mut visited = HashSet::new();
    let mut out = Vec::new();
    for &base in gcx.hir.contract(contract).bases {
        collect_branch_base(gcx, base, fn_name, want, domain, &mut visited, &mut out);
    }
    out
}

fn collect_branch_base(
    gcx: Gcx<'_>,
    contract: ContractId,
    fn_name: &str,
    want: &[String],
    domain: OverrideDomain,
    visited: &mut HashSet<ContractId>,
    out: &mut Vec<FunctionId>,
) {
    if !visited.insert(contract) {
        return;
    }
    let mut declared = false;
    for &item in gcx.hir.contract(contract).items {
        if let ItemId::Function(fid) = item {
            let f = gcx.hir.function(fid);
            let eligible = !f.is_yul
                && f.visibility != Visibility::Private
                && f.kind == domain.kind
                && f.gettee.is_none()
                && (!domain.external_only || f.visibility == Visibility::External);
            if eligible
                && function_name_matches(gcx, fid, fn_name)
                && function_param_types(gcx, fid).as_deref() == Some(want)
            {
                declared = true;
                if !out.contains(&fid) {
                    out.push(fid);
                }
            }
        }
    }
    if !declared {
        for &base in gcx.hir.contract(contract).bases {
            collect_branch_base(gcx, base, fn_name, want, domain, visited, out);
        }
    }
}

/// The compiler-generated getter of the public variable `var_name` in `contract_id`, or
/// `None` when it has none (not public).
fn variable_getter(gcx: Gcx<'_>, contract_id: ContractId, var_name: &str) -> Option<FunctionId> {
    for &item_id in gcx.hir.contract(contract_id).items {
        if let ItemId::Variable(vid) = item_id {
            let v = gcx.hir.variable(vid);
            if v.name.map(|n| n.as_str() == var_name).unwrap_or(false) {
                return v.getter;
            }
        }
    }
    None
}

/// Resolve natspec inherited implicitly for a public state variable: the source is the single
/// external base *function* the generated getter directly overrides. A base variable is never
/// an automatic source.
pub fn resolve_implicit_inheritdoc_var(
    gcx: Gcx<'_>,
    contract_id: ContractId,
    var_name: &str,
) -> Option<InheritedDoc> {
    let getter = variable_getter(gcx, contract_id, var_name)?;
    let source = single_direct_base(gcx, getter, var_name, true)?;
    let mut doc = implicit_function_doc(gcx, source).filter(InheritedDoc::is_renderable)?;
    attach_getter_signature(gcx, &mut doc, getter, source);
    Some(doc)
}

/// Renders a resolved type as its ABI signature string.
fn render_ty<'a>(gcx: Gcx<'a>, ty: Ty<'a>) -> String {
    let mut out = String::new();
    let _ = TyAbiPrinter::new(gcx, &mut out, TyAbiPrinterMode::Signature).print(ty);
    out
}

fn strip_one_whitespace(s: &str) -> Option<&str> {
    let mut chars = s.char_indices();
    let (_, first) = chars.next()?;
    first.is_whitespace().then(|| chars.next().map(|(index, _)| &s[index..]).unwrap_or(""))
}

/// The inherited description for the return at `index`, keyed by the source return name when
/// there is one (solar leaves an unnamed return's name token at the start of the text).
fn inherited_return_description(
    doc: &InheritedDoc,
    index: usize,
    source_name: Option<&str>,
) -> String {
    if let Some(source_name) = source_name {
        for (name, description) in &doc.returns {
            if name == source_name {
                return description.clone();
            }
            if name.is_empty()
                && let Some(rest) = description.strip_prefix(source_name)
                && let Some(rest) = strip_one_whitespace(rest)
            {
                return rest.trim_start().to_string();
            }
        }
        return String::new();
    }
    match doc.returns.get(index) {
        Some((name, description)) if description.is_empty() => name.clone(),
        Some((_, description)) => description.clone(),
        None => String::new(),
    }
}

/// Attach the target getter's signature rows to `doc`: names and ABI types from the generated
/// getter, descriptions from the corresponding source function entries.
fn attach_getter_signature(
    gcx: Gcx<'_>,
    doc: &mut InheritedDoc,
    getter: FunctionId,
    source: FunctionId,
) {
    let getter = gcx.hir.function(getter);
    let source = gcx.hir.function(source);
    let var_name =
        |vid: VariableId| gcx.hir.variable(vid).name.map(|name| name.as_str().to_string());
    let var_type = |vid: VariableId| render_ty(gcx, gcx.type_of_item(vid.into()));

    doc.getter_params = getter
        .parameters
        .iter()
        .enumerate()
        .map(|(index, &target)| {
            let source_name = source.parameters.get(index).and_then(|&vid| var_name(vid));
            let description = source_name
                .as_ref()
                .and_then(|name| doc.params.iter().find(|(candidate, _)| candidate == name))
                .map(|(_, description)| description.clone())
                .unwrap_or_default();
            GetterField {
                name: var_name(target).or(source_name),
                ty: var_type(target),
                description,
            }
        })
        .collect();

    doc.getter_returns = getter
        .returns
        .iter()
        .enumerate()
        .map(|(index, &target)| {
            let source_name = source.returns.get(index).and_then(|&vid| var_name(vid));
            GetterField {
                name: var_name(target).or_else(|| source_name.clone()),
                ty: var_type(target),
                description: inherited_return_description(doc, index, source_name.as_deref()),
            }
        })
        .collect();
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
    let source_types = function_source_param_types(gcx, fid).map(|params| {
        params
            .into_iter()
            // Compare non-ABI source spellings whitespace-insensitively so a base and an
            // override that differ only in the spacing of a `mapping(uint => uint)` type
            // still match.
            .map(|param| normalize_sol_type(&param.split_whitespace().collect::<String>()))
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

/// Extract function parameter types exactly as spelled in source.
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

/// Resolve each collected `@return` to the function's declared return name at the same
/// position. Solar leaves an unnamed return's name token at the start of the description;
/// stripping it and recording the name lets rendering map the description onto a renamed
/// override's return slot positionally instead of gluing the old name into the text.
fn resolve_return_names(gcx: Gcx<'_>, fid: FunctionId, doc: &mut InheritedDoc) {
    let returns = gcx.hir.function(fid).returns;
    for (index, (name, desc)) in doc.returns.iter_mut().enumerate() {
        if !name.is_empty() {
            continue;
        }
        let Some(declared) = returns.get(index).and_then(|&vid| gcx.hir.variable(vid).name) else {
            continue;
        };
        let declared = declared.as_str();
        if let Some(rest) = desc.strip_prefix(declared).and_then(strip_one_whitespace) {
            *desc = rest.trim_start().to_string();
        }
        *name = declared.to_string();
    }
}

/// Re-key inherited return descriptions to the callable at the current inheritance hop.
///
/// Solidity override return lists are positional, so an override rename replaces the name from
/// the previous declaration while preserving the description in the same slot.
fn remap_inherited_return_names(gcx: Gcx<'_>, fid: FunctionId, doc: &mut InheritedDoc) {
    let returns = gcx.hir.function(fid).returns;
    for (index, (name, _)) in doc.returns.iter_mut().enumerate() {
        *name = returns
            .get(index)
            .and_then(|&vid| gcx.hir.variable(vid).name)
            .map(|name| name.as_str().to_string())
            .unwrap_or_default();
    }
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

    let mut doc = collect_inherited_doc(docs);
    // Return-name resolution is applied only on the implicit function path
    // (`implicit_function_doc`); doing it here would also alter the explicit `@inheritdoc` path.
    let _ = &mut doc;
    Some(doc)
}

fn collect_inherited_doc(docs: &DocComments<'_>) -> InheritedDoc {
    let mut result = InheritedDoc {
        notices: Vec::new(),
        devs: Vec::new(),
        params: Vec::new(),
        returns: Vec::new(),
        getter_params: Vec::new(),
        getter_returns: Vec::new(),
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
