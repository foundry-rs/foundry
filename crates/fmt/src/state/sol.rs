#![allow(clippy::too_many_arguments)]

use super::{
    CommentConfig, Separator, State,
    common::{BlockFormat, ListFormat},
};
use crate::{
    pp::SIZE_INFINITY,
    state::{CallContext, common::LitExt},
};
use foundry_common::{comments::Comment, iter::IterDelimited};
use foundry_config::fmt::{self as config, MultilineFuncHeaderStyle};
use solar::parse::{
    ast::{self, Span},
    interface::BytePos,
};
use std::{collections::HashMap, fmt::Debug};

#[rustfmt::skip]
macro_rules! get_span {
    () => { |value| Some(value.span) };
    (()) => { |value| Some(value.span()) };
}

/// Language-specific pretty printing: Solidity.
impl<'ast> State<'_, 'ast> {
    pub(crate) fn print_source_unit(&mut self, source_unit: &'ast ast::SourceUnit<'ast>) {
        let mut items = source_unit.items.iter().peekable();
        let mut is_first = true;
        while let Some(item) = items.next() {
            // If imports shouldn't be sorted, or if the item is not an import, print it directly.
            if !self.config.sort_imports || !matches!(item.kind, ast::ItemKind::Import(_)) {
                self.print_item(item, is_first);
                is_first = false;
                if let Some(next_item) = items.peek() {
                    self.separate_items(next_item, false);
                }
                continue;
            }

            // Otherwise, collect a group of consecutive imports and sort them before printing.
            let mut import_group = vec![item];
            while let Some(next_item) = items.peek() {
                // Groups end when the next item is not an import or when there is a blank line.
                if !matches!(next_item.kind, ast::ItemKind::Import(_))
                    || self.has_comment_between(item.span.hi(), next_item.span.lo())
                {
                    break;
                }
                import_group.push(items.next().unwrap());
            }

            import_group.sort_by_key(|item| {
                if let ast::ItemKind::Import(import) = &item.kind {
                    import.path.value.as_str()
                } else {
                    unreachable!("Expected an import item")
                }
            });

            for (pos, group_item) in import_group.iter().delimited() {
                self.print_item(group_item, is_first);
                is_first = false;

                if !pos.is_last {
                    self.hardbreak_if_not_bol();
                }
            }
            if let Some(next_item) = items.peek() {
                self.separate_items(next_item, false);
            }
        }

        self.print_remaining_comments();
    }

    /// Prints a hardbreak if the item needs an isolated line break.
    fn separate_items(&mut self, next_item: &'ast ast::Item<'ast>, advance: bool) {
        if !item_needs_iso(&next_item.kind) {
            return;
        }
        let span = next_item.span;

        let cmnts = self
            .comments
            .iter()
            .filter_map(|c| if c.pos() < span.lo() { Some(c.style) } else { None })
            .collect::<Vec<_>>();

        if let Some(first) = cmnts.first()
            && let Some(last) = cmnts.last()
        {
            if !(first.is_blank() || last.is_blank()) {
                self.hardbreak();
                return;
            }
            if advance {
                if self.peek_comment_before(span.lo()).is_some() {
                    self.print_comments(span.lo(), CommentConfig::default());
                } else if self.inline_config.is_disabled(span.shrink_to_lo()) {
                    self.hardbreak();
                    self.cursor.advance_to(span.lo(), true);
                }
            }
        } else {
            self.hardbreak();
        }
    }

    fn print_item(&mut self, item: &'ast ast::Item<'ast>, skip_ws: bool) {
        let ast::Item { ref docs, span, ref kind } = *item;
        self.print_docs(docs);

        if self.handle_span(item.span, skip_ws) {
            if !self.print_trailing_comment(span.hi(), None) {
                self.print_sep(Separator::Hardbreak);
            }
            return;
        }

        if self
            .print_comments(
                span.lo(),
                if skip_ws {
                    CommentConfig::skip_leading_ws(false)
                } else {
                    CommentConfig::default()
                },
            )
            .is_some_and(|cmnt| cmnt.is_mixed())
        {
            self.zerobreak();
        }

        match kind {
            ast::ItemKind::Pragma(pragma) => self.print_pragma(pragma),
            ast::ItemKind::Import(import) => self.print_import(import),
            ast::ItemKind::Using(using) => self.print_using(using),
            ast::ItemKind::Contract(contract) => self.print_contract(contract, span),
            ast::ItemKind::Function(func) => self.print_function(func),
            ast::ItemKind::Variable(var) => self.print_var_def(var),
            ast::ItemKind::Struct(strukt) => self.print_struct(strukt, span),
            ast::ItemKind::Enum(enm) => self.print_enum(enm, span),
            ast::ItemKind::Udvt(udvt) => self.print_udvt(udvt),
            ast::ItemKind::Error(err) => self.print_error(err),
            ast::ItemKind::Event(event) => self.print_event(event),
        }

        self.cursor.advance_to(span.hi(), true);
        self.print_comments(span.hi(), CommentConfig::default());
        self.print_trailing_comment(span.hi(), None);
        self.hardbreak_if_not_bol();
        self.cursor.advance(1);
    }

    fn print_pragma(&mut self, pragma: &'ast ast::PragmaDirective<'ast>) {
        self.word("pragma ");
        match &pragma.tokens {
            ast::PragmaTokens::Version(ident, semver_req) => {
                self.print_ident(ident);
                self.nbsp();
                self.word(semver_req.to_string());
            }
            ast::PragmaTokens::Custom(a, b) => {
                self.print_ident_or_strlit(a);
                if let Some(b) = b {
                    self.nbsp();
                    self.print_ident_or_strlit(b);
                }
            }
            ast::PragmaTokens::Verbatim(tokens) => {
                self.print_tokens(tokens);
            }
        }
        self.word(";");
    }

    fn print_commasep_aliases<'a, I>(&mut self, aliases: I)
    where
        I: Iterator<Item = &'a (ast::Ident, Option<ast::Ident>)>,
        'ast: 'a,
    {
        for (pos, (ident, alias)) in aliases.delimited() {
            self.print_ident(ident);
            if let Some(alias) = alias {
                self.word(" as ");
                self.print_ident(alias);
            }
            if !pos.is_last {
                self.word(",");
                self.space();
            }
        }
    }

    fn print_import(&mut self, import: &'ast ast::ImportDirective<'ast>) {
        let ast::ImportDirective { path, items } = import;
        self.word("import ");
        match items {
            ast::ImportItems::Plain(_) | ast::ImportItems::Glob(_) => {
                self.print_ast_str_lit(path);
                if let Some(ident) = items.source_alias() {
                    self.word(" as ");
                    self.print_ident(&ident);
                }
            }

            ast::ImportItems::Aliases(aliases) => {
                self.s.cbox(self.ind);
                self.word("{");
                self.braces_break();

                if self.config.sort_imports {
                    let mut sorted: Vec<_> = aliases.iter().collect();
                    sorted.sort_by_key(|(ident, _alias)| ident.name.as_str());
                    self.print_commasep_aliases(sorted.into_iter());
                } else {
                    self.print_commasep_aliases(aliases.iter());
                };

                self.braces_break();
                self.s.offset(-self.ind);
                self.word("}");
                self.end();
                self.word(" from ");
                self.print_ast_str_lit(path);
            }
        }
        self.word(";");
    }

    fn print_using(&mut self, using: &'ast ast::UsingDirective<'ast>) {
        let ast::UsingDirective { list, ty, global } = using;
        self.word("using ");
        match list {
            ast::UsingList::Single(path) => self.print_path(path, true),
            ast::UsingList::Multiple(items) => {
                self.s.cbox(self.ind);
                self.word("{");
                self.braces_break();
                for (pos, (path, op)) in items.iter().delimited() {
                    self.print_path(path, true);
                    if let Some(op) = op {
                        self.word(" as ");
                        self.word(op.to_str());
                    }
                    if !pos.is_last {
                        self.word(",");
                        self.space();
                    }
                }
                self.braces_break();
                self.s.offset(-self.ind);
                self.word("}");
                self.end();
            }
        }
        self.word(" for ");
        if let Some(ty) = ty {
            self.print_ty(ty);
        } else {
            self.word("*");
        }
        if *global {
            self.word(" global");
        }
        self.word(";");
    }

    fn print_contract(&mut self, c: &'ast ast::ItemContract<'ast>, span: Span) {
        let ast::ItemContract { kind, name, layout, bases, body } = c;
        self.contract = Some(c);
        self.cursor.advance_to(span.lo(), true);

        self.s.cbox(self.ind);
        self.ibox(0);
        self.cbox(0);
        self.word_nbsp(kind.to_str());
        self.print_ident(name);
        self.nbsp();

        if let Some(first) = bases.first().map(|base| base.span())
            && let Some(last) = bases.last().map(|base| base.span())
            && self.inline_config.is_disabled(first.to(last))
        {
            _ = self.handle_span(first.until(last), false);
        } else if !bases.is_empty() {
            self.word("is");
            self.space();
            for (pos, base) in bases.iter().delimited() {
                if !self.handle_span(base.span(), false) {
                    self.print_modifier_call(base, false);
                    if !pos.is_last {
                        self.word(",");
                        self.space();
                    }
                }
            }
            self.space();
            self.s.offset(-self.ind);
        }
        self.end();
        if let Some(layout) = layout
            && !self.handle_span(layout.span, false)
        {
            self.word("layout at ");
            self.print_expr(layout.slot);
            self.print_sep(Separator::Space);
        }

        self.print_word("{");
        self.end();
        if !body.is_empty() {
            self.print_sep(Separator::Hardbreak);
            if self.config.contract_new_lines {
                self.hardbreak();
            }
            let body_lo = body[0].span.lo();
            if self.peek_comment_before(body_lo).is_some() {
                self.print_comments(body_lo, CommentConfig::skip_leading_ws(true));
            }

            let mut is_first = true;
            let mut items = body.iter().peekable();
            while let Some(item) = items.next() {
                self.print_item(item, is_first);
                is_first = false;
                if let Some(next_item) = items.peek() {
                    if self.inline_config.is_disabled(next_item.span) {
                        _ = self.handle_span(next_item.span, false);
                    } else {
                        self.separate_items(next_item, true);
                    }
                }
            }

            if let Some(cmnt) = self.print_comments(span.hi(), CommentConfig::skip_trailing_ws())
                && self.config.contract_new_lines
                && !cmnt.is_blank()
            {
                self.print_sep(Separator::Hardbreak);
            }
            self.s.offset(-self.ind);
            self.end();
            if self.config.contract_new_lines {
                self.hardbreak_if_nonempty();
            }
        } else {
            if self.print_comments(span.hi(), CommentConfig::skip_ws()).is_some() {
                self.zerobreak();
            } else if self.config.bracket_spacing {
                self.nbsp();
            };
            self.end();
        }
        self.print_word("}");

        self.cursor.advance_to(span.hi(), true);
        self.contract = None;
    }

    fn print_struct(&mut self, strukt: &'ast ast::ItemStruct<'ast>, span: Span) {
        let ast::ItemStruct { name, fields } = strukt;
        let ind = if self.estimate_size(name.span) + 8 >= self.space_left() { self.ind } else { 0 };
        self.s.ibox(self.ind);
        self.word("struct");
        self.space();
        self.print_ident(name);
        self.word(" {");
        if !fields.is_empty() {
            self.break_offset(SIZE_INFINITY as usize, ind);
        }
        self.s.ibox(0);
        for var in fields.iter() {
            self.print_var_def(var);
            if !self.print_trailing_comment(var.span.hi(), None) {
                self.hardbreak();
            }
        }
        self.print_comments(span.hi(), CommentConfig::skip_ws());
        if ind == 0 {
            self.s.offset(-self.ind);
        }
        self.end();
        self.end();
        self.word("}");
    }

    fn print_enum(&mut self, enm: &'ast ast::ItemEnum<'ast>, span: Span) {
        let ast::ItemEnum { name, variants } = enm;
        self.s.cbox(self.ind);
        self.word("enum ");
        self.print_ident(name);
        self.word(" {");
        self.hardbreak_if_nonempty();
        for (pos, ident) in variants.iter().delimited() {
            self.print_comments(ident.span.lo(), CommentConfig::default());
            self.print_ident(ident);
            if !pos.is_last {
                self.word(",");
            }
            if !self.print_trailing_comment(ident.span.hi(), None) {
                self.hardbreak();
            }
        }
        self.print_comments(span.hi(), CommentConfig::skip_ws());
        self.s.offset(-self.ind);
        self.end();
        self.word("}");
    }

    fn print_udvt(&mut self, udvt: &'ast ast::ItemUdvt<'ast>) {
        let ast::ItemUdvt { name, ty } = udvt;
        self.word("type ");
        self.print_ident(name);
        self.word(" is ");
        self.print_ty(ty);
        self.word(";");
    }

    // NOTE(rusowsky): Functions are the only source unit item that handle inline (disabled) format
    fn print_function(&mut self, func: &'ast ast::ItemFunction<'ast>) {
        let ast::ItemFunction { kind, ref header, ref body, body_span } = *func;
        let ast::FunctionHeader {
            name,
            ref parameters,
            visibility,
            state_mutability: sm,
            virtual_,
            ref override_,
            ref returns,
            ..
        } = *header;

        self.s.cbox(self.ind);

        // Print fn name and params
        _ = self.handle_span(self.cursor.span(header.span.lo()), false);
        self.print_word(kind.to_str());
        if let Some(name) = name {
            self.print_sep(Separator::Nbsp);
            self.print_ident(&name);
            self.cursor.advance_to(name.span.hi(), true);
        }
        self.s.cbox(-self.ind);
        let header_style = self.config.multiline_func_header;
        let params_format = match header_style {
            MultilineFuncHeaderStyle::ParamsFirst => ListFormat::always_break(),
            MultilineFuncHeaderStyle::AllParams
                if !header.parameters.is_empty() && !self.can_header_be_inlined(header) =>
            {
                ListFormat::always_break()
            }
            _ => ListFormat::consistent().break_cmnts(),
        };
        self.print_parameter_list(parameters, parameters.span, params_format);
        self.end();

        // Map attributes to their corresponding comments
        let (mut map, attributes, first_attrib_pos) =
            AttributeCommentMapper::new(returns.as_ref(), body_span.lo()).build(self, header);

        let mut handle_pre_cmnts = |this: &mut Self, span: Span| -> bool {
            if this.inline_config.is_disabled(span)
                // Note: `map` is still captured from the outer scope, which is fine.
                && let Some((pre_cmnts, ..)) = map.remove(&span.lo())
            {
                for (pos, cmnt) in pre_cmnts.into_iter().delimited() {
                    if pos.is_first && cmnt.style.is_isolated() && !this.is_bol_or_only_ind() {
                        this.print_sep(Separator::Hardbreak);
                    }
                    if let Some(cmnt) = this.handle_comment(cmnt, false) {
                        this.print_comment(cmnt, CommentConfig::skip_ws().mixed_post_nbsp());
                    }
                    if pos.is_last {
                        return true;
                    }
                }
            }
            false
        };

        let skip_attribs = returns.as_ref().is_some_and(|ret| {
            let attrib_span = Span::new(first_attrib_pos, ret.span.lo());
            handle_pre_cmnts(self, attrib_span);
            self.handle_span(attrib_span, false)
        });
        let skip_returns = {
            let pos = if skip_attribs { self.cursor.pos } else { first_attrib_pos };
            let ret_span = Span::new(pos, body_span.lo());
            handle_pre_cmnts(self, ret_span);
            self.handle_span(ret_span, false)
        };

        let attrib_box = self.config.multiline_func_header.params_first()
            || (self.config.multiline_func_header.attrib_first()
                && !self.can_header_params_be_inlined(header));
        if attrib_box {
            self.s.cbox(0);
        }
        if !(skip_attribs || skip_returns) {
            // Print fn attributes in correct order
            if let Some(v) = visibility {
                self.print_fn_attribute(v.span, &mut map, &mut |s| s.word(v.to_str()));
            }
            if let Some(sm) = sm
                && !matches!(*sm, ast::StateMutability::NonPayable)
            {
                self.print_fn_attribute(sm.span, &mut map, &mut |s| s.word(sm.to_str()));
            }
            if let Some(v) = virtual_ {
                self.print_fn_attribute(v, &mut map, &mut |s| s.word("virtual"));
            }
            if let Some(o) = override_ {
                self.print_fn_attribute(o.span, &mut map, &mut |s| s.print_override(o));
            }
            for m in attributes.iter().filter(|a| matches!(a.kind, AttributeKind::Modifier(_))) {
                if let AttributeKind::Modifier(modifier) = m.kind {
                    let is_base = self.is_modifier_a_base_contract(kind, modifier);
                    self.print_fn_attribute(m.span, &mut map, &mut |s| {
                        s.print_modifier_call(modifier, is_base)
                    });
                }
            }
        }
        if !skip_returns
            && let Some(ret) = returns
            && !ret.is_empty()
            && let Some(ret) = returns
        {
            if !self.handle_span(self.cursor.span(ret.span.lo()), false) {
                if !self.is_bol_or_only_ind() && !self.last_token_is_space() {
                    self.print_sep(Separator::Space);
                }
                self.cursor.advance_to(ret.span.lo(), true);
                self.print_word("returns ");
            }
            self.print_parameter_list(
                ret,
                ret.span,
                ListFormat::consistent(), // .with_cmnts_break(false),
            );
        }

        // Print fn body
        if let Some(body) = body {
            // update cache
            let cache = self.fn_body;
            self.fn_body = true;

            if self.handle_span(self.cursor.span(body_span.lo()), false) {
                // Print spacing if necessary. Updates cursor.
            } else {
                if let Some(cmnt) = self.peek_comment_before(body_span.lo()) {
                    if cmnt.style.is_mixed() {
                        // These shouldn't update the cursor, as we've already dealt with it above
                        self.space();
                        self.s.offset(-self.ind);
                        self.print_comments(body_span.lo(), CommentConfig::skip_ws());
                    } else {
                        self.zerobreak();
                        self.s.offset(-self.ind);
                        self.print_comments(body_span.lo(), CommentConfig::skip_ws());
                        self.s.offset(-self.ind);
                    }
                } else {
                    // If there are no modifiers, overrides, nor returns never break
                    if header.modifiers.is_empty()
                        && header.override_.is_none()
                        && returns.as_ref().is_none_or(|r| r.is_empty())
                        && (header.visibility().is_none() || body.is_empty())
                    {
                        self.nbsp();
                    } else {
                        self.space();
                        self.s.offset(-self.ind);
                    }
                }
                self.cursor.advance_to(body_span.lo(), true);
            }
            self.print_word("{");
            self.end();
            if attrib_box {
                self.end();
            }

            self.print_block_without_braces(body, body_span.hi(), Some(self.ind));
            if self.cursor.enabled || self.cursor.pos < body_span.hi() {
                self.print_word("}");
                self.cursor.advance_to(body_span.hi(), true);
            }

            // restore cache
            self.fn_body = cache;
        } else {
            self.print_comments(body_span.lo(), CommentConfig::skip_ws().mixed_prev_space());
            self.end();
            if attrib_box {
                self.end();
            }
            self.neverbreak();
            self.print_word(";");
        }
        self.fn_body = false;

        if let Some(cmnt) = self.peek_trailing_comment(body_span.hi(), None) {
            if cmnt.is_doc {
                // trailing doc comments after the fn body are isolated
                // these shouldn't update the cursor, as this is our own formatting
                self.hardbreak();
                self.hardbreak();
            }
            self.print_trailing_comment(body_span.hi(), None);
        }
    }

    fn print_fn_attribute(
        &mut self,
        span: Span,
        map: &mut AttributeCommentMap,
        print_fn: &mut dyn FnMut(&mut Self),
    ) {
        match map.remove(&span.lo()) {
            Some((pre_cmnts, inner_cmnts, post_cmnts)) => {
                // Print preceding comments.
                for cmnt in pre_cmnts {
                    let Some(cmnt) = self.handle_comment(cmnt, false) else {
                        continue;
                    };
                    self.print_comment(cmnt, CommentConfig::default());
                }
                // Push the inner comments back to the queue, so that they are printed in their
                // intended place.
                for cmnt in inner_cmnts.into_iter().rev() {
                    self.comments.push_front(cmnt);
                }
                let mut enabled = false;
                if !self.handle_span(span, false) {
                    if !self.is_bol_or_only_ind() {
                        self.space();
                    }
                    self.ibox(0);
                    print_fn(self);
                    self.cursor.advance_to(span.hi(), true);
                    enabled = true;
                }
                // Print subsequent comments.
                for cmnt in post_cmnts {
                    let Some(cmnt) = self.handle_comment(cmnt, false) else {
                        continue;
                    };
                    self.print_comment(cmnt, CommentConfig::default().mixed_prev_space());
                }
                if enabled {
                    self.end();
                }
            }
            // Fallback for attributes not in the map (should never happen)
            None => {
                if !self.is_bol_or_only_ind() {
                    self.space();
                }
                print_fn(self);
                self.cursor.advance_to(span.hi(), true);
            }
        }
    }

    fn is_modifier_a_base_contract(
        &self,
        kind: ast::FunctionKind,
        modifier: &'ast ast::Modifier<'ast>,
    ) -> bool {
        // Add `()` in functions when the modifier is a base contract.
        // HACK: heuristics:
        // 1. exactly matches the name of a base contract as declared in the `contract is`;
        // this does not account for inheritance;
        let is_contract_base = self.contract.is_some_and(|contract| {
            contract.bases.iter().any(|contract_base| contract_base.name == modifier.name)
        });
        // 2. assume that title case names in constructors are bases.
        // LEGACY: constructors used to also be `function NameOfContract...`; not checked.
        let is_constructor = matches!(kind, ast::FunctionKind::Constructor);
        // LEGACY: we are checking the beginning of the path, not the last segment.
        is_contract_base
            || (is_constructor
                && modifier.name.first().name.as_str().starts_with(char::is_uppercase))
    }

    fn print_error(&mut self, err: &'ast ast::ItemError<'ast>) {
        let ast::ItemError { name, parameters } = err;
        self.word("error ");
        self.print_ident(name);
        self.print_parameter_list(parameters, parameters.span, ListFormat::compact());
        self.word(";");
    }

    fn print_event(&mut self, event: &'ast ast::ItemEvent<'ast>) {
        let ast::ItemEvent { name, parameters, anonymous } = event;
        self.word("event ");
        self.print_ident(name);
        self.print_parameter_list(parameters, parameters.span, ListFormat::compact().break_cmnts());
        if *anonymous {
            self.word(" anonymous");
        }
        self.word(";");
    }

    fn print_var_def(&mut self, var: &'ast ast::VariableDefinition<'ast>) {
        self.print_var(var, true);
        self.word(";");
    }

    fn print_var(&mut self, var: &'ast ast::VariableDefinition<'ast>, is_var_def: bool) {
        let ast::VariableDefinition {
            span,
            ty,
            visibility,
            mutability,
            data_location,
            override_,
            indexed,
            name,
            initializer,
        } = var;

        if self.handle_span(*span, false) {
            return;
        }

        // NOTE(rusowsky): this is hacky but necessary to properly estimate if we figure out if we
        // have double breaks (which should have double indentation) or not.
        // Alternatively, we could achieve the same behavior with a new box group that supports
        // "continuation" which would only increase indentation if its parent box broke.
        let init_space_left = self.space_left();
        let mut pre_init_size = self.estimate_size(ty.span);

        // Non-elementary types use commasep which has its own padding.
        self.s.ibox(0);
        if override_.is_some() {
            self.s.cbox(self.ind);
        } else {
            self.s.ibox(self.ind);
        }
        self.print_ty(ty);

        self.print_attribute(visibility.map(|v| v.to_str()), is_var_def, &mut pre_init_size);
        self.print_attribute(mutability.map(|m| m.to_str()), is_var_def, &mut pre_init_size);
        self.print_attribute(data_location.map(|d| d.to_str()), is_var_def, &mut pre_init_size);

        if let Some(override_) = override_ {
            if self
                .print_comments(override_.span.lo(), CommentConfig::skip_ws().mixed_prev_space())
                .is_none()
            {
                self.print_sep(Separator::SpaceOrNbsp(is_var_def));
            }
            self.ibox(0);
            self.print_override(override_);
            pre_init_size += self.estimate_size(override_.span) + 1;
        }

        if *indexed {
            self.print_attribute(indexed.then_some("indexed"), is_var_def, &mut pre_init_size);
        }

        if let Some(ident) = name {
            self.print_sep(Separator::SpaceOrNbsp(is_var_def && override_.is_none()));
            self.print_comments(
                ident.span.lo(),
                CommentConfig::skip_ws().mixed_no_break().mixed_post_nbsp(),
            );
            self.print_ident(ident);
            pre_init_size += self.estimate_size(ident.span) + 1;
        }
        if let Some(init) = initializer {
            let cache = self.var_init;
            self.var_init = true;

            pre_init_size += 2;
            self.print_word(" =");
            if override_.is_some() {
                self.end();
            }
            self.end();
            if pre_init_size <= init_space_left {
                self.neverbreak();
                pre_init_size += 1;
                self.call_stack.add_precall(pre_init_size);
            } else {
                self.call_stack.add_precall(init_space_left + self.config.tab_width);
            }

            if let Some(cmnt) = self.peek_comment_before(init.span.lo())
                && self.inline_config.is_disabled(cmnt.span)
            {
                self.print_sep(Separator::Nbsp);
            }
            if self
                .print_comments(
                    init.span.lo(),
                    CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                )
                .is_some_and(|cmnt| cmnt.is_trailing())
            {
                self.break_offset_if_not_bol(SIZE_INFINITY as usize, self.ind, false);
            }

            if let ast::ExprKind::Lit(lit, ..) = &init.kind
                && lit.is_str_concatenation()
            {
                self.print_sep(Separator::Nbsp);
                self.neverbreak();
                self.s.ibox(self.ind);
                self.print_expr(init);
                self.end();
            } else if is_binary_expr(&init.kind) {
                if !self.is_bol_or_only_ind() {
                    Separator::Space.print(&mut self.s, &mut self.cursor);
                }
                if matches!(ty.kind, ast::TypeKind::Elementary(..) | ast::TypeKind::Mapping(..)) {
                    self.s.offset(self.ind);
                }
                self.print_expr(init);
            } else {
                if pre_init_size + 1 >= init_space_left && !is_call_chain(&init.kind, false) {
                    self.s.ibox(self.ind);
                } else {
                    self.s.ibox(0);
                };

                if has_complex_successor(&init.kind, true)
                    && !matches!(&init.kind, ast::ExprKind::Member(..))
                {
                    // delegate breakpoints to `self.commasep(..)`
                    if !self.is_bol_or_only_ind() {
                        let init_size = self.estimate_size(init.span);
                        if init_size + pre_init_size + 1 >= init_space_left
                            && init_size + self.config.tab_width < init_space_left
                            && !self.has_comment_between(init.span.lo(), init.span.hi())
                        {
                            self.print_sep(Separator::Space);
                            self.s.offset(self.ind);
                        } else {
                            self.print_sep(Separator::Nbsp);
                        }
                    }
                } else {
                    if !self.is_bol_or_only_ind() {
                        Separator::Space.print(&mut self.s, &mut self.cursor);
                    }
                    if matches!(ty.kind, ast::TypeKind::Elementary(..) | ast::TypeKind::Mapping(..))
                    {
                        self.s.offset(self.ind);
                    }
                }
                self.print_expr(init);
                self.end();
            }
            self.var_init = cache;
            self.call_stack.reset_precall();
        } else {
            self.end();
        }
        self.end();
    }

    fn print_attribute(
        &mut self,
        attribute: Option<&'static str>,
        is_var_def: bool,
        size: &mut usize,
    ) {
        if let Some(s) = attribute {
            self.print_sep(Separator::SpaceOrNbsp(is_var_def));
            self.print_word(s);
            *size += s.len() + 1;
        }
    }

    fn print_parameter_list(
        &mut self,
        parameters: &'ast [ast::VariableDefinition<'ast>],
        span: Span,
        format: ListFormat,
    ) {
        if self.handle_span(span, false) {
            return;
        }

        self.print_tuple(
            parameters,
            span.lo(),
            span.hi(),
            |fmt, var| fmt.print_var(var, false),
            get_span!(),
            format,
        );
    }

    fn print_ident_or_strlit(&mut self, value: &'ast ast::IdentOrStrLit) {
        match value {
            ast::IdentOrStrLit::Ident(ident) => self.print_ident(ident),
            ast::IdentOrStrLit::StrLit(strlit) => self.print_ast_str_lit(strlit),
        }
    }

    /// Prints a raw AST string literal, which is unescaped.
    fn print_ast_str_lit(&mut self, strlit: &'ast ast::StrLit) {
        self.print_str_lit(ast::StrKind::Str, strlit.span.lo(), strlit.value.as_str());
    }

    fn print_ty(&mut self, ty: &'ast ast::Type<'ast>) {
        if self.handle_span(ty.span, false) {
            return;
        }

        match &ty.kind {
            &ast::TypeKind::Elementary(ty) => 'b: {
                match ty {
                    // `address payable` is normalized to `address`.
                    ast::ElementaryType::Address(true) => {
                        self.word("address payable");
                        break 'b;
                    }
                    // Integers are normalized to long form.
                    ast::ElementaryType::Int(size) | ast::ElementaryType::UInt(size) => {
                        match (self.config.int_types, size.bits_raw()) {
                            (config::IntTypes::Short, 0 | 256)
                            | (config::IntTypes::Preserve, 0) => {
                                let short = match ty {
                                    ast::ElementaryType::Int(_) => "int",
                                    ast::ElementaryType::UInt(_) => "uint",
                                    _ => unreachable!(),
                                };
                                self.word(short);
                                break 'b;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                self.word(ty.to_abi_str());
            }
            ast::TypeKind::Array(ast::TypeArray { element, size }) => {
                self.print_ty(element);
                if let Some(size) = size {
                    self.word("[");
                    self.print_expr(size);
                    self.word("]");
                } else {
                    self.word("[]");
                }
            }
            ast::TypeKind::Function(ast::TypeFunction {
                parameters,
                visibility,
                state_mutability,
                returns,
            }) => {
                self.cbox(0);
                self.word("function");
                self.print_parameter_list(parameters, parameters.span, ListFormat::inline());

                if let Some(v) = visibility {
                    self.space();
                    self.word(v.to_str());
                }
                if let Some(sm) = state_mutability
                    && !matches!(**sm, ast::StateMutability::NonPayable)
                {
                    self.space();
                    self.word(sm.to_str());
                }
                if let Some(ret) = returns
                    && !ret.is_empty()
                {
                    self.nbsp();
                    self.word("returns");
                    self.nbsp();
                    self.print_parameter_list(
                        ret,
                        ret.span,
                        ListFormat::consistent(), // .with_cmnts_break(false),
                    );
                }
                self.end();
            }
            ast::TypeKind::Mapping(ast::TypeMapping { key, key_name, value, value_name }) => {
                self.word("mapping(");
                self.s.cbox(0);
                if let Some(cmnt) = self.peek_comment_before(key.span.lo()) {
                    if cmnt.style.is_mixed() {
                        self.print_comments(
                            key.span.lo(),
                            CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                        );
                        self.break_offset_if_not_bol(SIZE_INFINITY as usize, 0, false);
                    } else {
                        self.print_comments(key.span.lo(), CommentConfig::skip_ws());
                    }
                }
                // Fitting a mapping in one line takes, at least, 16 chars (one-char var name):
                // 'mapping(' + {key} + ' => ' {value} ') ' + {name} + ';'
                // To be more conservative, we use 18 to decide whether to force a break or not.
                else if 18
                    + self.estimate_size(key.span)
                    + key_name.map(|k| self.estimate_size(k.span)).unwrap_or(0)
                    + self.estimate_size(value.span)
                    + value_name.map(|v| self.estimate_size(v.span)).unwrap_or(0)
                    >= self.space_left()
                {
                    self.hardbreak();
                } else {
                    self.zerobreak();
                }
                self.s.cbox(0);
                self.print_ty(key);
                if let Some(ident) = key_name {
                    if self
                        .print_comments(
                            ident.span.lo(),
                            CommentConfig::skip_ws()
                                .mixed_no_break()
                                .mixed_prev_space()
                                .mixed_post_nbsp(),
                        )
                        .is_none()
                    {
                        self.nbsp();
                    }
                    self.print_ident(ident);
                }
                // NOTE(rusowsky): unless we add more spans to solar, using `value.span.lo()`
                // consumes "comment6" of which should be printed after the `=>`
                self.print_comments(
                    value.span.lo(),
                    CommentConfig::skip_ws()
                        .trailing_no_break()
                        .mixed_no_break()
                        .mixed_prev_space(),
                );
                self.space();
                self.s.offset(self.ind);
                self.word("=> ");
                self.s.ibox(self.ind);
                self.print_ty(value);
                if let Some(ident) = value_name {
                    self.neverbreak();
                    if self
                        .print_comments(
                            ident.span.lo(),
                            CommentConfig::skip_ws()
                                .mixed_no_break()
                                .mixed_prev_space()
                                .mixed_post_nbsp(),
                        )
                        .is_none()
                    {
                        self.nbsp();
                    }
                    self.print_ident(ident);
                    if self
                        .peek_comment_before(ty.span.hi())
                        .is_some_and(|cmnt| cmnt.style.is_mixed())
                    {
                        self.neverbreak();
                        self.print_comments(
                            value.span.lo(),
                            CommentConfig::skip_ws().mixed_no_break(),
                        );
                    }
                }
                self.end();
                self.end();
                if self
                    .print_comments(
                        ty.span.hi(),
                        CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                    )
                    .is_some_and(|cmnt| !cmnt.is_mixed())
                {
                    self.break_offset_if_not_bol(0, -self.ind, false);
                } else {
                    self.zerobreak();
                    self.s.offset(-self.ind);
                }
                self.end();
                self.word(")");
            }
            ast::TypeKind::Custom(path) => self.print_path(path, false),
        }
    }

    fn print_override(&mut self, override_: &'ast ast::Override<'ast>) {
        let ast::Override { span, paths } = override_;
        if self.handle_span(*span, false) {
            return;
        }
        self.word("override");
        if !paths.is_empty() {
            if self.config.override_spacing {
                self.nbsp();
            }
            self.print_tuple(
                paths,
                span.lo(),
                span.hi(),
                |this, path| this.print_path(path, false),
                get_span!(()),
                ListFormat::consistent(), // .with_cmnts_break(false),
            );
        }
    }

    /* --- Expressions --- */
    /// Prints an expression by matching on its variant and delegating to the appropriate
    /// printer method, handling all Solidity expression kinds.
    fn print_expr(&mut self, expr: &'ast ast::Expr<'ast>) {
        let ast::Expr { span, ref kind } = *expr;
        if self.handle_span(span, false) {
            return;
        }

        match kind {
            ast::ExprKind::Array(exprs) => {
                self.print_array(exprs, expr.span, |this, e| this.print_expr(e), get_span!())
            }
            ast::ExprKind::Assign(lhs, None, rhs) => self.print_assign_expr(lhs, rhs),
            ast::ExprKind::Assign(lhs, Some(op), rhs) => self.print_bin_expr(lhs, op, rhs, true),
            ast::ExprKind::Binary(lhs, op, rhs) => self.print_bin_expr(lhs, op, rhs, false),
            ast::ExprKind::Call(call_expr, call_args) => {
                self.print_member_or_call_chain(call_expr, None, |s| {
                    s.print_call_args(
                        call_args,
                        ListFormat::compact()
                            .break_cmnts()
                            .break_single(true)
                            .without_ind(s.return_bin_expr),
                        get_callee_head_size(call_expr),
                    );
                })
            }
            ast::ExprKind::CallOptions(expr, named_args) => {
                self.print_expr(expr);
                self.print_named_args(named_args, span.hi());
            }
            ast::ExprKind::Delete(expr) => {
                self.word("delete ");
                self.print_expr(expr);
            }
            ast::ExprKind::Ident(ident) => self.print_ident(ident),
            ast::ExprKind::Index(expr, kind) => self.print_index_expr(span, expr, kind),
            ast::ExprKind::Lit(lit, unit) => {
                self.print_lit(lit);
                if let Some(unit) = unit {
                    self.nbsp();
                    self.word(unit.to_str());
                }
            }
            ast::ExprKind::Member(member_expr, ident) => {
                self.print_member_or_call_chain(member_expr, Some(ident), |s| {
                    s.print_trailing_comment(member_expr.span.hi(), Some(ident.span.lo()));
                    if !matches!(member_expr.kind, ast::ExprKind::Ident(_) | ast::ExprKind::Type(_))
                    {
                        s.zerobreak();
                    }
                    s.word(".");
                    s.print_ident(ident);
                });
            }
            ast::ExprKind::New(ty) => {
                self.word("new ");
                self.print_ty(ty);
            }
            ast::ExprKind::Payable(args) => {
                self.word("payable");
                self.print_call_args(args, ListFormat::compact().break_cmnts(), 7);
            }
            ast::ExprKind::Ternary(cond, then, els) => self.print_ternary_expr(cond, then, els),
            ast::ExprKind::Tuple(exprs) => self.print_tuple(
                exprs,
                span.lo(),
                span.hi(),
                |this, expr| {
                    if let Some(expr) = expr {
                        this.print_expr(expr);
                    }
                },
                |e| e.as_deref().map(|e| e.span),
                ListFormat::compact().break_single(is_binary_expr(&expr.kind)),
            ),
            ast::ExprKind::TypeCall(ty) => {
                self.word("type");
                self.print_tuple(
                    std::slice::from_ref(ty),
                    span.lo(),
                    span.hi(),
                    Self::print_ty,
                    get_span!(),
                    ListFormat::consistent(),
                );
            }
            ast::ExprKind::Type(ty) => self.print_ty(ty),
            ast::ExprKind::Unary(un_op, expr) => {
                let prefix = un_op.kind.is_prefix();
                let op = un_op.kind.to_str();
                if prefix {
                    self.word(op);
                }
                self.print_expr(expr);
                if !prefix {
                    debug_assert!(un_op.kind.is_postfix());
                    self.word(op);
                }
            }
        }
        self.cursor.advance_to(span.hi(), true);
    }

    /// Prints a simple assignment expression of the form `lhs = rhs`.
    fn print_assign_expr(&mut self, lhs: &'ast ast::Expr<'ast>, rhs: &'ast ast::Expr<'ast>) {
        let prev_var_init = self.var_init;
        self.var_init = true;

        // Estimate layout constraints
        let space_left = self.space_left();
        let lhs_size = self.estimate_size(lhs.span);
        let rhs_size = self.estimate_size(rhs.span);

        let total_size = lhs_size + rhs_size + 4; // 'lhs' + ' = ' + 'rhs' + ';'
        let overflows = total_size >= space_left;
        let fits_alone = rhs_size + self.config.tab_width < space_left;

        self.call_stack.add_precall(lhs_size);

        let is_simple_rhs = matches!(rhs.kind, ast::ExprKind::Lit(..) | ast::ExprKind::Ident(..));
        let is_chain = is_call_chain(&rhs.kind, false);

        if (is_chain && overflows && fits_alone) || is_simple_rhs {
            self.s.ibox(self.ind)
        } else {
            self.s.ibox(0)
        }

        // Print LHS and '='
        self.print_expr(lhs);
        self.word(" =");

        // Handle RHS printing strategy
        match &rhs.kind {
            ast::ExprKind::Lit(lit, ..) if lit.is_str_concatenation() => {
                self.print_sep(Separator::Nbsp);
                self.neverbreak();
                self.s.ibox(self.ind);
                self.print_expr(rhs);
                self.end();
            }
            _ if (is_chain && overflows && fits_alone) || (is_simple_rhs && overflows) => {
                self.print_sep(Separator::Space);
                self.print_expr(rhs);
            }
            _ => {
                self.print_sep(Separator::Nbsp);
                self.neverbreak();
                self.print_expr(rhs);
            }
        }

        self.end();
        self.var_init = prev_var_init;
        self.call_stack.reset_precall();
    }

    /// Prints a binary operator expression. Handles operator chains and formatting.
    fn print_bin_expr(
        &mut self,
        lhs: &'ast ast::Expr<'ast>,
        bin_op: &ast::BinOp,
        rhs: &'ast ast::Expr<'ast>,
        is_assign: bool,
    ) {
        let prev_chain = self.binary_expr;
        let is_chain = prev_chain.is_some_and(|prev| prev == bin_op.kind.group());

        // Opening box if starting a new operator chain.
        if !is_chain {
            self.binary_expr = Some(bin_op.kind.group());

            let indent = if (is_assign && has_complex_successor(&rhs.kind, true))
                || self.call_stack.is_nested()
                    && is_call_chain(&lhs.kind, false)
                    && self.estimate_size(lhs.span) >= self.space_left()
            {
                0
            } else {
                self.ind
            };
            self.s.ibox(indent);
        }

        // Print LHS.
        self.print_expr(lhs);

        // Handle assignment (`+=`, etc.) vs binary ops (`+`, `*`, etc.).
        let no_trailing_comment = !self.print_trailing_comment(lhs.span.hi(), Some(rhs.span.lo()));
        if is_assign {
            if no_trailing_comment {
                self.nbsp();
            }
            self.word(bin_op.kind.to_str());
            self.word("= ");
        } else {
            if no_trailing_comment
                && self
                    .print_comments(
                        bin_op.span.lo(),
                        CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                    )
                    .is_none_or(|cmnt| cmnt.is_mixed())
            {
                if !self.config.pow_no_space || !matches!(bin_op.kind, ast::BinOpKind::Pow) {
                    self.space_if_not_bol();
                } else if !self.is_bol_or_only_ind() && !self.last_token_is_break() {
                    self.zerobreak();
                }
            }

            self.word(bin_op.kind.to_str());

            if !self.config.pow_no_space || !matches!(bin_op.kind, ast::BinOpKind::Pow) {
                self.nbsp();
            }
        }

        // Print RHS with optional ibox if mixed comment precedes.
        let rhs_has_mixed_comment =
            self.peek_comment_before(rhs.span.lo()).is_some_and(|cmnt| cmnt.style.is_mixed());
        if rhs_has_mixed_comment {
            self.ibox(0);
            self.print_expr(rhs);
            self.end();
        } else {
            self.print_expr(rhs);
        }

        // End current box if this was top-level in the chain.
        if !is_chain {
            self.binary_expr = prev_chain;
            self.end();
        }
    }

    /// Prints an indexing expression.
    fn print_index_expr(
        &mut self,
        span: Span,
        expr: &'ast ast::Expr<'ast>,
        kind: &'ast ast::IndexKind<'ast>,
    ) {
        self.print_expr(expr);
        self.word("[");
        self.s.cbox(self.ind);

        let mut skip_break = false;

        match kind {
            ast::IndexKind::Index(Some(inner_expr)) => {
                self.zerobreak();
                self.print_expr(inner_expr);
            }
            ast::IndexKind::Index(None) => {}
            ast::IndexKind::Range(start, end) => {
                if let Some(start_expr) = start {
                    if self
                        .print_comments(start_expr.span.lo(), CommentConfig::skip_ws())
                        .is_none_or(|s| s.is_mixed())
                    {
                        self.zerobreak();
                    }
                    self.print_expr(start_expr);
                } else {
                    self.zerobreak();
                }

                self.word(":");

                if let Some(end_expr) = end {
                    self.s.ibox(self.ind);
                    if start.is_some() {
                        self.zerobreak();
                    }
                    self.print_comments(
                        end_expr.span.lo(),
                        CommentConfig::skip_ws()
                            .mixed_prev_space()
                            .mixed_no_break()
                            .mixed_post_nbsp(),
                    );
                    self.print_expr(end_expr);
                }

                // Trailing comment handling.
                let mut is_trailing = false;
                if let Some(style) = self.print_comments(
                    span.hi(),
                    CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                ) {
                    skip_break = true;
                    is_trailing = style.is_trailing();
                }

                // Adjust indentation and line breaks.
                match (skip_break, end.is_some()) {
                    (true, true) => {
                        self.break_offset_if_not_bol(0, -2 * self.ind, false);
                        self.end();
                        if !is_trailing {
                            self.break_offset_if_not_bol(0, -self.ind, false);
                        }
                    }
                    (true, false) => {
                        self.break_offset_if_not_bol(0, -self.ind, false);
                    }
                    (false, true) => {
                        self.end();
                    }
                    _ => {}
                }
            }
        }

        if !skip_break {
            self.zerobreak();
            self.s.offset(-self.ind);
        }

        self.end();
        self.word("]");
    }

    /// Prints a ternary expression of the form `cond ? then : else`.
    fn print_ternary_expr(
        &mut self,
        cond: &'ast ast::Expr<'ast>,
        then: &'ast ast::Expr<'ast>,
        els: &'ast ast::Expr<'ast>,
    ) {
        self.s.cbox(self.ind);
        self.s.ibox(0);

        let mut print_ternary_expr =
            |span_lo, prefix: Option<&'static str>, expr: &'ast ast::Expr<'ast>| {
                match prefix {
                    Some(prefix) => {
                        if self.peek_comment_before(span_lo).is_some() {
                            self.space();
                        }
                        self.print_comments(span_lo, CommentConfig::skip_ws());
                        self.end();
                        if !self.is_bol_or_only_ind() {
                            self.space();
                        }
                        self.s.ibox(0);
                        self.word(prefix);
                    }
                    None => {
                        self.print_comments(expr.span.lo(), CommentConfig::skip_ws());
                    }
                };
                self.print_expr(expr);
            };

        // conditional expression
        print_ternary_expr(then.span.lo(), None, cond);
        // then expression
        print_ternary_expr(then.span.lo(), Some("? "), then);
        // else expression
        print_ternary_expr(els.span.lo(), Some(": "), els);

        self.end();
        self.neverbreak();
        self.s.offset(-self.ind);
        self.end();
    }

    // If `add_parens_if_empty` is true, then add parentheses `()` even if there are no arguments.
    fn print_modifier_call(
        &mut self,
        modifier: &'ast ast::Modifier<'ast>,
        add_parens_if_empty: bool,
    ) {
        let ast::Modifier { name, arguments } = modifier;
        self.print_path(name, false);
        if !arguments.is_empty() || add_parens_if_empty {
            self.print_call_args(
                arguments,
                ListFormat::compact().break_cmnts(),
                name.to_string().len(),
            );
        }
    }

    fn print_member_or_call_chain<F>(
        &mut self,
        child_expr: &'ast ast::Expr<'ast>,
        member_ident: Option<&ast::Ident>, // only members have `Ident`, calls don't
        print_suffix: F,
    ) where
        F: FnOnce(&mut Self),
    {
        let parent_is_chain = self.call_stack.last().copied().is_some_and(|call| call.is_chained());
        if !parent_is_chain {
            // Estimate sizes of callee and optional member
            let member_size = member_ident.map_or(0, |i| self.estimate_size(i.span));
            let callee_size = get_callee_head_size(child_expr);
            let total_size = callee_size + member_size + 2;

            // Start a new chain if needed
            if is_call_chain(&child_expr.kind, false) {
                self.call_stack.push(CallContext::chained(callee_size + member_size));
            }

            let fits_line = self.space_left() > total_size;
            let no_mixed_comment =
                self.peek_comment_before(child_expr.span.hi()).is_none_or(|c| c.style.is_mixed());

            if !is_call_chain(&child_expr.kind, true) && fits_line && no_mixed_comment {
                self.s.ibox(0);
            } else {
                self.s.ibox(self.ind);
            }
        }

        // Recursively print the child/prefix expression.
        self.print_expr(child_expr);

        // Call the closure to print the suffix for the current link, with the calculated position.
        print_suffix(self);

        // If a chain was started, clean up the state and end the box.
        if !parent_is_chain {
            if is_call_chain(&child_expr.kind, false) {
                self.call_stack.pop();
            }
            self.end();
        }
    }

    fn print_call_args(
        &mut self,
        args: &'ast ast::CallArgs<'ast>,
        format: ListFormat,
        callee_size: usize,
    ) {
        let ast::CallArgs { span, ref kind } = *args;
        if self.handle_span(span, true) {
            return;
        }

        self.call_stack.push(CallContext::nested(callee_size));
        // Clear the binary expression cache before the call.
        let cache = self.binary_expr.take();

        match kind {
            ast::CallArgsKind::Unnamed(exprs) => {
                self.print_tuple(
                    exprs,
                    span.lo(),
                    span.hi(),
                    |this, e| this.print_expr(e),
                    get_span!(),
                    format,
                );
            }
            ast::CallArgsKind::Named(named_args) => {
                self.print_inside_parens(|state| state.print_named_args(named_args, span.hi()));
            }
        }

        // Restore the cache to continue with the current chain.
        self.binary_expr = cache;
        self.call_stack.pop();
    }

    fn print_named_args(&mut self, args: &'ast [ast::NamedArg<'ast>], pos_hi: BytePos) {
        let cache = self.named_call_expr;
        if !cache {
            self.named_call_expr = true;
        };

        let list_format = match (self.config.bracket_spacing, self.config.call_compact_args) {
            (false, true) => ListFormat::compact(),
            (false, false) => ListFormat::consistent(),
            (true, true) => ListFormat::compact().with_space(),
            (true, false) => ListFormat::consistent().with_space(),
        };

        self.word("{");
        // Use the start position of the first argument's name for comment processing.
        let list_lo = args.first().map_or(pos_hi, |arg| arg.name.span.lo());

        self.commasep(
            args,
            list_lo,
            pos_hi,
            // Closure to print a single named argument (`name: value`)
            |s, arg| {
                s.cbox(0);
                s.print_ident(&arg.name);
                s.word(":");
                if s.same_source_line(arg.name.span.hi(), arg.value.span.hi())
                    || !s.print_trailing_comment(arg.name.span.hi(), None)
                {
                    s.nbsp();
                }
                s.print_comments(
                    arg.value.span.lo(),
                    CommentConfig::skip_ws().mixed_no_break().mixed_post_nbsp(),
                );
                s.print_expr(arg.value);
                s.end();
            },
            |arg| Some(arg.name.span.until(arg.value.span)),
            list_format.break_cmnts().break_single(true).without_ind(self.call_stack.is_chain()),
        );
        self.word("}");

        if !cache {
            self.named_call_expr = false;
        }
    }

    /* --- Statements --- */
    /// Prints the given statement in the source code, handling formatting, inline documentation,
    /// trailing comments and layout logic for various statement kinds.
    fn print_stmt(&mut self, stmt: &'ast ast::Stmt<'ast>) {
        let ast::Stmt { ref docs, span, ref kind } = *stmt;
        self.print_docs(docs);

        // Handle disabled statements.
        if self.handle_span(span, false) {
            self.print_trailing_comment_no_break(stmt.span.hi(), None);
            return;
        }

        // return statements can't have a preceding comment in the same line.
        let force_break = matches!(kind, ast::StmtKind::Return(..))
            && self.peek_comment_before(span.lo()).is_some_and(|cmnt| cmnt.style.is_mixed());

        match kind {
            ast::StmtKind::Assembly(ast::StmtAssembly { dialect, flags, block }) => {
                self.print_assembly_stmt(span, dialect, flags, block)
            }
            ast::StmtKind::DeclSingle(var) => self.print_var(var, true),
            ast::StmtKind::DeclMulti(vars, init_expr) => {
                self.print_multi_decl_stmt(span, vars, init_expr)
            }
            ast::StmtKind::Block(stmts) => self.print_block(stmts, span),
            ast::StmtKind::Break => self.word("break"),
            ast::StmtKind::Continue => self.word("continue"),
            ast::StmtKind::DoWhile(stmt, cond) => {
                self.word("do ");
                self.print_stmt_as_block(stmt, cond.span.lo(), false);
                self.nbsp();
                self.print_if_cond("while", cond, cond.span.hi());
            }
            ast::StmtKind::Emit(path, args) => self.print_emit_or_revert("emit", path, args),
            ast::StmtKind::Expr(expr) => self.print_expr(expr),
            ast::StmtKind::For { init, cond, next, body } => {
                self.print_for_stmt(span, init, cond, next, body)
            }
            ast::StmtKind::If(cond, then, els_opt) => self.print_if_stmt(span, cond, then, els_opt),
            ast::StmtKind::Return(expr) => self.print_return_stmt(force_break, expr),
            ast::StmtKind::Revert(path, args) => self.print_emit_or_revert("revert", path, args),
            ast::StmtKind::Try(ast::StmtTry { expr, clauses }) => {
                self.print_try_stmt(expr, clauses)
            }
            ast::StmtKind::UncheckedBlock(block) => {
                self.word("unchecked ");
                self.print_block(block, stmt.span);
            }
            ast::StmtKind::While(cond, stmt) => {
                // Check if blocks should be inlined and update cache if necessary
                let inline = self.is_single_line_block(cond, stmt, None);
                if !inline.is_cached && self.single_line_stmt.is_none() {
                    self.single_line_stmt = Some(inline.outcome);
                }

                // Print while cond and its statement
                self.print_if_cond("while", cond, stmt.span.lo());
                self.nbsp();
                self.print_stmt_as_block(stmt, stmt.span.hi(), inline.outcome);

                // Clear cache if necessary
                if !inline.is_cached && self.single_line_stmt.is_some() {
                    self.single_line_stmt = None;
                }
            }
            ast::StmtKind::Placeholder => self.word("_"),
        }
        if stmt_needs_semi(kind) {
            self.neverbreak(); // semicolon shouldn't account for linebreaks
            self.word(";");
            self.cursor.advance_to(span.hi(), true);
        }
        // print comments without breaks, as those are handled by the caller.
        self.print_comments(
            stmt.span.hi(),
            CommentConfig::default().trailing_no_break().mixed_no_break().mixed_prev_space(),
        );
        self.print_trailing_comment_no_break(stmt.span.hi(), None);
    }

    /// Prints an `assembly` statement, including optional dialect and flags,
    /// followed by its Yul block.
    fn print_assembly_stmt(
        &mut self,
        span: Span,
        dialect: &'ast Option<ast::StrLit>,
        flags: &'ast [ast::StrLit],
        block: &'ast ast::yul::Block<'ast>,
    ) {
        _ = self.handle_span(self.cursor.span(span.lo()), false);
        if !self.handle_span(span.until(block.span), false) {
            self.cursor.advance_to(span.lo(), true);
            self.print_word("assembly ");
            if let Some(dialect) = dialect {
                self.print_ast_str_lit(dialect);
                self.print_sep(Separator::Nbsp);
            }
            if !flags.is_empty() {
                self.print_tuple(
                    flags,
                    span.lo(),
                    block.span.lo(),
                    Self::print_ast_str_lit,
                    get_span!(),
                    ListFormat::consistent(),
                );
                self.print_sep(Separator::Nbsp);
            }
        }
        self.print_yul_block(block, block.span, false);
    }

    /// Prints a multiple-variable declaration with a single initializer expression,
    /// formatted as a tuple-style assignment (e.g., `(a, b) = foo();`).
    fn print_multi_decl_stmt(
        &mut self,
        span: Span,
        vars: &'ast [Option<ast::VariableDefinition<'ast>>],
        init_expr: &'ast ast::Expr<'ast>,
    ) {
        let space_left = self.space_left();

        self.s.ibox(self.ind);
        self.s.ibox(-self.ind);
        self.print_tuple(
            vars,
            span.lo(),
            init_expr.span.lo(),
            |this, var| {
                // NOTE(rusowsky): unless we add more spans to solar, it is not possible to print
                // comments between the commas of unhandled vars
                if let Some(var) = var {
                    this.print_var(var, true);
                }
            },
            |v| v.as_ref().map(|v| v.span),
            ListFormat::consistent(),
        );
        self.end();
        self.word(" =");

        // '(' + var + ', ' + var + ')' + ' ='
        let vars_size = vars
            .iter()
            .fold(0, |acc, v| acc + v.as_ref().map_or(0, |v| self.estimate_size(v.span)) + 2)
            + 2;
        self.call_stack.add_precall(vars_size);

        if self.estimate_size(init_expr.span) + self.config.tab_width
            <= std::cmp::max(space_left, self.space_left())
        {
            self.print_sep(Separator::Space);
            self.ibox(0);
        } else {
            self.print_sep(Separator::Nbsp);
            self.neverbreak();
            self.s.ibox(-self.ind);
        }
        self.print_expr(init_expr);
        self.end();
        self.end();
    }

    /// Prints a `for` loop statement, including its initializer, condition,
    /// increment expression, and loop body, with formatting and spacing.
    fn print_for_stmt(
        &mut self,
        span: Span,
        init: &'ast Option<&mut ast::Stmt<'ast>>,
        cond: &'ast Option<&mut ast::Expr<'ast>>,
        next: &'ast Option<&mut ast::Expr<'ast>>,
        body: &'ast ast::Stmt<'ast>,
    ) {
        self.cbox(0);
        self.s.ibox(self.ind);
        self.print_word("for (");
        self.zerobreak();

        // Print init.
        self.s.cbox(0);
        match init {
            Some(init_stmt) => self.print_stmt(init_stmt),
            None => self.print_word(";"),
        }

        // Print condition.
        match cond {
            Some(cond_expr) => {
                self.print_sep(Separator::Space);
                self.print_expr(cond_expr);
            }
            None => self.zerobreak(),
        }
        self.print_word(";");

        // Print next clause.
        match next {
            Some(next_expr) => {
                self.space();
                self.print_expr(next_expr);
            }
            None => self.zerobreak(),
        }

        // Close head.
        self.break_offset_if_not_bol(0, -self.ind, false);
        self.end();
        self.print_word(") ");
        self.neverbreak();
        self.end();

        // Print comments and body.
        self.print_comments(body.span.lo(), CommentConfig::skip_ws());
        self.print_stmt_as_block(body, span.hi(), false);
        self.end();
    }

    /// Prints an `if` statement, including its condition, `then` block, and any chained
    /// `else` or `else if` branches, handling inline formatting decisions and comments.
    fn print_if_stmt(
        &mut self,
        span: Span,
        cond: &'ast ast::Expr<'ast>,
        then: &'ast ast::Stmt<'ast>,
        els_opt: &'ast Option<&mut ast::Stmt<'ast>>,
    ) {
        // Check if blocks should be inlined and update cache if necessary
        let inline = self.is_single_line_block(cond, then, els_opt.as_ref());
        let set_inline_cache = !inline.is_cached && self.single_line_stmt.is_none();
        if set_inline_cache {
            self.single_line_stmt = Some(inline.outcome);
        }

        self.cbox(0);
        self.ibox(0);
        // Print if stmt
        self.print_if_no_else(cond, then, inline.outcome);

        // Print else (if) stmts, if any
        let mut current_else = els_opt.as_deref();
        while let Some(els) = current_else {
            if self.ends_with('}') {
                match self.print_comments(els.span.lo(), CommentConfig::skip_ws().mixed_no_break())
                {
                    Some(cmnt) => {
                        if cmnt.is_mixed() {
                            self.hardbreak();
                        }
                    }
                    None => self.nbsp(),
                }
            } else {
                self.hardbreak_if_not_bol();
                if self
                    .print_comments(els.span.lo(), CommentConfig::skip_ws())
                    .is_some_and(|cmnt| cmnt.is_mixed())
                {
                    self.hardbreak();
                };
            }

            self.ibox(0);
            self.print_word("else ");
            match &els.kind {
                ast::StmtKind::If(cond, then, next_else) => {
                    self.print_if_no_else(cond, then, inline.outcome);
                    current_else = next_else.as_deref();
                }
                _ => {
                    self.print_stmt_as_block(els, span.hi(), inline.outcome);
                    self.end(); // end ibox for final else
                    break;
                }
            }
        }
        self.end();

        // Clear inline cache if we set it earlier.
        if set_inline_cache {
            self.single_line_stmt = None;
        }
    }

    /// Prints a `return` statement, optionally including a return expression.
    /// Handles spacing, line breaking, and formatting.
    fn print_return_stmt(&mut self, force_break: bool, expr: &'ast Option<&mut ast::Expr<'ast>>) {
        if force_break {
            self.hardbreak_if_not_bol();
        }

        let space_left = self.space_left();
        let expr_size = expr.as_ref().map_or(0, |expr| self.estimate_size(expr.span));

        // `return ' + expr + ';'
        let overflows = space_left < 8 + expr_size;
        let fits_alone = space_left > expr_size;

        if let Some(expr) = expr {
            let is_simple = matches!(expr.kind, ast::ExprKind::Lit(..) | ast::ExprKind::Ident(..));
            let allow_break = overflows && fits_alone;

            self.return_bin_expr = matches!(expr.kind, ast::ExprKind::Binary(..));
            self.s.ibox(if is_simple || allow_break { self.ind } else { 0 });

            self.print_word("return");

            match self.print_comments(
                expr.span.lo(),
                CommentConfig::skip_ws().mixed_no_break().mixed_prev_space().mixed_post_nbsp(),
            ) {
                Some(cmnt) if cmnt.is_trailing() && !is_simple => self.s.offset(self.ind),
                None => self.print_sep(Separator::SpaceOrNbsp(allow_break)),
                _ => {}
            }

            self.print_expr(expr);
            self.end();
            self.return_bin_expr = false;
        } else {
            self.print_word("return");
        }
    }

    /// Prints a `try` statement along with its associated `catch` clauses,
    /// following Solidity's `try ... returns (...) { ... } catch (...) { ... }` syntax.
    fn print_try_stmt(
        &mut self,
        expr: &'ast ast::Expr<'ast>,
        clauses: &'ast [ast::TryCatchClause<'ast>],
    ) {
        self.cbox(0);
        if let Some((first, other)) = clauses.split_first() {
            // Print the 'try' clause
            let ast::TryCatchClause { args, block, span: try_span, .. } = first;
            self.ibox(0);
            self.print_word("try ");
            self.print_comments(expr.span.lo(), CommentConfig::skip_ws());
            self.print_expr(expr);

            // Print comments.
            self.print_comments(
                args.first().map(|p| p.span.lo()).unwrap_or_else(|| expr.span.lo()),
                CommentConfig::skip_ws(),
            );
            if !self.is_beginning_of_line() {
                self.nbsp();
            }

            if !args.is_empty() {
                self.print_word("returns ");
                self.print_parameter_list(
                    args,
                    args.span.with_hi(block.span.lo()),
                    ListFormat::compact(),
                );
                self.nbsp();
            }
            if block.is_empty() {
                self.print_block(block, *try_span);
                self.end();
            } else {
                self.print_word("{");
                self.end();
                self.neverbreak();
                self.print_trailing_comment_no_break(try_span.lo(), None);
                self.print_block_without_braces(block, try_span.hi(), Some(self.ind));
                if self.cursor.enabled || self.cursor.pos < try_span.hi() {
                    self.print_word("}");
                    self.cursor.advance_to(try_span.hi(), true);
                }
            }

            let mut skip_ind = false;
            if self.print_trailing_comment(try_span.hi(), other.first().map(|c| c.span.lo())) {
                // if a trailing comment is printed at the very end, we have to manually
                // adjust the offset to avoid having a double break.
                self.break_offset_if_not_bol(0, self.ind, false);
                skip_ind = true;
            };

            let mut prev_block_multiline = self.is_multiline_block(block, false);

            // Handle 'catch' clauses
            for (pos, ast::TryCatchClause { name, args, block, span: catch_span }) in
                other.iter().delimited()
            {
                let current_block_multiline = self.is_multiline_block(block, false);
                if !pos.is_first || !skip_ind {
                    if prev_block_multiline && (current_block_multiline || pos.is_last) {
                        self.nbsp();
                    } else {
                        self.space();
                        if !current_block_multiline {
                            self.s.offset(self.ind);
                        }
                    }
                }
                self.s.ibox(self.ind);
                self.print_comments(
                    catch_span.lo(),
                    CommentConfig::skip_ws().mixed_no_break().mixed_post_nbsp(),
                );

                self.print_word("catch ");
                if !args.is_empty() {
                    self.print_comments(
                        args[0].span.lo(),
                        CommentConfig::skip_ws().mixed_no_break().mixed_post_nbsp(),
                    );
                    if let Some(name) = name {
                        self.print_ident(name);
                    }
                    self.print_parameter_list(
                        args,
                        args.span.with_hi(block.span.lo()),
                        ListFormat::inline(),
                    );
                    self.nbsp();
                }
                self.print_word("{");
                self.end();
                self.print_trailing_comment_no_break(catch_span.lo(), None);
                self.print_block_without_braces(block, catch_span.hi(), Some(self.ind));
                if self.cursor.enabled || self.cursor.pos < try_span.hi() {
                    self.print_word("}");
                    self.cursor.advance_to(catch_span.hi(), true);
                }

                prev_block_multiline = current_block_multiline;
            }
        }
        self.end();
    }

    fn print_if_no_else(
        &mut self,
        cond: &'ast ast::Expr<'ast>,
        then: &'ast ast::Stmt<'ast>,
        inline: bool,
    ) {
        if !self.handle_span(cond.span.until(then.span), true) {
            self.print_if_cond("if", cond, then.span.lo());
            // if empty block without comments, ensure braces are inlined
            if let ast::StmtKind::Block(block) = &then.kind
                && block.is_empty()
                && self.peek_comment_before(then.span.hi()).is_none()
            {
                self.neverbreak();
                self.print_sep(Separator::Nbsp);
            } else {
                self.print_sep(Separator::Space);
            }
        }
        self.end();
        self.print_stmt_as_block(then, then.span.hi(), inline);
        self.cursor.advance_to(then.span.hi(), true);
    }

    fn print_if_cond(&mut self, kw: &'static str, cond: &'ast ast::Expr<'ast>, pos_hi: BytePos) {
        self.print_word(kw);
        Separator::Nbsp.print(&mut self.s, &mut self.cursor);
        self.print_tuple(
            std::slice::from_ref(cond),
            cond.span.lo(),
            pos_hi,
            Self::print_expr,
            get_span!(),
            ListFormat::compact().break_cmnts().break_single(is_binary_expr(&cond.kind)),
        );
    }

    fn print_emit_or_revert(
        &mut self,
        kw: &'static str,
        path: &'ast ast::PathSlice,
        args: &'ast ast::CallArgs<'ast>,
    ) {
        self.word(kw);
        if self
            .print_comments(
                path.span().lo(),
                CommentConfig::skip_ws().mixed_no_break().mixed_prev_space().mixed_post_nbsp(),
            )
            .is_none()
        {
            self.nbsp();
        };
        self.s.cbox(0);
        self.print_path(path, false);
        self.print_call_args(
            args,
            if args.len() == 1 {
                ListFormat::compact().break_cmnts()
            } else {
                ListFormat::compact().break_cmnts().no_delimiters()
            },
            path.to_string().len(),
        );
        self.end();
    }

    fn print_block(&mut self, block: &'ast [ast::Stmt<'ast>], span: Span) {
        self.print_block_inner(
            block,
            BlockFormat::Regular,
            Self::print_stmt,
            |b| b.span,
            span.hi(),
        );
    }

    fn print_block_without_braces(
        &mut self,
        block: &'ast [ast::Stmt<'ast>],
        pos_hi: BytePos,
        offset: Option<isize>,
    ) {
        self.print_block_inner(
            block,
            BlockFormat::NoBraces(offset),
            Self::print_stmt,
            |b| b.span,
            pos_hi,
        );
    }

    // Body of a if/loop.
    fn print_stmt_as_block(&mut self, stmt: &'ast ast::Stmt<'ast>, pos_hi: BytePos, inline: bool) {
        if self.handle_span(stmt.span, false) {
            return;
        }

        let stmts = if let ast::StmtKind::Block(stmts) = &stmt.kind {
            stmts
        } else {
            std::slice::from_ref(stmt)
        };

        if inline && !stmts.is_empty() {
            self.neverbreak();
            self.print_block_without_braces(stmts, pos_hi, None);
        } else {
            // Reset cache for nested (child) stmts within this (parent) block.
            let inline_parent = self.single_line_stmt.take();

            self.print_word("{");
            self.print_block_without_braces(stmts, pos_hi, Some(self.ind));
            self.print_word("}");

            // Restore cache for the rest of stmts within the same height.
            self.single_line_stmt = inline_parent;
        }
    }

    /// Determines if an `if/else` block should be inlined.
    /// Also returns if the value was cached, so that it can be cleaned afterwards.
    ///
    /// # Returns
    ///
    /// A tuple `(should_inline, was_cached)`. The second boolean is `true` if the
    /// decision was retrieved from the cache or is a final decision based on config,
    /// preventing the caller from clearing a cache value that was never set.
    fn is_single_line_block(
        &mut self,
        cond: &'ast ast::Expr<'ast>,
        then: &'ast ast::Stmt<'ast>,
        els_opt: Option<&'ast &'ast mut ast::Stmt<'ast>>,
    ) -> Decision {
        // If a decision is already cached from a parent, use it directly.
        if let Some(cached_decision) = self.single_line_stmt {
            return Decision { outcome: cached_decision, is_cached: true };
        }

        // Empty statements are always printed as blocks.
        if std::slice::from_ref(then).is_empty() {
            return Decision { outcome: false, is_cached: false };
        }

        // If possible, take an early decision based on the block style configuration.
        match self.config.single_line_statement_blocks {
            config::SingleLineBlockStyle::Preserve => {
                if self.is_stmt_in_new_line(cond, then) || self.is_multiline_block_stmt(then, true)
                {
                    return Decision { outcome: false, is_cached: false };
                }
            }
            config::SingleLineBlockStyle::Single => {
                if self.is_multiline_block_stmt(then, true) {
                    return Decision { outcome: false, is_cached: false };
                }
            }
            config::SingleLineBlockStyle::Multi => {
                return Decision { outcome: false, is_cached: false };
            }
        };

        // If no decision was made, estimate the length to be formatted.
        // NOTE: conservative check -> worst-case scenario is formatting as multi-line block.
        if !self.can_stmts_be_inlined(cond, then, els_opt) {
            return Decision { outcome: false, is_cached: false };
        }

        // If the parent would fit, check all of its children.
        if let Some(stmt) = els_opt {
            if let ast::StmtKind::If(child_cond, child_then, child_els_opt) = &stmt.kind {
                return self.is_single_line_block(child_cond, child_then, child_els_opt.as_ref());
            } else if self.is_multiline_block_stmt(stmt, true) {
                return Decision { outcome: false, is_cached: false };
            }
        }

        // If all children can also fit, allow single-line block.
        Decision { outcome: true, is_cached: false }
    }

    fn is_inline_stmt(&self, stmt: &'ast ast::Stmt<'ast>, cond_len: usize) -> bool {
        if let ast::StmtKind::If(cond, then, els_opt) = &stmt.kind {
            let if_span = cond.span.to(then.span);
            if self.sm.is_multiline(if_span)
                && matches!(
                    self.config.single_line_statement_blocks,
                    config::SingleLineBlockStyle::Preserve
                )
            {
                return false;
            }
            if cond_len + self.estimate_size(if_span) >= self.space_left() {
                return false;
            }
            if let Some(els) = els_opt
                && !self.is_inline_stmt(els, 6)
            {
                return false;
            }
        } else {
            if matches!(
                self.config.single_line_statement_blocks,
                config::SingleLineBlockStyle::Preserve
            ) && self.sm.is_multiline(stmt.span)
            {
                return false;
            }
            if cond_len + self.estimate_size(stmt.span) >= self.space_left() {
                return false;
            }
        }
        true
    }

    /// Checks if a statement was explicitly written in a new line.
    fn is_stmt_in_new_line(
        &self,
        cond: &'ast ast::Expr<'ast>,
        then: &'ast ast::Stmt<'ast>,
    ) -> bool {
        let span_between = cond.span.between(then.span);
        if let Ok(snip) = self.sm.span_to_snippet(span_between) {
            // Check for newlines after the closing parenthesis of the `if (...)`.
            if let Some((_, after_paren)) = snip.split_once(')') {
                return after_paren.lines().count() > 1;
            }
        }
        false
    }

    /// Checks if a block statement `{ ... }` contains more than one line of actual code.
    fn is_multiline_block_stmt(
        &self,
        stmt: &'ast ast::Stmt<'ast>,
        empty_as_multiline: bool,
    ) -> bool {
        if let ast::StmtKind::Block(block) = &stmt.kind {
            return self.is_multiline_block(block, empty_as_multiline);
        }
        false
    }

    /// Checks if a block statement `{ ... }` contains more than one line of actual code.
    fn is_multiline_block(&self, block: &'ast ast::Block<'ast>, empty_as_multiline: bool) -> bool {
        if block.stmts.is_empty() {
            return empty_as_multiline;
        }
        if self.sm.is_multiline(block.span)
            && let Ok(snip) = self.sm.span_to_snippet(block.span)
        {
            let code_lines = snip.lines().filter(|line| {
                let trimmed = line.trim();
                // Ignore empty lines and lines with only '{' or '}'
                if empty_as_multiline {
                    !trimmed.is_empty() && trimmed != "{" && trimmed != "}"
                } else {
                    !trimmed.is_empty()
                }
            });
            return code_lines.count() > 1;
        }
        false
    }

    /// Performs a size estimation to see if the if/else can fit on one line.
    fn can_stmts_be_inlined(
        &mut self,
        cond: &'ast ast::Expr<'ast>,
        then: &'ast ast::Stmt<'ast>,
        els_opt: Option<&'ast &'ast mut ast::Stmt<'ast>>,
    ) -> bool {
        let cond_len = self.estimate_size(cond.span);

        // If the condition fits in one line, 6 chars: 'if (' + {cond} + ') ' + {then}
        // Otherwise chars: ') ' + {then}
        let then_margin = if 6 + cond_len < self.space_left() { 6 + cond_len } else { 2 };

        if !self.is_inline_stmt(then, then_margin) {
            return false;
        }

        // Always 6 chars for the else: 'else '
        els_opt.is_none_or(|els| self.is_inline_stmt(els, 6))
    }

    fn can_header_be_inlined(&mut self, header: &ast::FunctionHeader<'_>) -> bool {
        // ' ' + visibility
        let visibility = header.visibility.map_or(0, |v| self.estimate_size(v.span) + 1);
        // ' ' + state mutability
        let mutability = header.state_mutability.map_or(0, |sm| self.estimate_size(sm.span) + 1);
        // ' ' + modifier + (' ' + modifier)
        let modifiers =
            header.modifiers.iter().fold(0, |len, m| len + self.estimate_size(m.span())) + 1;
        // ' ' + override
        let override_ = header.override_.as_ref().map_or(0, |o| self.estimate_size(o.span) + 1);
        // ' returns(' + var + (', ' + var) + ')'
        let returns = header.returns.as_ref().map_or(0, |ret| {
            ret.vars
                .iter()
                .fold(0, |len, p| if len != 0 { len + 2 } else { 8 } + self.estimate_size(p.span))
        });

        self.estimate_header_params_size(header)
            + visibility
            + mutability
            + modifiers
            + override_
            + returns
            <= self.space_left()
    }

    fn estimate_header_params_size(&mut self, header: &ast::FunctionHeader<'_>) -> usize {
        // '(' + param + (', ' + param) + ')'
        let params = header
            .parameters
            .vars
            .iter()
            .fold(0, |len, p| if len != 0 { len + 2 } else { 2 } + self.estimate_size(p.span));

        // 'function ' + name + ' ' + params
        9 + header.name.map_or(0, |name| self.estimate_size(name.span) + 1) + params
    }

    fn can_header_params_be_inlined(&mut self, header: &ast::FunctionHeader<'_>) -> bool {
        self.estimate_header_params_size(header) <= self.space_left()
    }
}

