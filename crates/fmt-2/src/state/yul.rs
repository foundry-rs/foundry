#![allow(clippy::too_many_arguments)]

use super::{CommentConfig, State, common::*};
use solar_parse::ast::{Span, yul};

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
                self.commasep(
                    paths,
                    stmt.span.lo(),
                    stmt.span.hi(),
                    |this, path| this.print_path(path, false),
                    get_span!(()),
                    ListFormat::Consistent { cmnts_break: false, with_space: false },
                    false,
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
                self.print_yul_block(stmts, span, false);
            }
            yul::StmtKind::For { init, cond, step, body } => {
                self.ibox(0);

                self.word("for ");
                self.print_yul_block(init, span, false);

                self.space();
                self.print_yul_expr(cond);

                self.space();
                self.print_yul_block(step, span, false);

                self.space();
                self.print_yul_block(body, span, false);

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
                    self.print_yul_block(body, span, false);

                    self.print_trailing_comment(selector.span.hi(), None);
                }

                if let Some(default_case) = default_case {
                    self.hardbreak_if_not_bol();
                    self.word("default ");
                    self.print_yul_block(default_case, span, false);
                }
            }
            yul::StmtKind::Leave => self.word("leave"),
            yul::StmtKind::Break => self.word("break"),
            yul::StmtKind::Continue => self.word("continue"),
            yul::StmtKind::FunctionDef(yul::Function { name, parameters, returns, body }) => {
                self.cbox(0);
                self.s.ibox(0);
                self.word("function ");
                self.print_ident(name);
                self.print_tuple(
                    parameters,
                    span.lo(),
                    span.hi(),
                    Self::print_ident,
                    get_span!(),
                    ListFormat::Consistent { cmnts_break: false, with_space: false },
                    false,
                );
                self.nbsp();
                let has_returns = !returns.is_empty();
                let skip_opening_brace = has_returns;
                if has_returns {
                    self.commasep(
                        returns,
                        stmt.span.lo(),
                        stmt.span.hi(),
                        Self::print_ident,
                        get_span!(),
                        ListFormat::Yul { sym_prev: Some("->"), sym_post: Some("{") },
                        false,
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
                    stmt.span.hi(),
                    Self::print_ident,
                    get_span!(),
                    ListFormat::Consistent { cmnts_break: false, with_space: false },
                    false,
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
            yul::ExprKind::Lit(lit) => self.print_lit(lit),
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
            ListFormat::Consistent { cmnts_break: false, with_space: false },
            true,
        );
    }

    pub(super) fn print_yul_block(
        &mut self,
        block: &'ast [yul::Stmt<'ast>],
        span: Span,
        skip_opening_brace: bool,
    ) {
        if self.handle_span(span, false) {
            return;
        }

        if !skip_opening_brace {
            self.print_word("{");
        }

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
        self.print_word("}");
    }
}
