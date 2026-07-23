//! Rendering: `solar` AST -> vocs MDX.

use crate::{
    hir_ext::{self, NameToPage, clean_block_doc_content},
    utils::Deployment,
};
use foundry_common::sh_warn;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use solar::{
    ast::{
        CommentKind, ContractKind, DocComments, FunctionKind, ItemContract, ItemEnum, ItemError,
        ItemEvent, ItemFunction, ItemKind, ItemStruct, ItemUdvt, NatSpecKind, ParameterList,
        SourceUnit, Span, VariableDefinition,
    },
    interface::{
        Ident,
        source_map::{FileName, SourceFile, SourceMap},
    },
    sema::{Gcx, hir},
};
use std::{
    borrow::Cow,
    collections::HashMap,
    fmt::Write as _,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

// ── public entry point ───────────────────────────────────────────────────────

/// Render a single Solidity source file as a list of `(relative_output_path, mdx_content)` pairs.
#[allow(clippy::too_many_arguments)]
pub fn source<'ast, 'gcx>(
    ast: &'ast SourceUnit<'ast>,
    file: &Arc<SourceFile>,
    _sm: &SourceMap,
    rel_sol_path: &Path,
    abs_sol_path: &Path,
    _root: &Path,
    gcx: Gcx<'gcx>,
    name_to_page: &NameToPage,
    git_url: Option<&str>,
    deployments: &HashMap<String, Vec<Deployment>>,
) -> Vec<(PathBuf, String)> {
    let out_dir = rel_sol_path.parent().unwrap_or(Path::new(""));
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
                let fname = format!("{kind_str}.{}.mdx", c.name.as_str());
                let page_path = out_dir.join(&fname);
                // Look up HIR contract id for inheritance/inheritdoc.
                let hir_id = find_contract_id(gcx, c.name.as_str(), abs_sol_path);
                // Deployments only apply to non-abstract, non-interface, non-library contracts.
                let contract_deployments = if matches!(c.kind, ContractKind::Contract) {
                    deployments.get(c.name.as_str()).map(Vec::as_slice).unwrap_or(&[])
                } else {
                    &[]
                };
                let content = render_contract(
                    span,
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

            ItemKind::Struct(s) => {
                let fname = format!("struct.{}.mdx", s.name.as_str());
                let page_path = out_dir.join(&fname);
                pages.push((
                    page_path.clone(),
                    render_struct(span, s, &item.docs, &ctx, name_to_page, &page_path, git_url),
                ));
            }

            ItemKind::Enum(e) => {
                let fname = format!("enum.{}.mdx", e.name.as_str());
                let page_path = out_dir.join(&fname);
                pages.push((
                    page_path.clone(),
                    render_enum(span, e, &item.docs, &ctx, name_to_page, &page_path, git_url),
                ));
            }

            ItemKind::Udvt(u) => {
                let fname = format!("type.{}.mdx", u.name.as_str());
                let page_path = out_dir.join(&fname);
                pages.push((
                    page_path.clone(),
                    render_udvt(span, u, &item.docs, &ctx, name_to_page, &page_path, git_url),
                ));
            }

            ItemKind::Error(e) => {
                let fname = format!("error.{}.mdx", e.name.as_str());
                let page_path = out_dir.join(&fname);
                pages.push((
                    page_path.clone(),
                    render_error(span, e, &item.docs, &ctx, name_to_page, &page_path, git_url),
                ));
            }

            ItemKind::Event(e) => {
                let fname = format!("event.{}.mdx", e.name.as_str());
                let page_path = out_dir.join(&fname);
                pages.push((
                    page_path.clone(),
                    render_event(span, e, &item.docs, &ctx, name_to_page, &page_path, git_url),
                ));
            }
        }
    }

    for (name, overloads) in &free_fns {
        let fname = format!("function.{name}.mdx");
        let page_path = out_dir.join(&fname);
        let content =
            render_free_functions(name, overloads, &ctx, name_to_page, &page_path, git_url);
        pages.push((page_path, content));
    }

    if !const_vars.is_empty() {
        let fname = format!("constants.{stem}.mdx");
        let page_path = out_dir.join(&fname);
        let content = render_constants(stem, &const_vars, &ctx, name_to_page, &page_path, git_url);
        pages.push((page_path, content));
    }

    pages
}

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