// -- HELPERS (language-specific) ----------------------------------------------

#[derive(Debug, Clone)]
enum AttributeKind<'ast> {
    Visibility(ast::Visibility),
    StateMutability(ast::StateMutability),
    Virtual,
    Override(&'ast ast::Override<'ast>),
    Modifier(&'ast ast::Modifier<'ast>),
}

type AttributeCommentMap = HashMap<BytePos, (Vec<Comment>, Vec<Comment>, Vec<Comment>)>;

impl<'ast> AttributeKind<'ast> {
    fn is_visibility(&self) -> bool {
        matches!(self, Self::Visibility(_))
    }

    fn is_state_mutability(&self) -> bool {
        matches!(self, Self::StateMutability(_))
    }

    fn is_non_payable(&self) -> bool {
        matches!(self, Self::StateMutability(ast::StateMutability::NonPayable))
    }

    fn is_virtual(&self) -> bool {
        matches!(self, Self::Virtual)
    }

    fn is_override(&self) -> bool {
        matches!(self, Self::Override(_))
    }

    fn is_modifier(&self) -> bool {
        matches!(self, Self::Modifier(_))
    }
}

#[derive(Debug, Clone)]
struct AttributeInfo<'ast> {
    kind: AttributeKind<'ast>,
    span: Span,
}

