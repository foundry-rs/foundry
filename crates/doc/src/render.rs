//! Rendering: `solar` AST -> vocs MDX.

use crate::{
    hir_ext::{self, NameToPage, clean_block_doc_content},
    markdown::{code_regions, logical_lines, neutralize_esm},
    utils::{Deployment, contract_kind_str, page_path},
};
use foundry_common::sh_warn;
use markdown::{ParseOptions, mdast::Node, to_mdast};
use solar::{
    ast::{
        CommentKind, ContractKind, DocComments, FunctionKind, Item, ItemContract, ItemFunction,
        ItemKind, NatSpecKind, ParameterList, SourceUnit, Span, VariableDefinition,
    },
    interface::{
        Ident,
        source_map::{FileName, SourceFile},
    },
    sema::{Gcx, hir},
};
use std::{
    fmt::Write as _,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

// ── rendering context ────────────────────────────────────────────────────────

struct Ctx<'a> {
    src_text: &'a str,
    src_start: usize,
}

impl<'a> Ctx<'a> {
    fn snippet(&self, span: Span) -> &'a str {
        let lo = span.lo().to_usize().saturating_sub(self.src_start);
        let hi = span.hi().to_usize().saturating_sub(self.src_start);
        let lo = lo.min(self.src_text.len());
        let hi = hi.min(self.src_text.len());
        &self.src_text[lo..hi]
    }

    fn dedented_snippet(&self, span: Span) -> String {
        dedent(self.snippet(span))
    }
}

/// The link environment for one page, including its local member anchors.
#[derive(Clone, Copy)]
struct Links<'a> {
    names: &'a NameToPage,
    page: &'a Path,
    local: Option<&'a hir_ext::LocalMembers>,
}

impl Links<'_> {
    fn prose(self, text: &str) -> String {
        hir_ext::replace_inline_links(text, self.names, self.page, self.local)
    }

    fn description(self, text: &str) -> String {
        replace_description_links(text, self.names, self.page, self.local)
    }
}

// ── contract ─────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_contract<'ast, 'gcx>(
    c: &'ast ItemContract<'ast>,
    docs: &'ast DocComments<'ast>,
    ctx: &Ctx<'_>,
    gcx: Gcx<'gcx>,
    hir_id: Option<hir::ContractId>,
    name_to_page: &NameToPage,
    page_path: &Path,
    git_url: Option<&str>,
    deployments: &[Deployment],
) -> String {
    let name = c.name.as_str();

    // Index the members rendered as headings on this page so `{member}` and
    // `{Contract-member}` self-references resolve to anchor-only links.
    let mut local = hir_id.map_or_else(
        || hir_ext::LocalMembers::new(name),
        |id| hir_ext::LocalMembers::for_contract(gcx, id, name_to_page),
    );
    for member in c.body.iter() {
        match &member.kind {
            ItemKind::Variable(v) => {
                if let Some(n) = v.name {
                    local.insert(n.as_str());
                }
            }
            ItemKind::Function(f) => {
                local.insert(&function_heading(f));
                if let Some(anchor) = function_signature_anchor(f, ctx) {
                    local.insert_anchor(anchor);
                }
            }
            ItemKind::Event(e) => local.insert(e.name.as_str()),
            ItemKind::Error(e) => local.insert(e.name.as_str()),
            ItemKind::Struct(s) => local.insert(s.name.as_str()),
            ItemKind::Enum(e) => local.insert(e.name.as_str()),
            ItemKind::Udvt(u) => local.insert(u.name.as_str()),
            _ => {}
        }
    }
    let links = Links { names: name_to_page, page: page_path, local: Some(&local) };

    let comments = collect_comments(docs, links);
    let mut out = write_page_header(name, first_notice(&comments), git_url);
    write_deployments_table(&mut out, deployments);

    // inheritance links.
    if let Some(id) = hir_id
        && let Some(inherits) = hir_ext::inheritance_links(gcx, id, name_to_page, page_path)
    {
        writeln!(out, "{inherits}").unwrap();
        writeln!(out).unwrap();
    }

    write_comment_block(&mut out, &comments);

    // Group members.
    let mut constants: Vec<(Span, &VariableDefinition<'_>, &DocComments<'_>)> = Vec::new();
    let mut state_vars: Vec<(Span, &VariableDefinition<'_>, &DocComments<'_>)> = Vec::new();
    let mut functions: Vec<(Span, &ItemFunction<'_>, &DocComments<'_>)> = Vec::new();
    let mut events = Vec::new();
    let mut errors = Vec::new();
    let mut structs = Vec::new();
    let mut enums = Vec::new();
    let mut udvts = Vec::new();

    for member in c.body.iter() {
        let s = member.span;
        match &member.kind {
            ItemKind::Variable(v) => {
                // constants and immutables get their own section.
                if v.mutability.is_some_and(|m| m.is_constant() || m.is_immutable()) {
                    constants.push((s, v, &member.docs));
                } else {
                    state_vars.push((s, v, &member.docs));
                }
            }
            ItemKind::Function(f) => functions.push((s, f, &member.docs)),
            ItemKind::Event(_) => events.push(member),
            ItemKind::Error(_) => errors.push(member),
            ItemKind::Struct(_) => structs.push(member),
            ItemKind::Enum(_) => enums.push(member),
            ItemKind::Udvt(_) => udvts.push(member),
            _ => {}
        }
    }

    let write_vars =
        |out: &mut String, vars: &[(Span, &VariableDefinition<'_>, &DocComments<'_>)]| {
            for (span, v, docs) in vars {
                let vname = v.name.map(|n| n.as_str().to_string()).unwrap_or_default();
                writeln!(out, "### {vname}").unwrap();
                writeln!(out).unwrap();
                let mut c = collect_comments(docs, links);
                // Explicit `@inheritdoc` merges into a partial local doc; implicit
                // inheritance only runs when the variable has no local NatSpec at all.
                let vid = hir_id.and_then(|cid| {
                    gcx.hir.contract(cid).items.iter().find_map(|&item| match item {
                        hir::ItemId::Variable(id) if gcx.hir.variable(id).span == *span => Some(id),
                        _ => None,
                    })
                });
                let inherited = vid.and_then(|id| {
                    let explicit = has_inheritdoc(docs);
                    (explicit || !has_local_natspec(docs))
                        .then(|| hir_ext::natspec_doc(gcx, id.into(), !explicit))
                        .flatten()
                });
                let sanitize = |s: &str| links.prose(s);
                let sanitize_description = |s: &str| links.description(s);
                if let Some(base_doc) = &inherited {
                    c.inherit_descriptions(base_doc, &sanitize_description);
                }
                write_comment_block(out, &c);
                write_code_block(out, &ctx.dedented_snippet(*span));
                // When the documentation is inherited from the variable's generated getter,
                // render the getter's parameter and return signature instead of the declared
                // (possibly mapping) type.
                let getter_doc = inherited.as_ref().filter(|d| !d.getter_returns.is_empty());
                if let Some(base_doc) = getter_doc {
                    write_getter_table(out, "Parameters", &base_doc.getter_params, &sanitize);
                    write_getter_table(out, "Returns", &base_doc.getter_returns, &sanitize);
                } else if !c.returns.is_empty() {
                    let ty = format!("`{}`", ctx.snippet(v.ty.span).trim());
                    write_signature_table_header(out, "Returns");
                    for (name, desc) in &c.returns {
                        // Solar parses `@return <first-word> <rest>` where the first word
                        // becomes `name` and the rest becomes `desc`. For unnamed returns the
                        // first word is actually part of the description, so recombine them.
                        let full_desc =
                            if desc.is_empty() { name.clone() } else { format!("{name} {desc}") };
                        let desc_cell = escape_table_cell(&full_desc);
                        writeln!(out, "| &lt;none&gt; | {ty} | {desc_cell} |").unwrap();
                    }
                    writeln!(out).unwrap();
                }
            }
        };

    for (heading, vars) in [("Constants", constants), ("State Variables", state_vars)] {
        if !vars.is_empty() {
            writeln!(out, "## {heading}\n").unwrap();
            write_vars(&mut out, &vars);
        }
    }

    if !functions.is_empty() {
        writeln!(out, "## Functions").unwrap();
        writeln!(out).unwrap();
        for (span, f, docs) in &functions {
            let fn_name = match f.kind {
                FunctionKind::Constructor => None,
                FunctionKind::Fallback => Some("fallback".to_string()),
                FunctionKind::Receive => Some("receive".to_string()),
                FunctionKind::Function | FunctionKind::Modifier => {
                    f.header.name.map(|name| name.as_str().to_string())
                }
            };
            // Explicit `@inheritdoc` merges into a partial local doc; implicit inheritance
            // only runs when the function has no local NatSpec at all.
            let inherited = fn_name.as_deref().and_then(|fname| {
                let fid = hir_id.and_then(|cid| {
                    gcx.hir.contract(cid).items.iter().find_map(|&item| match item {
                        hir::ItemId::Function(id) if gcx.hir.function(id).span == *span => Some(id),
                        _ => None,
                    })
                });
                match (has_inheritdoc(docs), has_local_natspec(docs)) {
                    (true, _) => {
                        if hir_id.is_some() && fid.is_none() {
                            let _ = sh_warn!(
                                "forge doc: failed to find HIR function for `{}.{fname}` while resolving @inheritdoc",
                                c.name
                            );
                        }
                        fid.and_then(|id| hir_ext::natspec_doc(gcx, id.into(), false))
                    }
                    (false, false) =>
                        fid.and_then(|id| hir_ext::natspec_doc(gcx, id.into(), true)),
                    (false, true) => None,
                }
            });
            render_function_section(&mut out, *span, f, docs, ctx, links, inherited.as_ref());
        }
    }

    for (heading, items) in [
        ("Events", events),
        ("Errors", errors),
        ("Structs", structs),
        ("Enums", enums),
        ("Custom Types", udvts),
    ] {
        if !items.is_empty() {
            writeln!(out, "## {heading}\n").unwrap();
            for item in items {
                writeln!(out, "### {}\n", item.name().unwrap()).unwrap();
                let comments = collect_comments(&item.docs, links);
                write_item_body(&mut out, item, &comments, ctx);
            }
        }
    }

    out
}