// ── contract ─────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_contract<'ast, 'gcx>(
    _span: Span,
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
    let local = Some(&local);

    let comments = collect_comments(docs, name_to_page, page_path, local);
    let mut out = String::new();
    write_frontmatter(&mut out, name, first_notice(&comments).as_deref());
    writeln!(out, "# {name}").unwrap();
    writeln!(out).unwrap();
    write_git_source(&mut out, git_url);
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
    let mut events: Vec<(Span, &ItemEvent<'_>, &DocComments<'_>)> = Vec::new();
    let mut errors: Vec<(Span, &ItemError<'_>, &DocComments<'_>)> = Vec::new();
    let mut structs: Vec<(Span, &ItemStruct<'_>, &DocComments<'_>)> = Vec::new();
    let mut enums: Vec<(Span, &ItemEnum<'_>, &DocComments<'_>)> = Vec::new();
    let mut udvts: Vec<(Span, &ItemUdvt<'_>, &DocComments<'_>)> = Vec::new();

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
            ItemKind::Event(e) => events.push((s, e, &member.docs)),
            ItemKind::Error(e) => errors.push((s, e, &member.docs)),
            ItemKind::Struct(st) => structs.push((s, st, &member.docs)),
            ItemKind::Enum(e) => enums.push((s, e, &member.docs)),
            ItemKind::Udvt(u) => udvts.push((s, u, &member.docs)),
            _ => {}
        }
    }

    let write_vars =
        |out: &mut String, vars: &[(Span, &VariableDefinition<'_>, &DocComments<'_>)]| {
            for (span, v, docs) in vars {
                let vname = v.name.map(|n| n.as_str().to_string()).unwrap_or_default();
                writeln!(out, "### {vname}").unwrap();
                writeln!(out).unwrap();
                let mut c = collect_comments(docs, name_to_page, page_path, local);
                // Attempt @inheritdoc resolution for public state variables.
                let inherited = inheritdoc_base(docs).and_then(|base| {
                    hir_id.and_then(|cid| hir_ext::resolve_inheritdoc_var(gcx, cid, &vname, &base))
                });
                if let Some(ref base_doc) = inherited {
                    let sanitize =
                        |s: &str| hir_ext::replace_inline_links(s, name_to_page, page_path, local);
                    if c.notices.is_empty() {
                        let inherited_notices: Vec<String> =
                            base_doc.notices.iter().map(|s| sanitize(s)).collect();
                        let mut new_desc: Vec<Description> = inherited_notices
                            .iter()
                            .map(|s| Description { kind: DescKind::Notice, content: s.clone() })
                            .collect();
                        new_desc.append(&mut c.descriptions);
                        c.descriptions = new_desc;
                        c.notices.extend(inherited_notices);
                    }
                    if c.devs.is_empty() {
                        let inherited_devs: Vec<String> =
                            base_doc.devs.iter().map(|s| sanitize(s)).collect();
                        c.descriptions.extend(
                            inherited_devs
                                .iter()
                                .map(|s| Description { kind: DescKind::Dev, content: s.clone() }),
                        );
                        c.devs.extend(inherited_devs);
                    }
                    for (name, desc) in &base_doc.returns {
                        if !c.returns.iter().any(|(n, _)| n == name) {
                            c.returns.push((name.clone(), sanitize(desc)));
                        }
                    }
                }
                write_comment_block(out, &c);
                write_code_block(out, &ctx.dedented_snippet(*span));
                if !c.returns.is_empty() {
                    let ty = format!("`{}`", ctx.snippet(v.ty.span).trim());
                    writeln!(out, "**Returns**").unwrap();
                    writeln!(out).unwrap();
                    writeln!(out, "| Name | Type | Description |").unwrap();
                    writeln!(out, "| ---- | ---- | ----------- |").unwrap();
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

    if !constants.is_empty() {
        writeln!(out, "## Constants").unwrap();
        writeln!(out).unwrap();
        write_vars(&mut out, &constants);
    }

    if !state_vars.is_empty() {
        writeln!(out, "## State Variables").unwrap();
        writeln!(out).unwrap();
        write_vars(&mut out, &state_vars);
    }

    if !functions.is_empty() {
        writeln!(out, "## Functions").unwrap();
        writeln!(out).unwrap();
        for (span, f, docs) in &functions {
            let fn_name = f.header.name.map(|n| n.as_str().to_string());
            // Attempt inheritdoc resolution.
            let inherited = fn_name.as_deref().and_then(|fname| {
                let base = inheritdoc_base(docs)?;
                let param_types = hir_id
                    .and_then(|cid| {
                        let fid = gcx.hir.contract(cid).items.iter().find_map(|&item| match item {
                            hir::ItemId::Function(id) if gcx.hir.function(id).span == *span => {
                                Some(id)
                            }
                            _ => None,
                        });
                        if fid.is_none() {
                            let _ = sh_warn!(
                                "forge doc: failed to find HIR function for `{}.{fname}` while resolving @inheritdoc",
                                c.name
                            );
                        }
                        fid
                    })
                    .and_then(|fid| hir_ext::function_param_types(gcx, fid));
                hir_id.and_then(|cid| {
                    hir_ext::resolve_inheritdoc(gcx, cid, fname, &base, param_types.as_deref())
                })
            });
            render_function_section(
                &mut out,
                *span,
                f,
                docs,
                ctx,
                name_to_page,
                page_path,
                local,
                inherited.as_ref(),
            );
        }
    }

    if !events.is_empty() {
        writeln!(out, "## Events").unwrap();
        writeln!(out).unwrap();
        for (span, e, docs) in &events {
            writeln!(out, "### {}", e.name.as_str()).unwrap();
            writeln!(out).unwrap();
            let c = collect_comments(docs, name_to_page, page_path, local);
            write_comment_block(&mut out, &c);
            write_code_block(&mut out, &ctx.dedented_snippet(*span));
            write_param_table(&mut out, "Parameters", &e.parameters, &c, ctx);
        }
    }

    if !errors.is_empty() {
        writeln!(out, "## Errors").unwrap();
        writeln!(out).unwrap();
        for (span, e, docs) in &errors {
            writeln!(out, "### {}", e.name.as_str()).unwrap();
            writeln!(out).unwrap();
            let c = collect_comments(docs, name_to_page, page_path, local);
            write_comment_block(&mut out, &c);
            write_code_block(&mut out, &ctx.dedented_snippet(*span));
            write_param_table(&mut out, "Parameters", &e.parameters, &c, ctx);
        }
    }

    if !structs.is_empty() {
        writeln!(out, "## Structs").unwrap();
        writeln!(out).unwrap();
        for (span, s, docs) in &structs {
            writeln!(out, "### {}", s.name.as_str()).unwrap();
            writeln!(out).unwrap();
            let c = collect_comments(docs, name_to_page, page_path, local);
            write_comment_block(&mut out, &c);
            write_code_block(&mut out, &ctx.dedented_snippet(*span));
            write_struct_properties_table(&mut out, s.fields, &c, ctx);
        }
    }

    if !enums.is_empty() {
        writeln!(out, "## Enums").unwrap();
        writeln!(out).unwrap();
        for (span, e, docs) in &enums {
            writeln!(out, "### {}", e.name.as_str()).unwrap();
            writeln!(out).unwrap();
            let c = collect_comments(docs, name_to_page, page_path, local);
            write_comment_block(&mut out, &c);
            write_code_block(&mut out, &ctx.dedented_snippet(*span));
            write_enum_variants_table(&mut out, e.variants, &c);
        }
    }

    if !udvts.is_empty() {
        writeln!(out, "## Custom Types").unwrap();
        writeln!(out).unwrap();
        for (span, u, docs) in &udvts {
            writeln!(out, "### {}", u.name.as_str()).unwrap();
            writeln!(out).unwrap();
            let c = collect_comments(docs, name_to_page, page_path, local);
            write_comment_block(&mut out, &c);
            write_code_block(&mut out, &format!("{};", ctx.dedented_snippet(*span)));
        }
    }

    out
}

// ── free functions ────────────────────────────────────────────────────────────

fn render_free_functions(
    name: &str,
    overloads: &[(Span, &ItemFunction<'_>, &DocComments<'_>)],
    ctx: &Ctx<'_>,
    name_to_page: &NameToPage,
    page_path: &Path,
    git_url: Option<&str>,
) -> String {
    let title = if name.is_empty() { "function" } else { name };
    let first_comments = collect_comments(overloads[0].2, name_to_page, page_path, None);
    let mut out = String::new();
    write_frontmatter(&mut out, title, first_notice(&first_comments).as_deref());
    writeln!(out, "# {title}").unwrap();
    writeln!(out).unwrap();
    write_git_source(&mut out, git_url);
    for (span, f, docs) in overloads {
        render_function_section(&mut out, *span, f, docs, ctx, name_to_page, page_path, None, None);
    }
    out
}

// ── constants ─────────────────────────────────────────────────────────────────

fn render_constants(
    stem: &str,
    vars: &[(Span, &VariableDefinition<'_>, &DocComments<'_>)],
    ctx: &Ctx<'_>,
    name_to_page: &NameToPage,
    page_path: &Path,
    git_url: Option<&str>,
) -> String {
    let title = format!("{stem} Constants");
    let mut out = String::new();
    write_frontmatter(&mut out, &title, None);
    writeln!(out, "# {title}").unwrap();
    writeln!(out).unwrap();
    write_git_source(&mut out, git_url);
    for (span, v, docs) in vars {
        let name = v.name.map(|n| n.as_str().to_string()).unwrap_or_else(|| "_".to_string());
        writeln!(out, "## {name}").unwrap();
        writeln!(out).unwrap();
        let c = collect_comments(docs, name_to_page, page_path, None);
        write_comment_block(&mut out, &c);
        write_code_block(&mut out, &ctx.dedented_snippet(*span));
    }
    out
}

// ── standalone items ──────────────────────────────────────────────────────────

fn render_struct<'ast>(
    span: Span,
    s: &'ast ItemStruct<'ast>,
    docs: &'ast DocComments<'ast>,
    ctx: &Ctx<'_>,
    name_to_page: &NameToPage,
    page_path: &Path,
    git_url: Option<&str>,
) -> String {
    let name = s.name.as_str();
    let c = collect_comments(docs, name_to_page, page_path, None);
    let mut out = String::new();
    write_frontmatter(&mut out, name, first_notice(&c).as_deref());
    writeln!(out, "# {name}").unwrap();
    writeln!(out).unwrap();
    write_git_source(&mut out, git_url);
    write_comment_block(&mut out, &c);
    write_code_block(&mut out, &ctx.dedented_snippet(span));
    write_struct_properties_table(&mut out, s.fields, &c, ctx);
    out
}

fn render_enum<'ast>(
    span: Span,
    e: &'ast ItemEnum<'ast>,
    docs: &'ast DocComments<'ast>,
    ctx: &Ctx<'_>,
    name_to_page: &NameToPage,
    page_path: &Path,
    git_url: Option<&str>,
) -> String {
    let name = e.name.as_str();
    let c = collect_comments(docs, name_to_page, page_path, None);
    let mut out = String::new();
    write_frontmatter(&mut out, name, first_notice(&c).as_deref());
    writeln!(out, "# {name}").unwrap();
    writeln!(out).unwrap();
    write_git_source(&mut out, git_url);
    write_comment_block(&mut out, &c);
    write_code_block(&mut out, &ctx.dedented_snippet(span));
    write_enum_variants_table(&mut out, e.variants, &c);
    out
}