/// Helper struct to map attributes to their associated comments in function headers.
struct AttributeCommentMapper<'ast> {
    limit_pos: BytePos,
    comments: Vec<Comment>,
    attributes: Vec<AttributeInfo<'ast>>,
    empty_returns: bool,
}

impl<'ast> AttributeCommentMapper<'ast> {
    fn new(returns: Option<&'ast ast::ParameterList<'ast>>, body_pos: BytePos) -> Self {
        Self {
            comments: Vec::new(),
            attributes: Vec::new(),
            empty_returns: returns.is_none(),
            limit_pos: returns.as_ref().map_or(body_pos, |ret| ret.span.lo()),
        }
    }

    #[allow(clippy::type_complexity)]
    fn build(
        mut self,
        state: &mut State<'_, 'ast>,
        header: &'ast ast::FunctionHeader<'ast>,
    ) -> (AttributeCommentMap, Vec<AttributeInfo<'ast>>, BytePos) {
        let first_attr = self.collect_attributes(header);
        self.cache_comments(state);
        (self.map(), self.attributes, first_attr)
    }

    fn map(&mut self) -> AttributeCommentMap {
        let mut map = HashMap::new();
        for a in 0..self.attributes.len() {
            let is_last = a == self.attributes.len() - 1;
            let (mut before, mut inner, mut after) = (Vec::new(), Vec::new(), Vec::new());

            let before_limit = self.attributes[a].span.lo();
            let inner_limit = self.attributes[a].span.hi();
            let after_limit =
                if !is_last { self.attributes[a + 1].span.lo() } else { self.limit_pos };

            let mut c = 0;
            while c < self.comments.len() {
                if self.comments[c].pos() <= before_limit {
                    before.push(self.comments.remove(c));
                } else if self.comments[c].pos() <= inner_limit {
                    inner.push(self.comments.remove(c));
                } else if (after.is_empty() || is_last) && self.comments[c].pos() <= after_limit {
                    after.push(self.comments.remove(c));
                } else {
                    c += 1;
                }
            }
            map.insert(before_limit, (before, inner, after));
        }
        map
    }

