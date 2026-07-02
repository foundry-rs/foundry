use super::EmptyBlock;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{FunctionKind, ItemFunction, StateMutability};

declare_forge_lint!(EMPTY_BLOCK, Severity::Low, "empty-block", "empty function body");

impl<'ast> EarlyLintPass<'ast> for EmptyBlock {
    fn check_item_function(&mut self, ctx: &LintContext, func: &'ast ItemFunction<'ast>) {
        // An empty body on a regular function is dead or unfinished code. Bodies whose emptiness
        // is the behavior are exempt: constructors, receive/fallback (the empty body accepts
        // calls or ether), `virtual` functions (an empty default meant to be overridden) and
        // `payable` functions (an empty ether sink). Functions without a body (interfaces,
        // abstract declarations) have nothing to flag, and an empty modifier body is a solc
        // compile error (2883), so it never reaches the linter.
        if let Some(body) = &func.body
            && body.is_empty()
            && matches!(func.kind, FunctionKind::Function)
            && func.header.virtual_.is_none()
            && func.header.state_mutability() != StateMutability::Payable
        {
            ctx.emit(&EMPTY_BLOCK, body.span);
        }
    }
}