fn render_udvt<'ast>(
    span: Span,
    u: &'ast ItemUdvt<'ast>,
    docs: &'ast DocComments<'ast>,
    ctx: &Ctx<'_>,
    name_to_page: &NameToPage,
    page_path: &Path,
    git_url: Option<&str>,
) -> String {
    let name = u.name.as_str();
    let c = collect_comments(docs, name_to_page, page_path, None);
    let mut out = String::new();
    write_frontmatter(&mut out, name, first_notice(&c).as_deref());
    writeln!(out, "# {name}").unwrap();
    writeln!(out).unwrap();
    write_git_source(&mut out, git_url);
    write_comment_block(&mut out, &c);
    write_code_block(&mut out, &format!("{};", ctx.dedented_snippet(span)));
    out
}

fn render_error<'ast>(
    span: Span,
    e: &'ast ItemError<'ast>,
    docs: &'ast DocComments<'ast>,
    ctx: &Ctx<'_>,
    name_to_page: &NameToPage,
    page_path: &Path,
    git_url: Option<&str>,
) -> String {
    let name = e.name.as_str();
    let c = collect_comments(docs, name_to_page, page_path, None);
    let mut out = String::new();
    write_frontmatter(&mut out, name, first_notice(&c).as_deref());
    writeln!(out, "# {name}").unwrap();
    writeln!(out).unwrap();
    write_git_source(&mut out, git_url);
    write_comment_block(&mut out, &c);
    write_code_block(&mut out, &ctx.dedented_snippet(span));
    write_param_table(&mut out, "Parameters", &e.parameters, &c, ctx);
    out
}