    fn collect_attributes(&mut self, header: &'ast ast::FunctionHeader<'ast>) -> BytePos {
        let mut first_pos = BytePos(u32::MAX);
        if let Some(v) = header.visibility {
            if v.span.lo() < first_pos {
                first_pos = v.span.lo()
            }
            self.attributes
                .push(AttributeInfo { kind: AttributeKind::Visibility(*v), span: v.span });
        }
        if let Some(sm) = header.state_mutability {
            if sm.span.lo() < first_pos {
                first_pos = sm.span.lo()
            }
            self.attributes
                .push(AttributeInfo { kind: AttributeKind::StateMutability(*sm), span: sm.span });
        }
        if let Some(span) = header.virtual_ {
            if span.lo() < first_pos {
                first_pos = span.lo()
            }
            self.attributes.push(AttributeInfo { kind: AttributeKind::Virtual, span });
        }
        if let Some(ref o) = header.override_ {
            if o.span.lo() < first_pos {
                first_pos = o.span.lo()
            }
            self.attributes.push(AttributeInfo { kind: AttributeKind::Override(o), span: o.span });
        }
        for m in header.modifiers.iter() {
            if m.span().lo() < first_pos {
                first_pos = m.span().lo()
            }
            self.attributes
                .push(AttributeInfo { kind: AttributeKind::Modifier(m), span: m.span() });
        }
        self.attributes.sort_by_key(|attr| attr.span.lo());
        first_pos
    }