// ── free functions ────────────────────────────────────────────────────────────

fn render_free_functions(
    name: &str,
    overloads: &[(Span, &ItemFunction<'_>, &DocComments<'_>)],
    ctx: &Ctx<'_>,
    links: Links<'_>,
    git_url: Option<&str>,
) -> String {
    let title = if name.is_empty() { "function" } else { name };
    let first_comments = collect_comments(overloads[0].2, links);
    let mut out = write_page_header(title, first_notice(&first_comments), git_url);
    for (span, f, docs) in overloads {
        render_function_section(&mut out, *span, f, docs, ctx, links, None);
    }
    out
}

// ── constants ─────────────────────────────────────────────────────────────────

fn render_constants(
    stem: &str,
    vars: &[(Span, &VariableDefinition<'_>, &DocComments<'_>)],
    ctx: &Ctx<'_>,
    links: Links<'_>,
    git_url: Option<&str>,
) -> String {
    let title = format!("{stem} Constants");
    let mut out = write_page_header(&title, None, git_url);
    for (span, v, docs) in vars {
        let name = v.name.map(|n| n.as_str().to_string()).unwrap_or_else(|| "_".to_string());
        writeln!(out, "## {name}").unwrap();
        writeln!(out).unwrap();
        let c = collect_comments(docs, links);
        write_comment_block(&mut out, &c);
        write_code_block(&mut out, &ctx.dedented_snippet(*span));
    }
    out
}

// ── standalone items ──────────────────────────────────────────────────────────

/// Render the common body of a struct, enum, event, error, or value type.
fn write_item_body(out: &mut String, item: &Item<'_>, comments: &CommentData, ctx: &Ctx<'_>) {
    write_comment_block(out, comments);
    let mut snippet = ctx.dedented_snippet(item.span);
    if matches!(item.kind, ItemKind::Udvt(_)) {
        snippet.push(';');
    }
    write_code_block(out, &snippet);
    match &item.kind {
        ItemKind::Struct(s) => write_struct_properties_table(out, s.fields, comments, ctx),
        ItemKind::Enum(e) => write_enum_variants_table(out, e.variants, comments),
        ItemKind::Event(e) => {
            write_param_table(out, "Parameters", &e.parameters, comments, None, ctx)
        }
        ItemKind::Error(e) => {
            write_param_table(out, "Parameters", &e.parameters, comments, None, ctx)
        }
        _ => {}
    }
}