fn render_event<'ast>(
    span: Span,
    e: &'ast ItemEvent<'ast>,
    docs: &'ast DocComments<'ast>,
    ctx: &Ctx<'_>,
    name_to_page: &NameToPage,
    page_path: &Path,
    git_url: Option<&str>,
) -> String {
    let name = e.name.as_str();
    let c = collect_comments(docs, name_to_page, page_path, None);
    let mut out = String::new();
    write_frontmatter(&mut out, name, first_notice(&c).as_deref());
    writeln!(out, "# {name}").unwrap();
    writeln!(out).unwrap();
    write_git_source(&mut out, git_url);
    write_comment_block(&mut out, &c);
    write_code_block(&mut out, &ctx.dedented_snippet(span));
    write_param_table(&mut out, "Parameters", &e.parameters, &c, ctx);
    out
}

// ── function section ──────────────────────────────────────────────────────────
#[allow(clippy::too_many_arguments)]
fn render_function_section(
    out: &mut String,
    span: Span,
    f: &ItemFunction<'_>,
    docs: &DocComments<'_>,
    ctx: &Ctx<'_>,
    name_to_page: &NameToPage,
    page_path: &Path,
    local: Option<&hir_ext::LocalMembers>,
    inherited: Option<&hir_ext::InheritedDoc>,
) {
    let heading = function_heading(f);
    if let Some(anchor) = function_signature_anchor(f, ctx) {
        writeln!(out, "<a id=\"{anchor}\"></a>").unwrap();
        writeln!(out).unwrap();
    }
    writeln!(out, "### {heading}").unwrap();
    writeln!(out).unwrap();
    let mut c = collect_comments(docs, name_to_page, page_path, local);
    // Merge inherited natspec for missing tags.
    if let Some(inherited) = inherited {
        let sanitize = |s: &str| hir_ext::replace_inline_links(s, name_to_page, page_path, local);
        let inherited_notices: Vec<String> =
            inherited.notices.iter().map(|s| sanitize(s)).collect();
        let inherited_devs: Vec<String> = inherited.devs.iter().map(|s| sanitize(s)).collect();
        if c.notices.is_empty() {
            let mut new_desc: Vec<Description> = inherited_notices
                .iter()
                .map(|s| Description { kind: DescKind::Notice, content: s.clone() })
                .collect();
            new_desc.append(&mut c.descriptions);
            c.descriptions = new_desc;
            c.notices.extend_from_slice(&inherited_notices);
        }
        if c.devs.is_empty() {
            c.devs.extend_from_slice(&inherited_devs);
            c.descriptions.extend(
                inherited_devs
                    .iter()
                    .map(|s| Description { kind: DescKind::Dev, content: s.clone() }),
            );
        }
        for (name, desc) in &inherited.params {
            let name = inherited_param_name_for_current_function(
                name,
                &f.header.parameters,
                &inherited.params,
            );
            if !c.params.iter().any(|(n, _)| n == &name) {
                c.params.push((name, sanitize(desc)));
            }
        }
        for (name, desc) in &inherited.returns {
            if !c.returns.iter().any(|(n, _)| n == name) {
                c.returns.push((name.clone(), sanitize(desc)));
            }
        }
    }
    write_comment_block(out, &c);
    let hspan = if f.header.span.lo() == f.header.span.hi() { span } else { f.header.span };
    let snippet = ctx.dedented_snippet(hspan);
    write_code_block(out, &format!("{snippet};"));
    write_param_table(out, "Parameters", &f.header.parameters, &c, ctx);
    if let Some(returns) = &f.header.returns {
        write_param_table(out, "Returns", returns, &c, ctx);
    }
}