    fn cache_comments(&mut self, state: &mut State<'_, 'ast>) {
        let mut pending = None;
        for cmnt in state.comments.iter() {
            if cmnt.pos() >= self.limit_pos {
                break;
            }
            match pending {
                Some(ref p) => pending = Some(p + 1),
                None => pending = Some(0),
            }
        }
        while let Some(p) = pending {
            if p == 0 {
                pending = None;
            } else {
                pending = Some(p - 1);
            }
            let cmnt = state.next_comment().unwrap();
            if cmnt.style.is_blank() {
                continue;
            }
            self.comments.push(cmnt);
        }
    }
}

fn stmt_needs_semi(stmt: &ast::StmtKind<'_>) -> bool {
    match stmt {
        ast::StmtKind::Assembly { .. }
        | ast::StmtKind::Block { .. }
        | ast::StmtKind::For { .. }
        | ast::StmtKind::If { .. }
        | ast::StmtKind::Try { .. }
        | ast::StmtKind::UncheckedBlock { .. }
        | ast::StmtKind::While { .. } => false,

        ast::StmtKind::DeclSingle { .. }
        | ast::StmtKind::DeclMulti { .. }
        | ast::StmtKind::Break { .. }
        | ast::StmtKind::Continue { .. }
        | ast::StmtKind::DoWhile { .. }
        | ast::StmtKind::Emit { .. }
        | ast::StmtKind::Expr { .. }
        | ast::StmtKind::Return { .. }
        | ast::StmtKind::Revert { .. }
        | ast::StmtKind::Placeholder { .. } => true,
    }
}