// ── function section ──────────────────────────────────────────────────────────
fn render_function_section(
    out: &mut String,
    span: Span,
    f: &ItemFunction<'_>,
    docs: &DocComments<'_>,
    ctx: &Ctx<'_>,
    links: Links<'_>,
    inherited: Option<&hir_ext::NatSpecDoc>,
) {
    let heading = function_heading(f);
    if let Some(anchor) = function_signature_anchor(f, ctx) {
        writeln!(out, "<a id=\"{anchor}\"></a>").unwrap();
        writeln!(out).unwrap();
    }
    writeln!(out, "### {heading}").unwrap();
    writeln!(out).unwrap();
    let mut c = collect_comments(docs, links);
    let mut inherited_params = None;
    // Merge inherited natspec for missing tags.
    if let Some(inherited) = inherited {
        let sanitize = |s: &str| links.prose(s);
        let sanitize_description = |s: &str| links.description(s);
        c.inherit_descriptions(inherited, &sanitize_description);
        if c.params.is_empty() {
            let params = inherited.params.iter().map(|desc| sanitize(desc)).collect::<Vec<_>>();
            for (index, desc) in params.iter().enumerate() {
                if let Some(name) = f.header.parameters.get(index).and_then(|param| param.name) {
                    c.params.push((name.as_str().to_string(), desc.clone()));
                }
            }
            inherited_params = Some(params);
        }
        if c.returns.is_empty() {
            for (index, desc) in inherited.returns.iter().enumerate() {
                let name = f
                    .header
                    .returns
                    .as_ref()
                    .and_then(|returns| returns.get(index))
                    .and_then(|return_| return_.name)
                    .map(|name| name.as_str().to_string())
                    .unwrap_or_default();
                c.returns.push((name, sanitize(desc)));
            }
        }
    }
    write_comment_block(out, &c);
    let hspan = if f.header.span.lo() == f.header.span.hi() { span } else { f.header.span };
    let snippet = ctx.dedented_snippet(hspan);
    write_code_block(out, &format!("{snippet};"));
    write_param_table(
        out,
        "Parameters",
        &f.header.parameters,
        &c,
        inherited_params.as_deref(),
        ctx,
    );
    if let Some(returns) = &f.header.returns {
        write_param_table(out, "Returns", returns, &c, None, ctx);
    }
}

fn function_heading(f: &ItemFunction<'_>) -> String {
    match f.kind {
        FunctionKind::Constructor => "constructor".to_string(),
        FunctionKind::Fallback => "fallback".to_string(),
        FunctionKind::Receive => "receive".to_string(),
        FunctionKind::Function | FunctionKind::Modifier => {
            f.header.name.map(|n| n.as_str().to_string()).unwrap_or_else(|| "function".to_string())
        }
    }
}

fn function_signature_anchor(f: &ItemFunction<'_>, ctx: &Ctx<'_>) -> Option<String> {
    let name = match f.kind {
        FunctionKind::Constructor => "constructor".to_string(),
        FunctionKind::Fallback => "fallback".to_string(),
        FunctionKind::Receive => "receive".to_string(),
        FunctionKind::Function | FunctionKind::Modifier => f.header.name?.as_str().to_string(),
    };
    let params = f
        .header
        .parameters
        .vars
        .iter()
        .map(|v| ctx.snippet(v.ty.span).trim().to_string())
        .collect::<Vec<_>>();

    Some(hir_ext::function_signature_anchor(&name, &params))
}

// ── natspec comment collection ────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum DescKind {
    Notice,
    Dev,
}

struct Description {
    kind: DescKind,
    content: String,
}

#[derive(Default)]
struct CommentData {
    titles: Vec<String>,
    authors: Vec<String>,
    /// All notice/dev text in source order, tagged with their kind, with continuation
    /// lines joined to their parent. Used for rendering to preserve correct paragraph
    /// ordering and to italicize `@dev` paragraphs as a whole.
    descriptions: Vec<Description>,
    params: Vec<(String, String)>,
    returns: Vec<(String, String)>,
    customs: Vec<(String, String)>,
    /// `@custom:name <name>` values, used to fill in unnamed function parameters.
    unnamed_param_names: Vec<String>,
}

impl CommentData {
    /// Fill missing notice/dev tags, keeping inherited notices before local descriptions.
    fn inherit_descriptions(
        &mut self,
        inherited: &hir_ext::NatSpecDoc,
        sanitize: &impl Fn(&str) -> String,
    ) {
        if !self.descriptions.iter().any(|d| d.kind == DescKind::Notice) {
            let mut descriptions = inherited
                .notices
                .iter()
                .map(|s| Description { kind: DescKind::Notice, content: sanitize(s) })
                .collect::<Vec<_>>();
            descriptions.append(&mut self.descriptions);
            self.descriptions = descriptions;
        }
        if !self.descriptions.iter().any(|d| d.kind == DescKind::Dev) {
            self.descriptions.extend(
                inherited
                    .devs
                    .iter()
                    .map(|s| Description { kind: DescKind::Dev, content: sanitize(s) }),
            );
        }
    }
}