fn inherited_param_name_for_current_function(
    inherited_name: &str,
    params: &ParameterList<'_>,
    inherited_params: &[(String, String)],
) -> String {
    if params
        .iter()
        .any(|param| param.name.map(|name| name.as_str() == inherited_name).unwrap_or(false))
    {
        return inherited_name.to_string();
    }

    let normalized_inherited = inherited_name.trim_matches('_');
    if normalized_inherited.is_empty() {
        return inherited_name.to_string();
    }

    if inherited_params
        .iter()
        .filter(|(name, _)| name.trim_matches('_') == normalized_inherited)
        .take(2)
        .count()
        != 1
    {
        return inherited_name.to_string();
    }

    let mut candidates = params.iter().filter_map(|param| {
        let name = param.name?;
        let name = name.as_str();
        (name.trim_matches('_') == normalized_inherited).then(|| name.to_string())
    });

    match (candidates.next(), candidates.next()) {
        (Some(name), None) => name,
        _ => inherited_name.to_string(),
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

struct CommentData {
    titles: Vec<String>,
    authors: Vec<String>,
    /// Real `@notice` items (used for inheritdoc merging and frontmatter description).
    notices: Vec<String>,
    /// Real `@dev` items (used for inheritdoc merging).
    devs: Vec<String>,
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

/// Collect natspec from doc comments, applying inline link replacement.
///
/// Solar emits each `///` line as a separate `DocComment`. Lines without a `@` tag become
/// synthetic `@notice` items with leading whitespace in their raw content. We detect these
/// continuation lines and join them to the previous description paragraph so that multi-line
/// natspec tags appear as a single coherent block in the right source order.
fn collect_comments(
    docs: &DocComments<'_>,
    name_to_page: &NameToPage,
    page_path: &Path,
    local: Option<&hir_ext::LocalMembers>,
) -> CommentData {
    let mut data = CommentData {
        titles: Vec::new(),
        authors: Vec::new(),
        notices: Vec::new(),
        devs: Vec::new(),
        descriptions: Vec::new(),
        params: Vec::new(),
        returns: Vec::new(),
        customs: Vec::new(),
        unnamed_param_names: Vec::new(),
    };

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

            // Detect a solar "synthetic" @notice: a continuation line with no `@` tag.
            // Solar produces these when a `///` line has no tag; the raw content starts
            // with whitespace (the indentation after `///`).
            let is_continuation = matches!(item.kind, NatSpecKind::Notice)
                && raw.starts_with(|c: char| c.is_whitespace());

            let trimmed = raw.trim();
            if trimmed.is_empty() {
                prev_doc_was_blank = true;
                continue;
            }

            // Apply inline {Ident} -> markdown link replacement.
            let content = hir_ext::replace_inline_links(trimmed, name_to_page, page_path, local);

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
                    data.notices.push(content.clone());
                    data.descriptions.push(Description { kind: DescKind::Notice, content });
                    last_section = Some(LastSection::Desc);
                }
                NatSpecKind::Dev => {
                    data.devs.push(content.clone());
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
fn inheritdoc_base(docs: &DocComments<'_>) -> Option<String> {
    for doc in docs.iter() {
        for item in doc.natspec.iter() {
            if let NatSpecKind::Inheritdoc { contract } = item.kind {
                return Some(contract.as_str().to_string());
            }
        }
    }
    None
}

fn first_notice(data: &CommentData) -> Option<String> {
    data.notices.first().cloned()
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

/// Byte ranges that MDX parses as code. Pulldown-cmark reports CommonMark fences and inline code
/// spans, including multi-line ones. MDX-only fences indented by four or more columns are added
/// separately. An HTML entity would render literally inside these ranges, so neutralization skips
/// them.
fn code_regions(text: &str) -> Vec<Range<usize>> {
    let (markdown, collapsed_crlf_ends) = markdown_parse_source(text);
    let mut regions = Vec::new();
    let mut fenced_start: Option<usize> = None;
    for (event, range) in Parser::new_ext(&markdown, Options::empty()).into_offset_iter() {
        let range = original_offset(range.start, &collapsed_crlf_ends)
            ..original_offset(range.end, &collapsed_crlf_ends);
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                fenced_start = Some(range.start);
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = fenced_start.take() {
                    regions.push(start..mdx_fence_end(text, start, range.end));
                }
            }
            Event::Code(_) => regions.push(range),
            _ => {}
        }
    }
    let mut regions = merge_regions(regions);
    let indented_fences = mdx_indented_fence_regions(text, &regions);
    regions.extend(indented_fences);
    merge_regions(regions)
}

/// Normalize line endings only for Markdown parsing and record where CRLF pairs were contracted.
/// Pulldown-cmark then sees CR, CRLF, and LF as equivalent, while the sparse offset map lets code
/// regions refer back to the unchanged source text.
fn markdown_parse_source(text: &str) -> (Cow<'_, str>, Vec<usize>) {
    if !text.contains('\r') {
        return (Cow::Borrowed(text), Vec::new());
    }

    let bytes = text.as_bytes();
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut collapsed_crlf_ends = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset] == b'\r' {
            normalized.push(b'\n');
            offset += 1;
            if bytes.get(offset) == Some(&b'\n') {
                offset += 1;
                collapsed_crlf_ends.push(normalized.len());
            }
        } else {
            normalized.push(bytes[offset]);
            offset += 1;
        }
    }

    (
        Cow::Owned(String::from_utf8(normalized).expect("normalization preserves UTF-8")),
        collapsed_crlf_ends,
    )
}

/// Translate a byte offset in normalized Markdown back to the original text. Every contracted CRLF
/// before the boundary contributes one additional source byte.
fn original_offset(normalized: usize, collapsed_crlf_ends: &[usize]) -> usize {
    normalized + collapsed_crlf_ends.partition_point(|&end| end <= normalized)
}

/// Sort and merge overlapping or adjacent code regions so they can be traversed with one cursor.
fn merge_regions(mut regions: Vec<Range<usize>>) -> Vec<Range<usize>> {
    regions.sort_unstable_by_key(|region| region.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(regions.len());
    for region in regions {
        if let Some(previous) = merged.last_mut()
            && region.start <= previous.end
        {
            previous.end = previous.end.max(region.end);
        } else {
            merged.push(region);
        }
    }
    merged
}

/// Logical lines and their byte offsets in the original text. CRLF is one separator; lone CR and
/// LF are separators too. The separator bytes are excluded from the returned slices and preserved
/// in the source string.
fn logical_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
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
fn region_contains(regions: &[Range<usize>], cursor: &mut usize, position: usize) -> bool {
    while regions.get(*cursor).is_some_and(|region| region.end <= position) {
        *cursor += 1;
    }
    regions.get(*cursor).is_some_and(|region| region.start <= position)
}

/// Fenced code blocks that MDX recognizes but CommonMark does not: opening fences indented by
/// four or more columns. Once opened, a matching closer may use any indentation; without one the
/// block extends to EOF.
fn mdx_indented_fence_regions(text: &str, parsed_regions: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut regions = Vec::new();
    let mut open: Option<(usize, char, usize)> = None;
    let mut parsed_cursor = 0;

    for (offset, line) in logical_lines(text) {
        let line_end = offset + line.len();
        let marker = line.trim_start_matches([' ', '\t']);

        if let Some((start, fence, open_len)) = open {
            let run = marker.chars().take_while(|&c| c == fence).count();
            if run >= open_len && marker[run..].trim_matches([' ', '\t']).is_empty() {
                regions.push(start..line_end);
                open = None;
            }
        } else {
            let indent = &line[..line.len() - marker.len()];
            let indent_columns = indent.bytes().fold(0, |column, byte| match byte {
                b'\t' => column + 4 - column % 4,
                _ => column + 1,
            });
            if indent_columns >= 4
                && let Some(fence) = marker.chars().next().filter(|&c| c == '`' || c == '~')
            {
                let open_len = marker.chars().take_while(|&c| c == fence).count();
                let start = offset + indent.len();
                if open_len >= 3
                    && (fence != '`' || !marker[open_len..].contains('`'))
                    && !region_contains(parsed_regions, &mut parsed_cursor, start)
                {
                    open = Some((start, fence, open_len));
                }
            }
        }
    }

    if let Some((start, _, _)) = open {
        regions.push(start..text.len());
    }
    regions
}

/// Where MDX ends the fenced block that pulldown-cmark reports as `start..cmark_end`. MDX
/// (micromark) has no indented code blocks, so it closes a fence at a matching marker at *any*
/// indentation, while CommonMark requires the closer to be indented at most three spaces. When a
/// fence is closed by a more-indented marker, pulldown keeps the block open past it and would hide
/// a following `import`/`export` inside the range; truncating at the first MDX-style closer keeps
/// that statement neutralizable.
fn mdx_fence_end(text: &str, start: usize, cmark_end: usize) -> usize {
    let block = &text[start..cmark_end];
    let mut lines = logical_lines(block);
    let opener = lines.next().map_or("", |(_, line)| line).trim_start();
    let Some(fence) = opener.chars().next().filter(|&c| c == '`' || c == '~') else {
        return cmark_end;
    };
    let open_len = opener.chars().take_while(|&c| c == fence).count();
    for (offset, line) in lines {
        let marker = line.trim_start();
        let run = marker.chars().take_while(|&c| c == fence).count();
        if run >= open_len && run > 0 && marker[run..].trim().is_empty() {
            return start + offset + line.len();
        }
    }
    cmark_end
}

/// Neutralize any line MDX would parse as an ESM statement (`import`/`export` at the start of
/// a line): the keyword's first letter becomes an HTML entity, so the line renders the same but
/// no longer begins with an ESM token. NatSpec text can be inherited from a dependency via
/// `@inheritdoc`, so this must run wherever displayed prose is assembled. A keyword that falls
/// inside a Markdown code span or fenced code block is left untouched (see `code_regions`): the
/// entity would render literally and corrupt the example, and MDX would not execute it there.
fn neutralize_esm(text: &str) -> String {
    let regions = code_regions(text);
    let mut region_cursor = 0;
    let mut copied = 0;
    let mut out = String::with_capacity(text.len());

    for (line_start, line) in logical_lines(text) {
        let trimmed = line.trim_start();
        let mut entity = None;
        for (keyword, replacement) in [("import", "&#105;"), ("export", "&#101;")] {
            if let Some(rest) = trimmed.strip_prefix(keyword)
                && !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$')
            {
                entity = Some(replacement);
                break;
            }
        }
        let Some(entity) = entity else { continue };
        let keyword_pos = line_start + (line.len() - trimmed.len());
        if region_contains(&regions, &mut region_cursor, keyword_pos) {
            continue;
        }
        out.push_str(&text[copied..keyword_pos]);
        out.push_str(entity);
        copied = keyword_pos + 1;
    }

    out.push_str(&text[copied..]);
    out
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
    ctx: &Ctx<'_>,
) {
    if params.is_empty() {
        return;
    }
    writeln!(out, "**{heading}**").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "| Name | Type | Description |").unwrap();
    writeln!(out, "| ---- | ---- | ----------- |").unwrap();
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
            comments.params.iter().find(|(n, _)| n == &name).map(|(_, d)| d.as_str()).unwrap_or("")
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
        None => split_first_word(desc).map(|(_, desc)| desc).unwrap_or(desc),
    }
}