/// Returns `true` if the item needs an isolated line break.
fn item_needs_iso(item: &ast::ItemKind<'_>) -> bool {
    match item {
        ast::ItemKind::Pragma(..)
        | ast::ItemKind::Import(..)
        | ast::ItemKind::Using(..)
        | ast::ItemKind::Variable(..)
        | ast::ItemKind::Udvt(..)
        | ast::ItemKind::Enum(..)
        | ast::ItemKind::Error(..)
        | ast::ItemKind::Event(..) => false,

        ast::ItemKind::Contract(..) => true,

        ast::ItemKind::Struct(strukt) => !strukt.fields.is_empty(),
        ast::ItemKind::Function(func) => {
            func.body.as_ref().is_some_and(|b| !b.is_empty())
                && !matches!(func.kind, ast::FunctionKind::Modifier)
        }
    }
}

fn is_binary_expr(expr_kind: &ast::ExprKind<'_>) -> bool {
    matches!(expr_kind, ast::ExprKind::Binary(..))
}

fn is_comp_op(op_kind: ast::BinOpKind) -> bool {
    match op_kind {
        ast::BinOpKind::BitOr
        | ast::BinOpKind::BitXor
        | ast::BinOpKind::BitAnd
        | ast::BinOpKind::Shl
        | ast::BinOpKind::Shr
        | ast::BinOpKind::Sar
        | ast::BinOpKind::Add
        | ast::BinOpKind::Sub
        | ast::BinOpKind::Mul
        | ast::BinOpKind::Div
        | ast::BinOpKind::Rem
        | ast::BinOpKind::Pow => false,

        ast::BinOpKind::Lt
        | ast::BinOpKind::Le
        | ast::BinOpKind::Gt
        | ast::BinOpKind::Ge
        | ast::BinOpKind::Eq
        | ast::BinOpKind::Ne
        | ast::BinOpKind::Or
        | ast::BinOpKind::And => true,
    }
}