/// Collect natspec from doc comments, applying inline link replacement.
///
/// Solar emits each `///` line as a separate `DocComment`. Lines without a `@` tag become
/// synthetic `@notice` items. We join adjacent synthetic items to the previous rendered section
/// so multi-line natspec tags form a single coherent block in source order.
fn collect_comments(docs: &DocComments<'_>, links: Links<'_>) -> CommentData {
    let mut data = CommentData::default();

    // Tags that are not user-facing natspec; do not warn on these.
    const FILTERED_CUSTOM: &[&str] = &["solidity", "src", "use-src", "ast-id"];
    // Recognised natspec custom tags (mirror legacy behaviour).
    const KNOWN_CUSTOM: &[&str] = &["name"];

    // Track whether the previous DocComment was blank (empty natspec), which signals a
    // paragraph break even between continuation lines.
    let mut prev_doc_was_blank = false;
    #[derive(Clone, Copy)]
    enum LastSection {
        Desc, // notice or dev (both go through descriptions)
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
            // For /** */ block comments Solar preserves raw ` * ` line decorations inside the
            // content range. Strip them so multi-line content renders cleanly.
            let raw: &str =
                if doc.kind == CommentKind::Block { &clean_block_doc_content(raw) } else { raw };

            // Solar represents an untagged doc comment as a synthetic notice whose span is the
            // whole comment. Treat it as a continuation when it follows a rendered section;
            // this also joins adjacent line and block doc comments before fence detection.
            let is_continuation = matches!(item.kind, NatSpecKind::Notice) && item.span == doc.span;

            let trimmed = raw.trim();
            if trimmed.is_empty() {
                prev_doc_was_blank = true;
                continue;
            }

            // Keep descriptions raw until continuation lines have been joined. Only complete,
            // standalone descriptions can safely identify fenced code blocks.
            let content = trimmed.to_string();

            if is_continuation && !prev_doc_was_blank {
                let appended = match last_section {
                    Some(LastSection::Desc) => data.descriptions.last_mut().map(|d| &mut d.content),
                    Some(LastSection::Param) => data.params.last_mut().map(|(_, d)| d),
                    Some(LastSection::Return) => data.returns.last_mut().map(|(_, d)| d),
                    None => None,
                };
                if let Some(last) = appended {
                    last.push('\n');
                    last.push_str(&content);
                    prev_doc_was_blank = false;
                    continue;
                }
            }

            prev_doc_was_blank = false;

            match item.kind {
                NatSpecKind::Title => data.titles.push(content),
                NatSpecKind::Author => data.authors.push(content),
                NatSpecKind::Notice => {
                    data.descriptions.push(Description { kind: DescKind::Notice, content });
                    last_section = Some(LastSection::Desc);
                }
                NatSpecKind::Dev => {
                    data.descriptions.push(Description { kind: DescKind::Dev, content });
                    last_section = Some(LastSection::Desc);
                }
                NatSpecKind::Param { name } => {
                    data.params.push((name.as_str().to_string(), content));
                    last_section = Some(LastSection::Param);
                }
                NatSpecKind::Return { name } => {
                    data.returns.push((
                        name.map(|name| name.as_str().to_string()).unwrap_or_default(),
                        content,
                    ));
                    last_section = Some(LastSection::Return);
                }
                NatSpecKind::Inheritdoc { .. } => {} // resolved separately via HIR
                NatSpecKind::Custom { name } => {
                    let tag = name.as_str();
                    if FILTERED_CUSTOM.contains(&tag) {
                        // Silently ignored.
                    } else if tag == "name" {
                        // `@custom:name <name>` -> unnamed param name (legacy parity).
                        let content = links.prose(&content);
                        if let Some(first) = content.split_whitespace().next() {
                            data.unnamed_param_names.push(first.to_string());
                        }
                    } else {
                        // unknown-natspec-tag warning.
                        if !KNOWN_CUSTOM.contains(&tag) && !is_known_custom_tag(tag) {
                            warn!("unknown natspec custom tag: @custom:{tag}");
                        }
                        data.customs.push((tag.to_string(), content));
                    }
                }
                NatSpecKind::Internal { .. } => {}
            }
        }
    }

    for content in data
        .titles
        .iter_mut()
        .chain(&mut data.authors)
        .chain(data.customs.iter_mut().map(|(_, content)| content))
    {
        *content = sanitize_description_prose(content, links.names, links.page, links.local);
    }
    for (_, content) in data.params.iter_mut().chain(&mut data.returns) {
        *content = links.prose(content);
    }
    for description in &mut data.descriptions {
        description.content = links.description(&description.content);
    }

    data
}

/// Returns true if `tag` looks like a generally-recognised natspec custom tag.
///
/// We accept any non-empty alphanumeric/dash identifier as "known enough" not
/// to warn, only obviously malformed tags trigger the warning channel.
fn is_known_custom_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Returns the base contract name from `@inheritdoc Base`, or `None`.
/// Whether the declaration carries any local NatSpec item other than `@inheritdoc`. Solidity
/// only auto-inherits documentation for a member with none, so implicit inheritance is gated
/// on this being false; a local `@custom:*`, `@title` or `@author` counts as local
/// documentation just like `@notice`/`@dev`/`@param`/`@return`.
fn has_local_natspec(docs: &DocComments<'_>) -> bool {
    docs.iter().any(|doc| {
        doc.natspec.iter().any(|item| !matches!(item.kind, NatSpecKind::Inheritdoc { .. }))
    })
}

/// Render a getter signature table (`Parameters` or `Returns`) from its inherited rows.
fn write_getter_table(
    out: &mut String,
    heading: &str,
    fields: &[hir_ext::GetterField],
    sanitize: &impl Fn(&str) -> String,
) {
    if fields.is_empty() {
        return;
    }
    write_signature_table_header(out, heading);
    for field in fields {
        let name = field
            .name
            .as_deref()
            .map(escape_table_cell)
            .unwrap_or_else(|| "&lt;none&gt;".to_string());
        let ty = escape_table_cell(&field.ty);
        let desc = escape_table_cell(&sanitize(&field.description));
        writeln!(out, "| {name} | `{ty}` | {desc} |").unwrap();
    }
    writeln!(out).unwrap();
}

fn has_inheritdoc(docs: &DocComments<'_>) -> bool {
    docs.iter()
        .flat_map(|doc| doc.natspec.iter())
        .any(|item| matches!(item.kind, NatSpecKind::Inheritdoc { .. }))
}

fn first_notice(data: &CommentData) -> Option<&str> {
    data.descriptions
        .iter()
        .find(|description| description.kind == DescKind::Notice)
        .map(|description| description.content.as_str())
}

// ── markdown output helpers ───────────────────────────────────────────────────

fn write_frontmatter(out: &mut String, title: &str, description: Option<&str>) {
    writeln!(out, "---").unwrap();
    writeln!(out, "title: \"{}\"", yaml_escape_double_quoted(title)).unwrap();
    if let Some(desc) = description {
        // Collapse whitespace so multi-line notices stay on one line, then escape.
        let collapsed: String = desc.split_whitespace().collect::<Vec<_>>().join(" ");
        writeln!(out, "description: \"{}\"", yaml_escape_double_quoted(&collapsed)).unwrap();
    }
    writeln!(out, "---").unwrap();
    writeln!(out).unwrap();
}

