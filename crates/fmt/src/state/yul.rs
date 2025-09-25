#![allow(clippy::too_many_arguments)]

use super::{
    CommentConfig, State,
    common::{BlockFormat, ListFormat},
};
use solar::parse::ast::{self, Span, yul};

#[rustfmt::skip]
macro_rules! get_span {
    () => { |value| Some(value.span) };
    (()) => { |value| Some(value.span()) };
}

/// Language-specific pretty printing: Yul.
impl<'ast> State<'_, 'ast> {
    pub(crate) fn print_yul_stmt(&mut self, stmt: &'ast yul::Stmt<'ast>) {
        let yul::Stmt { ref docs, span, ref kind } = *stmt;
        self.print_docs(docs);
        if self.handle_span(span, false) {
            return;
        }

        match kind {
            yul::StmtKind::Block(stmts) => self.print_yul_block(stmts, span, false),
            yul::StmtKind::AssignSingle(path, expr) => {
                self.print_path(path, false);
                self.word(" := ");
                self.neverbreak();
                self.print_yul_expr(expr);
            }
            yul::StmtKind::AssignMulti(paths, expr_call) => {
                self.ibox(0);
                self.commasep(
                    paths,
                    stmt.span.lo(),
                    stmt.span.hi(),
                    |this, path| this.print_path(path, false),
                    get_span!(()),
                    ListFormat::consistent(),
                );
                self.word(" :=");
                self.space();
                self.s.offset(self.ind);
                self.ibox(0);
                self.print_yul_expr_call(expr_call, span);
                self.end();
                self.end();
            }
            yul::StmtKind::Expr(expr_call) => self.print_yul_expr_call(expr_call, span),
            yul::StmtKind::If(expr, stmts) => {
                self.word("if ");
                self.print_yul_expr(expr);
                self.nbsp();
                self.print_yul_block(stmts, span, false);
            }
            yul::StmtKind::For { init, cond, step, body } => {
                self.ibox(0);

                self.word("for ");
                self.print_yul_block(init, init.span, false);

                self.space();
                self.print_yul_expr(cond);

                self.space();
                self.print_yul_block(step, step.span, false);

                self.space();
                self.print_yul_block(body, body.span, false);

                self.end();
            }
            yul::StmtKind::Switch(yul::StmtSwitch { selector, cases }) => {
                self.word("switch ");
                self.print_yul_expr(selector);

                self.print_trailing_comment(selector.span.hi(), None);

                for yul::StmtSwitchCase { constant, body, span } in cases.iter() {
                    self.hardbreak_if_not_bol();
                    if let Some(constant) = constant {
                        self.print_comments(
                            constant.span.lo(),
                            CommentConfig::default().mixed_prev_space(),
                        );
                        self.word("case ");
                        self.print_lit(constant);
                        self.nbsp();
                    } else {
                        self.print_comments(
                            body.span.lo(),
                            CommentConfig::default().mixed_prev_space(),
                        );
                        self.word("default ");
                    }
                    self.print_yul_block(body, *span, false);

                    self.print_trailing_comment(selector.span.hi(), None);
                }
            }
            yul::StmtKind::Leave => self.word("leave"),
            yul::StmtKind::Break => self.word("break"),
            yul::StmtKind::Continue => self.word("continue"),
            yul::StmtKind::FunctionDef(func) => {
                let yul::Function { name, parameters, returns, body } = func;
                let params_hi = parameters
                    .last()
                    .map_or(returns.first().map_or(body.span.lo(), |r| r.span.lo()), |p| {
                        p.span.hi()
                    });

                self.cbox(0);
                self.s.ibox(0);
                self.word("function ");
                self.print_ident(name);
                self.print_tuple(
                    parameters,
                    span.lo(),
                    params_hi,
                    Self::print_ident,
                    get_span!(),
                    ListFormat::consistent(),
                );
                self.nbsp();
                let has_returns = !returns.is_empty();
                let skip_opening_brace = has_returns;
                if self.can_yul_header_params_be_inlined(func) {
                    self.neverbreak();
                }
                if has_returns {
                    self.commasep(
                        returns,
                        returns.first().map_or(params_hi, |ret| ret.span.lo()),
                        returns.last().map_or(body.span.lo(), |ret| ret.span.hi()),
                        Self::print_ident,
                        get_span!(),
                        ListFormat::yul(Some("->"), Some("{")),
                    );
                }
                self.end();
                self.print_yul_block(body, span, skip_opening_brace);
                self.end();
            }
            yul::StmtKind::VarDecl(idents, expr) => {
                self.s.ibox(self.ind);
                self.word("let ");
                self.commasep(
                    idents,
                    stmt.span.lo(),
                    idents.last().map_or(stmt.span.lo(), |i| i.span.hi()),
                    Self::print_ident,
                    get_span!(),
                    ListFormat::consistent(),
                );
                if let Some(expr) = expr {
                    self.word(" :=");
                    self.space();
                    self.print_yul_expr(expr);
                }
                self.end();
            }
        }
    }

    fn print_yul_expr(&mut self, expr: &'ast yul::Expr<'ast>) {
        let yul::Expr { span, ref kind } = *expr;
        if self.handle_span(span, false) {
            return;
        }

        match kind {
            yul::ExprKind::Path(path) => self.print_path(path, false),
            yul::ExprKind::Call(call) => self.print_yul_expr_call(call, span),
            yul::ExprKind::Lit(lit) => {
                if matches!(&lit.kind, ast::LitKind::Address(_)) {
                    self.print_span_cold(lit.span);
                } else {
                    self.print_lit(lit);
                }
            }
        }
    }

    fn print_yul_expr_call(&mut self, expr: &'ast yul::ExprCall<'ast>, span: Span) {
        let yul::ExprCall { name, arguments } = expr;
        self.print_ident(name);
        self.print_tuple(
            arguments,
            span.lo(),
            span.hi(),
            |s, arg| s.print_yul_expr(arg),
            get_span!(),
            ListFormat::consistent().break_single(true),
        );
    }

    pub(super) fn print_yul_block(
        &mut self,
        block: &'ast yul::Block<'ast>,
        span: Span,
        skip_opening_brace: bool,
    ) {
        if self.handle_span(span, false) {
            return;
        }

        if !skip_opening_brace {
            self.print_word("{");
        }

        let can_inline_block = block.len() <= 1
            && !self.is_multiline_yul_block(block)
            && self.estimate_size(block.span) <= self.space_left();
        if can_inline_block {
            self.neverbreak();
            self.print_block_inner(
                block,
                BlockFormat::NoBraces(None),
                |s, stmt| {
                    s.nbsp();
                    s.print_yul_stmt(stmt);
                    if s.peek_comment_before(stmt.span.hi()).is_none()
                        && s.peek_trailing_comment(stmt.span.hi(), None).is_none()
                    {
                        s.nbsp();
                    }
                    s.print_comments(
                        stmt.span.hi(),
                        CommentConfig::skip_ws().mixed_no_break().mixed_post_nbsp(),
                    );
                    if !s.last_token_is_space() {
                        s.nbsp();
                    }
                },
                |b| b.span,
                span.hi(),
            );
        } else {
            let (mut i, n_args) = (0, block.len().saturating_sub(1));
            self.print_block_inner(
                block,
                BlockFormat::NoBraces(Some(self.ind)),
                |s, stmt| {
                    s.print_yul_stmt(stmt);
                    s.print_comments(stmt.span.hi(), CommentConfig::default());
                    if i != n_args {
                        let next_span = block[i + 1].span;
                        s.print_trailing_comment(stmt.span.hi(), Some(next_span.lo()));
                        if !s.is_bol_or_only_ind() && !s.inline_config.is_disabled(stmt.span) {
                            // when disabling a single line, manually add a nonbreaking line jump so
                            // that the indentation of the disabled line is maintained.
                            if s.inline_config.is_disabled(next_span)
                                && s.peek_comment_before(next_span.lo())
                                    .is_none_or(|cmnt| !cmnt.style.is_isolated())
                            {
                                s.word("\n");
                            // otherwise, use a regular hardbreak
                            } else {
                                s.hardbreak_if_not_bol();
                            }
                        }
                        i += 1;
                    } else {
                        s.print_trailing_comment(stmt.span.hi(), Some(span.hi()));
                    }
                },
                |b| b.span,
                span.hi(),
            );
        }
        self.print_word("}");
        self.print_trailing_comment(span.hi(), None);
    }

    /// Checks if a block statement `{ ... }` contains more than one line of actual code.
    fn is_multiline_yul_block(&self, block: &'ast yul::Block<'ast>) -> bool {
        if block.stmts.is_empty() {
            return false;
        }
        if self.sm.is_multiline(block.span)
            && let Ok(snip) = self.sm.span_to_snippet(block.span)
        {
            let code_lines = snip.lines().filter(|line| {
                let trimmed = line.trim();
                // Ignore empty lines and lines with only '{' or '}'
                !trimmed.is_empty()
            });
            return code_lines.count() > 1;
        }
        false
    }

    fn estimate_yul_header_params_size(&mut self, func: &yul::Function<'_>) -> usize {
        // '(' + param + (', ' + param) + ')'
        let params = func
            .parameters
            .iter()
            .fold(0, |len, p| if len != 0 { len + 2 } else { 2 } + self.estimate_size(p.span));

        // 'function ' + name + ' ' + params + ' ->'
        9 + self.estimate_size(func.name.span) + 1 + params + 3
    }

    fn can_yul_header_params_be_inlined(&mut self, func: &yul::Function<'_>) -> bool {
        self.estimate_yul_header_params_size(func) <= self.space_left()
    }
}