fn has_complex_successor(expr_kind: &ast::ExprKind<'_>, left: bool) -> bool {
    match expr_kind {
        ast::ExprKind::Binary(lhs, _, rhs) => {
            if left {
                has_complex_successor(&lhs.kind, left)
            } else {
                has_complex_successor(&rhs.kind, left)
            }
        }
        ast::ExprKind::Unary(_, expr) => has_complex_successor(&expr.kind, left),
        ast::ExprKind::Lit(..) | ast::ExprKind::Ident(_) => false,
        _ => true,
    }
}

/// Traverses a nested Member/Call expression chain to find the lowest-level expression.
fn get_chain_bottom<'a>(mut expr: &'a ast::Expr<'a>) -> &'a ast::Expr<'a> {
    loop {
        match &expr.kind {
            ast::ExprKind::Member(child, ..) | ast::ExprKind::Call(child, ..) => {
                expr = child;
            }
            _ => return expr,
        }
    }
}

fn is_call(expr_kind: &ast::ExprKind<'_>) -> bool {
    matches!(expr_kind, ast::ExprKind::Call(..))
}

fn is_call_or_type(expr_kind: &ast::ExprKind<'_>) -> bool {
    matches!(expr_kind, ast::ExprKind::Call(..) | ast::ExprKind::Type(..))
}