/// Escape a string for use as a YAML double-quoted scalar.
///
/// Per the YAML 1.2 spec, double-quoted scalars must escape `"` and `\`, and
/// any control character (including newline, tab, carriage return) must be
/// represented via an escape sequence rather than embedded literally.
fn yaml_escape_double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0}' => out.push_str("\\0"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Italicize a `@dev` block by wrapping it in `<i>...</i>` HTML tags. Surrounding
/// blank lines around the tags ensure MDX/CommonMark parses the inner content as
/// block-level markdown (lists, code fences, multiple paragraphs all work).
fn italicize_dev(content: &str) -> String {
    let trimmed = content.trim_matches('\n');
    if trimmed.is_empty() { String::new() } else { format!("<i>\n\n{trimmed}\n\n</i>") }
}

/// Replace inline links in a standalone notice or dev description while preserving complete,
/// top-level fenced code blocks. Other NatSpec fields use `replace_inline_links` directly because
/// their rendering context (notably table cells) cannot contain block-level Markdown.
fn replace_description_links(
    text: &str,
    name_to_page: &NameToPage,
    current_page: &Path,
    local: Option<&hir_ext::LocalMembers>,
) -> String {
    let regions = fenced_description_regions(text);
    let mut out = String::with_capacity(text.len());
    let mut rendered_regions = Vec::with_capacity(regions.len());
    let mut copied = 0;
    for region in regions {
        out.push_str(&sanitize_description_prose(
            &text[copied..region.start],
            name_to_page,
            current_page,
            local,
        ));
        let start = out.len();
        out.push_str(&text[region.clone()]);
        rendered_regions.push(start..out.len());
        copied = region.end;
    }
    out.push_str(&sanitize_description_prose(&text[copied..], name_to_page, current_page, local));
    let mdx_regions = code_regions(&out, &ParseOptions::mdx());
    if rendered_regions.iter().all(|region| mdx_regions.contains(region)) {
        out
    } else {
        sanitize_description_prose(text, name_to_page, current_page, local)
    }
}

fn sanitize_description_prose(
    text: &str,
    name_to_page: &NameToPage,
    current_page: &Path,
    local: Option<&hir_ext::LocalMembers>,
) -> String {
    let text = hir_ext::replace_inline_links(text, name_to_page, current_page, local);
    neutralize_fence_markers(&text)
}

/// Keep rejected or incomplete fence markers from changing the Markdown context of subsequent
/// descriptions. Entities render as the original marker characters without acting as syntax.
fn neutralize_fence_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let marker @ (b'`' | b'~') = bytes[i] {
            let length = bytes[i..].iter().take_while(|&&byte| byte == marker).count();
            if length >= 3 {
                out.push_str(if marker == b'`' { "&#96;" } else { "&#126;" });
                out.push_str(&text[i + 1..i + length]);
                i += length;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Complete fenced code blocks that are direct children of the description document. Restricting
/// preservation to root-level blocks keeps list, quote, table, and unclosed-fence behavior on the
/// conservative escaping path.
fn fenced_description_regions(text: &str) -> Vec<Range<usize>> {
    let Ok(Node::Root(root)) = to_mdast(text, &ParseOptions::gfm()) else {
        return Vec::new();
    };
    root.children
        .iter()
        .filter_map(|node| {
            let Node::Code(_) = node else { return None };
            let position = node.position()?;
            let range = position.start.offset..position.end.offset;
            is_complete_fence(&text[range.clone()]).then_some(range)
        })
        .collect()
}

fn is_complete_fence(text: &str) -> bool {
    let mut lines = logical_lines(text);
    let Some((_, first)) = lines.next() else { return false };
    let Some((marker, length)) = fence_marker(first) else { return false };
    let mut last = None;
    for (_, line) in lines {
        last = Some(line);
    }
    let Some(last) = last else { return false };
    let indent = last.len() - last.trim_start_matches(' ').len();
    if indent > 3 {
        return false;
    }
    let last = &last[indent..];
    let closing_length = last.chars().take_while(|&ch| ch == marker).count();
    closing_length >= length && last[closing_length..].trim().is_empty()
}

fn fence_marker(line: &str) -> Option<(char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let line = &line[indent..];
    let marker @ ('`' | '~') = line.chars().next()? else { return None };
    let length = line.chars().take_while(|&ch| ch == marker).count();
    (length >= 3).then_some((marker, length))
}

fn write_comment_block(out: &mut String, data: &CommentData) {
    let mut block = String::new();
    if !data.titles.is_empty() {
        let label = if data.titles.len() == 1 { "Title" } else { "Titles" };
        writeln!(block, "**{label}:** {}", data.titles.join(", ")).unwrap();
        writeln!(block).unwrap();
    }
    if !data.authors.is_empty() {
        let label = if data.authors.len() == 1 { "Author" } else { "Authors" };
        writeln!(block, "**{label}:** {}", data.authors.join(", ")).unwrap();
        writeln!(block).unwrap();
    }
    // Render descriptions in source order (notices and devs interleaved, continuations joined).
    // `@dev` paragraphs are wrapped in `_..._` per paragraph so each multi-line block renders
    // as a single italic span (markdown emphasis cannot cross blank lines).
    for desc in &data.descriptions {
        match desc.kind {
            DescKind::Notice => writeln!(block, "{}", desc.content).unwrap(),
            DescKind::Dev => writeln!(block, "{}", italicize_dev(&desc.content)).unwrap(),
        }
        writeln!(block).unwrap();
    }
    if !data.customs.is_empty() {
        let label = if data.customs.len() == 1 { "Note" } else { "Notes" };
        writeln!(block, "**{label}:**").unwrap();
        writeln!(block).unwrap();
        for (tag, content) in &data.customs {
            writeln!(block, "- **{tag}:** {content}").unwrap();
        }
        writeln!(block).unwrap();
    }
    // Neutralize the fully assembled block once: fence state stays continuous across the
    // whole block, and every displayed line (authors, notices, custom notes), not just
    // descriptions, is covered.
    out.push_str(&neutralize_esm(&block));
}

fn write_code_block(out: &mut String, snippet: &str) {
    writeln!(out, "```solidity").unwrap();
    writeln!(out, "{}", snippet.trim_end()).unwrap();
    writeln!(out, "```").unwrap();
    writeln!(out).unwrap();
}

fn write_page_header(title: &str, description: Option<&str>, git_url: Option<&str>) -> String {
    let mut out = String::new();
    write_frontmatter(&mut out, title, description);
    writeln!(out, "# {title}").unwrap();
    writeln!(out).unwrap();
    write_git_source(&mut out, git_url);
    out
}

/// Write link if `git_url` is set.
fn write_git_source(out: &mut String, git_url: Option<&str>) {
    if let Some(url) = git_url {
        writeln!(out, "[Git Source]({url})").unwrap();
        writeln!(out).unwrap();
    }
}

/// Escape a value so it is safe inside a markdown (GFM) table cell:
/// - replace `|` with `\|` (column separator)
/// - replace newlines with `<br/>` (cells must be one logical line)
/// - replace `\r` so CRLF natspec doesn't create stray spaces
fn escape_table_cell(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace("\r\n", "<br/>")
        .replace(['\n', '\r'], "<br/>")
}

/// Write the **Deployments** table for a contract page.
fn write_deployments_table(out: &mut String, deployments: &[Deployment]) {
    if deployments.is_empty() {
        return;
    }
    writeln!(out, "**Deployments**").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "| Network | Address |").unwrap();
    writeln!(out, "| ------- | ------- |").unwrap();
    for d in deployments {
        let network = escape_table_cell(d.network.as_deref().unwrap_or("-"));
        writeln!(out, "| {network} | `{:#x}` |", d.address).unwrap();
    }
    writeln!(out).unwrap();
}

fn write_param_table(
    out: &mut String,
    heading: &str,
    params: &ParameterList<'_>,
    comments: &CommentData,
    inherited_params: Option<&[String]>,
    ctx: &Ctx<'_>,
) {
    if params.is_empty() {
        return;
    }
    write_signature_table_header(out, heading);
    let is_return = heading == "Returns";
    // Positional fall-back to `@custom:name <name>` for unnamed params
    // (parameters only, return names aren't substituted).
    let mut unnamed_iter = comments.unnamed_param_names.iter();
    for (index, var) in params.iter().enumerate() {
        let name = match var.name {
            Some(n) => n.as_str().to_string(),
            None if !is_return => unnamed_iter.next().cloned().unwrap_or_else(|| "_".to_string()),
            None => "&lt;none&gt;".to_string(),
        };
        let ty = format!("`{}`", ctx.snippet(var.ty.span).trim());
        let desc = if is_return {
            return_description(comments, index, var.name.map(|_| name.as_str()))
        } else {
            let named = comments.params.iter().find(|(n, _)| n == &name).map(|(_, d)| d.as_str());
            if var.name.is_none() {
                inherited_params
                    .and_then(|params| params.get(index))
                    .map(String::as_str)
                    .or(named)
                    .unwrap_or("")
            } else {
                named.unwrap_or("")
            }
        };
        let name = escape_table_cell(&name);
        let desc = escape_table_cell(desc);
        writeln!(out, "| {name} | {ty} | {desc} |").unwrap();
    }
    writeln!(out).unwrap();
}

fn return_description<'a>(
    comments: &'a CommentData,
    index: usize,
    return_name: Option<&str>,
) -> &'a str {
    if let Some(return_name) = return_name
        && let Some((_, desc)) = comments.returns.iter().find(|(n, _)| n == return_name)
    {
        return desc;
    }

    let Some((doc_name, desc)) = comments.returns.get(index) else {
        return "";
    };

    if !doc_name.is_empty() {
        return desc;
    }

    match return_name {
        Some(return_name) => desc.strip_prefix(return_name).and_then(strip_one_ws).unwrap_or(desc),
        None => desc,
    }
}