fn strip_one_ws(s: &str) -> Option<&str> {
    let mut chars = s.char_indices();
    let (_, first) = chars.next()?;
    first.is_whitespace().then(|| chars.next().map(|(idx, _)| &s[idx..]).unwrap_or(""))
}

fn split_first_word(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_start();
    let split = trimmed.find(char::is_whitespace)?;
    let (first, rest) = trimmed.split_at(split);
    Some((first, rest.trim_start()))
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
    writeln!(out, "**Properties**").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "| Name | Type | Description |").unwrap();
    writeln!(out, "| ---- | ---- | ----------- |").unwrap();
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

const fn contract_kind_str(kind: ContractKind) -> &'static str {
    match kind {
        ContractKind::Contract => "contract",
        ContractKind::AbstractContract => "abstract",
        ContractKind::Interface => "interface",
        ContractKind::Library => "library",
    }
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

#[cfg(test)]
mod tests {
    use super::neutralize_esm;

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
        assert_eq!(neutralize_esm("Intro.\r\nimport outside"), "Intro.\r\n&#105;mport outside");
        assert_eq!(
            neutralize_esm("export first\r\npréface `span:\rimport inside`\r\nimport last"),
            "&#101;xport first\r\npréface `span:\rimport inside`\r\n&#105;mport last"
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
    fn leaves_non_esm_prefixes_unchanged() {
        let input = "important\nexporter\nimport_\nexport$\nImport value\nExport value";
        assert_eq!(neutralize_esm(input), input);
    }

    #[test]
    fn neutralizes_many_candidates_across_many_code_regions() {
        let input = "`code`\nexport outside\n".repeat(10_000);
        let output = neutralize_esm(&input);
        assert_eq!(output.matches("`code`").count(), 10_000);
        assert_eq!(output.matches("&#101;xport outside").count(), 10_000);
    }
}