fn is_call_chain(expr_kind: &ast::ExprKind<'_>, must_have_child: bool) -> bool {
    if let ast::ExprKind::Member(child, ..) = expr_kind {
        is_call_chain(&child.kind, false)
    } else {
        !must_have_child && is_call(expr_kind)
    }
}

fn is_call_chain_traverse_bin_ops(expr_kind: &ast::ExprKind<'_>, must_have_child: bool) -> bool {
    match expr_kind {
        ast::ExprKind::Binary(lhs, _, rhs) => {
            is_call_chain_traverse_bin_ops(&lhs.kind, false)
                || is_call_chain_traverse_bin_ops(&rhs.kind, false)
        }
        ast::ExprKind::Member(child, ..) => is_call_chain_traverse_bin_ops(&child.kind, false),
        _ => !must_have_child && is_call(expr_kind),
    }
}

#[derive(Debug)]
struct Decision {
    outcome: bool,
    is_cached: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOpGroup {
    Arithmetic,
    Bitwise,
    Comparison,
    Logical,
}

trait BinOpExt {
    fn group(&self) -> BinOpGroup;
}

impl BinOpExt for ast::BinOpKind {
    fn group(&self) -> BinOpGroup {
        match self {
            Self::Or | Self::And => BinOpGroup::Logical,
            Self::Eq | Self::Ne | Self::Lt | Self::Le | Self::Gt | Self::Ge => {
                BinOpGroup::Comparison
            }
            Self::BitOr | Self::BitXor | Self::BitAnd | Self::Shl | Self::Shr | Self::Sar => {
                BinOpGroup::Bitwise
            }
            Self::Add | Self::Sub | Self::Mul | Self::Div | Self::Rem | Self::Pow => {
                BinOpGroup::Arithmetic
            }
        }
    }
}

/// Calculates the size the callee's "head," excluding its arguments.
///
/// # Examples
///
/// - `myFunction(..)`: 8 (length of `myFunction`)
/// - `uint256(..)`: 7 (length of `uint256`)
/// - `abi.encode(..)`: 10 (length of `abi.encode`)
/// - `foo(..).bar(..)`: 3 (length of `foo`)
pub(super) fn get_callee_head_size(callee: &ast::Expr<'_>) -> usize {
    match &callee.kind {
        ast::ExprKind::Ident(id) => id.as_str().len(),
        ast::ExprKind::Type(ast::Type { kind: ast::TypeKind::Elementary(ty), .. }) => {
            ty.to_abi_str().len()
        }
        ast::ExprKind::Member(base, member_ident) => {
            match &base.kind {
                ast::ExprKind::Ident(..) | ast::ExprKind::Type(..) => {
                    get_callee_head_size(base) + 1 + member_ident.as_str().len()
                }

                // Chainned calls are not traversed, and instead just the member identifier is used
                ast::ExprKind::Member(child, ..)
                    if !matches!(&child.kind, ast::ExprKind::Call(..)) =>
                {
                    get_callee_head_size(base) + 1 + member_ident.as_str().len()
                }
                _ => member_ident.as_str().len(),
            }
        }

        // If the callee is not an identifier or member access, it has no "head"
        _ => 0,
    }
}