fn strip_one_ws(s: &str) -> Option<&str> {
    let mut chars = s.char_indices();
    let (_, first) = chars.next()?;
    first.is_whitespace().then(|| chars.next().map(|(idx, _)| &s[idx..]).unwrap_or(""))
}

fn write_struct_properties_table(
    out: &mut String,
    fields: &[VariableDefinition<'_>],
    comments: &CommentData,
    ctx: &Ctx<'_>,
) {
    if fields.is_empty() {
        return;
    }
    write_signature_table_header(out, "Properties");
    for field in fields {
        let name = field.name.map(|n| n.as_str().to_string()).unwrap_or_else(|| "_".to_string());
        let ty = format!("`{}`", ctx.snippet(field.ty.span).trim());
        let desc =
            comments.params.iter().find(|(n, _)| n == &name).map(|(_, d)| d.as_str()).unwrap_or("");
        let name = escape_table_cell(&name);
        let desc = escape_table_cell(desc);
        writeln!(out, "| {name} | {ty} | {desc} |").unwrap();
    }
    writeln!(out).unwrap();
}

fn write_enum_variants_table(out: &mut String, variants: &[Ident], comments: &CommentData) {
    if variants.is_empty() {
        return;
    }
    writeln!(out, "**Variants**").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "| Name | Description |").unwrap();
    writeln!(out, "| ---- | ----------- |").unwrap();
    for variant in variants {
        let name = variant.as_str();
        let desc =
            comments.params.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_str()).unwrap_or("");
        let name = escape_table_cell(name);
        let desc = escape_table_cell(desc);
        writeln!(out, "| {name} | {desc} |").unwrap();
    }
    writeln!(out).unwrap();
}

/// Common layout for parameters, returns, and struct properties.
fn write_signature_table_header(out: &mut String, heading: &str) {
    writeln!(out, "**{heading}**\n\n| Name | Type | Description |\n| ---- | ---- | ----------- |")
        .unwrap();
}

/// Find the HIR `ContractId` for a contract by name, requiring the contract to
/// live in the source file currently being rendered (compared via absolute path)
/// so contracts that share a file stem across `src/` and `lib/` cannot collide.
fn find_contract_id<'gcx>(
    gcx: Gcx<'gcx>,
    name: &str,
    abs_sol_path: &Path,
) -> Option<hir::ContractId> {
    gcx.hir.contract_ids().find(|&id| {
        let c = gcx.hir.contract(id);
        if c.name.as_str() != name {
            return false;
        }
        match &gcx.hir.source(c.source).file.name {
            FileName::Real(p) => p == abs_sol_path,
            _ => false,
        }
    })
}

