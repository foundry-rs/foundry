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
            yul::StmtKind::Block(stmts) => {
                self.print_yul_block(stmts, span, self.can_yul_block_be_inlined(stmts), false)
            }
            yul::StmtKind::AssignSingle(path, expr) => {
                self.print_path(path, false);
                self.word(" := ");
                self.neverbreak();
                self.print_yul_expr(expr);
            }
            yul::StmtKind::AssignMulti(paths, expr_call) => {
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
                self.print_yul_expr_call(expr_call);
                self.end();
            }
            yul::StmtKind::Expr(expr_call) => self.print_yul_expr_call(expr_call),
            yul::StmtKind::If(expr, stmts) => {
                self.word("if ");
                self.print_yul_expr(expr);
                self.nbsp();
                self.print_yul_block(stmts, span, self.can_yul_block_be_inlined(stmts), false);
            }
            yul::StmtKind::For { init, cond, step, body } => {
                self.ibox(0);

                self.word("for ");
                self.print_yul_block(init, init.span, self.can_yul_block_be_inlined(init), false);

                self.space();
                self.print_yul_expr(cond);

                self.space();
                self.print_yul_block(step, step.span, self.can_yul_block_be_inlined(step), false);

                self.space();
                self.print_yul_block(body, body.span, self.can_yul_block_be_inlined(body), false);

                self.end();
            }
            yul::StmtKind::Switch(yul::StmtSwitch { selector, branches, default_case }) => {
                self.word("switch ");
                self.print_yul_expr(selector);

                self.print_trailing_comment(selector.span.hi(), None);

                for yul::StmtSwitchCase { constant, body } in branches.iter() {
                    self.hardbreak_if_not_bol();
                    self.word("case ");
                    self.print_lit(constant);
                    self.nbsp();
                    self.print_yul_block(body, span, self.can_yul_block_be_inlined(body), false);

                    self.print_trailing_comment(selector.span.hi(), None);
                }

                if let Some(default_case) = default_case {
                    self.hardbreak_if_not_bol();
                    self.word("default ");
                    self.print_yul_block(
                        default_case,
                        span,
                        self.can_yul_block_be_inlined(default_case),
                        false,
                    );
                }
            }
            yul::StmtKind::Leave => self.word("leave"),
            yul::StmtKind::Break => self.word("break"),
            yul::StmtKind::Continue => self.word("continue"),
            yul::StmtKind::FunctionDef(func) => {
                let yul::Function { name, parameters, returns, body } = func;
                let params_hi = parameters
                    .last()
                    .map_or(returns.first().map_or(span.hi(), |r| r.span.lo()), |p| p.span.hi());

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
                        returns.last().map_or(span.hi(), |ret| ret.span.hi()),
                        Self::print_ident,
                        get_span!(),
                        ListFormat::yul(Some("->"), Some("{")),
                    );
                }
                self.end();
                self.print_yul_block(
                    body,
                    span,
                    self.can_yul_block_be_inlined(body),
                    skip_opening_brace,
                );
                self.end();
            }
            yul::StmtKind::VarDecl(idents, expr) => {
                self.s.ibox(self.ind);
                self.word("let ");
                self.commasep(
                    idents,
                    stmt.span.lo(),
                    stmt.span.hi(),
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
            yul::ExprKind::Call(call) => self.print_yul_expr_call(call),
            yul::ExprKind::Lit(lit) => {
                if matches!(&lit.kind, ast::LitKind::Address(_)) {
                    self.print_span_cold(lit.span);
                } else {
                    self.print_lit(lit);
                }
            }
        }
    }

    fn print_yul_expr_call(&mut self, expr: &'ast yul::ExprCall<'ast>) {
        let yul::ExprCall { name, arguments } = expr;
        self.print_ident(name);
        self.print_tuple(
            arguments,
            Span::DUMMY.lo(),
            Span::DUMMY.hi(),
            Self::print_yul_expr,
            get_span!(),
            ListFormat::consistent().break_single_if(true),
        );
    }

    pub(super) fn print_yul_block(
        &mut self,
        block: &'ast [yul::Stmt<'ast>],
        span: Span,
        inline: bool,
        skip_opening_brace: bool,
    ) {
        if self.handle_span(span, false) {
            return;
        }

        if !skip_opening_brace {
            self.print_word("{");
        }

        if inline {
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
                    s.print_trailing_comment(stmt.span.hi(), None);
                },
                |b| b.span,
                span.hi(),
            );
        } else {
            let mut i = if block.is_empty() { 0 } else { block.len() - 1 };
            self.print_block_inner(
                block,
                BlockFormat::NoBraces(Some(self.ind)),
                |s, stmt| {
                    s.print_yul_stmt(stmt);
                    s.print_comments(stmt.span.hi(), CommentConfig::default());
                    s.print_trailing_comment(stmt.span.hi(), None);
                    if i != 0 {
                        s.hardbreak_if_not_bol();
                        i -= 1;
                    }
                },
                |b| b.span,
                span.hi(),
            );
        }
        self.print_word("}");
    }

    fn can_yul_block_be_inlined(&self, block: &'ast yul::Block<'ast>) -> bool {
        if block.len() > 1 || self.is_multiline_yul_block(block) {
            false
        } else {
            self.estimate_size(block.span) <= self.space_left()
        }
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