/// Strip common leading whitespace from all non-empty lines.
fn dedent(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.is_empty() {
        return s.to_string();
    }
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| if l.len() >= indent { &l[indent..] } else { l.trim() })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── public entry point ───────────────────────────────────────────────────────

/// Render a single Solidity source file as a list of `(relative_output_path, mdx_content)` pairs.
#[allow(clippy::too_many_arguments)]
pub fn source<'ast, 'gcx>(
    ast: &'ast SourceUnit<'ast>,
    file: &Arc<SourceFile>,
    rel_sol_path: &Path,
    abs_sol_path: &Path,
    gcx: Gcx<'gcx>,
    name_to_page: &NameToPage,
    git_url: Option<&str>,
    deployments: &[Deployment],
) -> Vec<(PathBuf, String)> {
    let stem = rel_sol_path.file_stem().and_then(|s| s.to_str()).unwrap_or("constants");

    let src_text = file.src.as_str();
    let src_start = file.start_pos.to_usize();
    let ctx = Ctx { src_text, src_start };

    let mut pages: Vec<(PathBuf, String)> = Vec::new();
    let mut const_vars: Vec<(Span, &VariableDefinition<'_>, &DocComments<'_>)> = Vec::new();
    let mut free_fns: std::collections::BTreeMap<
        String,
        Vec<(Span, &ItemFunction<'_>, &DocComments<'_>)>,
    > = Default::default();

    for item in ast.items.iter() {
        let span = item.span;
        match &item.kind {
            ItemKind::Pragma(_) | ItemKind::Import(_) | ItemKind::Using(_) => (),
            ItemKind::Contract(c) => {
                let kind_str = contract_kind_str(c.kind);
                let page_path = page_path(rel_sol_path, kind_str, c.name.as_str());
                // Look up HIR contract id for inheritance/inheritdoc.
                let hir_id = find_contract_id(gcx, c.name.as_str(), abs_sol_path);
                // Deployments only apply to non-abstract, non-interface, non-library contracts.
                let contract_deployments = if matches!(c.kind, ContractKind::Contract)
                    && rel_sol_path.file_stem().and_then(|s| s.to_str()) == Some(c.name.as_str())
                {
                    deployments
                } else {
                    &[]
                };
                let content = render_contract(
                    c,
                    &item.docs,
                    &ctx,
                    gcx,
                    hir_id,
                    name_to_page,
                    &page_path,
                    git_url,
                    contract_deployments,
                );
                pages.push((page_path, content));
            }

            ItemKind::Function(f) => {
                let name = f.header.name.map(|n| n.as_str().to_string()).unwrap_or_default();
                free_fns.entry(name).or_default().push((span, f, &item.docs));
            }

            ItemKind::Variable(v) => {
                const_vars.push((span, v, &item.docs));
            }

            ItemKind::Struct(_)
            | ItemKind::Enum(_)
            | ItemKind::Udvt(_)
            | ItemKind::Error(_)
            | ItemKind::Event(_) => {
                let prefix = match &item.kind {
                    ItemKind::Struct(_) => "struct",
                    ItemKind::Enum(_) => "enum",
                    ItemKind::Udvt(_) => "type",
                    ItemKind::Error(_) => "error",
                    ItemKind::Event(_) => "event",
                    _ => unreachable!(),
                };
                let name = item.name().unwrap();
                let page_path = page_path(rel_sol_path, prefix, name.as_str());
                let comments = collect_comments(
                    &item.docs,
                    Links { names: name_to_page, page: &page_path, local: None },
                );
                let mut content =
                    write_page_header(name.as_str(), first_notice(&comments), git_url);
                write_item_body(&mut content, item, &comments, &ctx);
                pages.push((page_path, content));
            }
        }
    }

    for (name, overloads) in &free_fns {
        let page_path = page_path(rel_sol_path, "function", name);
        let content = render_free_functions(
            name,
            overloads,
            &ctx,
            Links { names: name_to_page, page: &page_path, local: None },
            git_url,
        );
        pages.push((page_path, content));
    }

    if !const_vars.is_empty() {
        let page_path = page_path(rel_sol_path, "constants", stem);
        let content = render_constants(
            stem,
            &const_vars,
            &ctx,
            Links { names: name_to_page, page: &page_path, local: None },
            git_url,
        );
        pages.push((page_path, content));
    }

    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use markdown::MdxSignal;

    fn parse_mdx(text: &str) -> Node {
        let mut options = ParseOptions::mdx();
        options.mdx_esm_parse = Some(Box::new(|_| MdxSignal::Ok));
        to_mdast(text, &options).unwrap()
    }

    fn contains_mdx_esm(node: &Node) -> bool {
        matches!(node, Node::MdxjsEsm(_))
            || node.children().is_some_and(|children| children.iter().any(contains_mdx_esm))
    }

    fn contains_mdx_expression(node: &Node) -> bool {
        matches!(node, Node::MdxFlowExpression(_) | Node::MdxTextExpression(_))
            || node.children().is_some_and(|children| children.iter().any(contains_mdx_expression))
    }

    #[test]
    fn preserves_complete_top_level_description_fences() {
        let input = "Before < and {\n~~~solidity\nif (a < b) { revert(); }\n~~~\nAfter < and {";
        let output = replace_description_links(
            input,
            &NameToPage::new(),
            Path::new("src/contract.Foo.mdx"),
            None,
        );

        assert_eq!(
            output,
            "Before &lt; and &#123;\n~~~solidity\nif (a < b) { revert(); }\n~~~\nAfter &lt; and &#123;"
        );
    }

    #[test]
    fn conservatively_escapes_non_standalone_fences() {
        for (input, expected) in [
            (
                "- ~~~\n  example <\n  ~~~\nOutside < and {",
                "- &#126;~~\n  example &lt;\n  &#126;~~\nOutside &lt; and &#123;",
            ),
            ("~~~\nexample < and {", "&#126;~~\nexample &lt; and &#123;"),
        ] {
            assert_eq!(
                replace_description_links(
                    input,
                    &NameToPage::new(),
                    Path::new("src/contract.Foo.mdx"),
                    None,
                ),
                expected
            );
        }
    }

    #[test]
    fn rejected_fences_cannot_change_later_mdx_context() {
        let name_to_page = NameToPage::new();
        let path = Path::new("src/contract.Foo.mdx");
        let first = replace_description_links("~~~", &name_to_page, path, None);
        let second = replace_description_links("~~~\n{1+1}\n~~~", &name_to_page, path, None);
        let output = format!("{first}\n\n{second}");
        assert_eq!(output, "&#126;~~\n\n~~~\n{1+1}\n~~~");
        assert!(!contains_mdx_expression(&parse_mdx(&output)));

        let output =
            replace_description_links("~~~\n    ~~~\n{1+1}\n~~~", &name_to_page, path, None);
        assert_eq!(output, "&#126;~~\n    &#126;~~\n`1+1`\n&#126;~~");
        assert!(!contains_mdx_expression(&parse_mdx(&output)));

        let notice = replace_description_links("~~~\n{1+1}\n~~~", &name_to_page, path, None);
        for prefix in ["**Title:**", "**Author:**", "- **note:**"] {
            let metadata = sanitize_description_prose("metadata\n~~~", &name_to_page, path, None);
            let output = format!("{prefix} {metadata}\n\n{notice}");
            assert!(!contains_mdx_expression(&parse_mdx(&output)), "{output}");
        }
    }

    #[test]
    fn preserves_line_endings_while_neutralizing_esm() {
        assert_eq!(
            neutralize_esm("Intro.\r\rexport const afterCarriageReturn = 1"),
            "Intro.\r\r&#101;xport const afterCarriageReturn = 1"
        );
        for (input, expected) in [
            (
                "```\rimport inside\r```\rexport outside",
                "```\rimport inside\r```\r&#101;xport outside",
            ),
            (
                "```\r\nimport inside\r\n```\r\nexport outside",
                "```\r\nimport inside\r\n```\r\n&#101;xport outside",
            ),
            (
                "`example:\rimport inside`\rexport outside",
                "`example:\rimport inside`\r&#101;xport outside",
            ),
            (
                "`example:\r\nimport inside`\r\nexport outside",
                "`example:\r\nimport inside`\r\n&#101;xport outside",
            ),
            (
                "    ```\r    import inside\r    ```\rexport outside",
                "    ```\r    import inside\r    ```\r&#101;xport outside",
            ),
            ("```\rinside\r    ```\rexport outside", "```\rinside\r    ```\r&#101;xport outside"),
        ] {
            assert_eq!(neutralize_esm(input), expected);
        }
        assert_eq!(
            neutralize_esm("Intro.\r\nimport outside"),
            "Intro.\r\n&#105;&#109;port outside"
        );
        assert_eq!(
            neutralize_esm("export first\r\npréface `span:\rimport inside`\r\nimport last"),
            "&#101;xport first\r\npréface `span:\rimport inside`\r\n&#105;&#109;port last"
        );
    }

    #[test]
    fn traverses_sorted_code_regions() {
        let input = "`first`\n    ```\n    import inside fence\n    ```\n`last`\nexport outside";
        let expected =
            "`first`\n    ```\n    import inside fence\n    ```\n`last`\n&#101;xport outside";
        assert_eq!(neutralize_esm(input), expected);
    }

    #[test]
    fn preserves_code_across_mdx_edge_cases() {
        for (input, expected) in [
            (
                "```\nimport inside\n```\nexport outside",
                "```\nimport inside\n```\n&#101;xport outside",
            ),
            (
                "```\n~~~\nimport inside\n```\nexport outside",
                "```\n~~~\nimport inside\n```\n&#101;xport outside",
            ),
            (
                "````\n```\nimport inside\n```\n````\nexport outside",
                "````\n```\nimport inside\n```\n````\n&#101;xport outside",
            ),
            (
                "```\nimport inside\n```suffix\nexport inside\n```\nimport outside",
                "```\nimport inside\n```suffix\nexport inside\n```\n&#105;&#109;port outside",
            ),
            (
                "`example:\nimport inside`\nexport outside",
                "`example:\nimport inside`\n&#101;xport outside",
            ),
            (
                "```\ninside\n    ```\n```\nimport inside second\n```\nexport outside",
                "```\ninside\n    ```\n```\nimport inside second\n```\n&#101;xport outside",
            ),
            (
                "- ```\n  import inside list\n  ```\n\nexport outside",
                "- ```\n  import inside list\n  ```\n\n&#101;xport outside",
            ),
            (
                "    ```\n    import inside indented\n    ```\nexport outside",
                "    ```\n    import inside indented\n    ```\n&#101;xport outside",
            ),
            ("    ```\n    import inside unclosed", "    ```\n    import inside unclosed"),
        ] {
            assert_eq!(neutralize_esm(input), expected, "input:\n{input}");
        }
    }

    #[test]
    fn neutralized_output_contains_no_mdx_esm() {
        let input = "import injected from \"x\"\n\n```js\nexport const example = 1\n```";
        assert!(contains_mdx_esm(&parse_mdx(input)));

        let output = neutralize_esm(input);
        assert_eq!(
            output,
            "&#105;&#109;port injected from \"x\"\n\n```js\nexport const example = 1\n```"
        );

        let tree = parse_mdx(&output);
        assert!(!contains_mdx_esm(&tree), "{tree:#?}");
        assert!(tree.children().is_some_and(|children| {
            children.iter().any(
                |node| matches!(node, Node::Code(code) if code.value == "export const example = 1"),
            )
        }));
    }

    #[test]
    fn leaves_non_esm_prefixes_unchanged() {
        let input = "important\nexporter\nimport_\nexport$\nImport value\nExport value\n    import value\n\t export value\nimport(value)\nexport: value\nimport\tvalue";
        assert_eq!(neutralize_esm(input), input);
    }

    #[test]
    fn neutralizes_only_exact_mdx_esm_prefixes() {
        assert_eq!(
            neutralize_esm("import value\nexport value\nimport  value\nexport  value"),
            "&#105;&#109;port value\n&#101;xport value\n&#105;&#109;port  value\n&#101;xport  value"
        );
    }

    #[test]
    fn neutralizes_many_candidates_across_many_code_regions() {
        let input = "`code`\nexport outside\n".repeat(10_000);
        let output = neutralize_esm(&input);
        assert_eq!(output.matches("`code`").count(), 10_000);
        assert_eq!(output.matches("&#101;xport outside").count(), 10_000);
    }
}
