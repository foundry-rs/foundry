//! An attribute-like procedural macro for unrolling for loops with integer literal bounds.
//!
//! This crate provides the [`unroll_for_loops`](../attr.unroll_for_loops.html) attribute-like macro that can be applied to
//! functions containing for-loops with integer bounds. This macro looks for loops to unroll and
//! unrolls them at compile time.
//!
//!
//! ## Usage
//!
//! Just add `#[unroll_for_loops]` above the function whose for loops you would like to unroll.
//! Currently all for loops with integer literal bounds will be unrolled, although this macro
//! currently can't see inside complex code (e.g. for loops within closures).
//!
//!
//! ## Example
//!
//! The following function computes a matrix-vector product and returns the result as an array.
//! Both of the inner for-loops are unrolled when `#[unroll_for_loops]` is applied.
//!
//! ```rust
//! use ark_ff_asm::unroll_for_loops;
//!
//! #[unroll_for_loops]
//! fn mtx_vec_mul(mtx: &[[f64; 5]; 5], vec: &[f64; 5]) -> [f64; 5] {
//!     let mut out = [0.0; 5];
//!     for col in 0..5 {
//!         for row in 0..5 {
//!             out[row] += mtx[col][row] * vec[col];
//!         }
//!     }
//!     out
//! }
//! ```
//!
//! This code was adapted from the [`unroll`](https://crates.io/crates/unroll) crate.

use syn::token::Brace;
use syn::{
    parse_quote, Block, Expr, ExprBlock, ExprForLoop, ExprIf, ExprLet, ExprLit, ExprRange, Lit,
    Pat, PatIdent, RangeLimits, Stmt,
};

/// Routine to unroll for loops within a block
pub(crate) fn unroll_in_block(block: &Block) -> Block {
    let &Block {
        ref brace_token,
        ref stmts,
    } = block;
    let mut new_stmts = Vec::new();
    for stmt in stmts.iter() {
        if let Stmt::Expr(expr) = stmt {
            new_stmts.push(Stmt::Expr(unroll(expr)));
        } else if let Stmt::Semi(expr, semi) = stmt {
            new_stmts.push(Stmt::Semi(unroll(expr), *semi));
        } else {
            new_stmts.push((*stmt).clone());
        }
    }
    Block {
        brace_token: *brace_token,
        stmts: new_stmts,
    }
}

/// Routine to unroll a for loop statement, or return the statement unchanged if it's not a for
/// loop.
fn unroll(expr: &Expr) -> Expr {
    // impose a scope that we can break out of so we can return stmt without copying it.
    if let Expr::ForLoop(for_loop) = expr {
        let ExprForLoop {
            ref attrs,
            ref label,
            ref pat,
            expr: ref range_expr,
            ref body,
            ..
        } = *for_loop;

        let new_body = unroll_in_block(&*body);

        let forloop_with_body = |body| {
            Expr::ForLoop(ExprForLoop {
                body,
                ..(*for_loop).clone()
            })
        };

        if let Pat::Ident(PatIdent {
            ref by_ref,
            ref mutability,
            ref ident,
            ref subpat,
            ..
        }) = *pat
        {
            // Don't know how to deal with these so skip and return the original.
            if !by_ref.is_none() || !mutability.is_none() || !subpat.is_none() {
                return forloop_with_body(new_body);
            }
            let idx = ident; // got the index variable name

            if let Expr::Range(ExprRange {
                from: ref mb_box_from,
                ref limits,
                to: ref mb_box_to,
                ..
            }) = **range_expr
            {
                // Parse mb_box_from
                let begin = if let Some(ref box_from) = *mb_box_from {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Int(ref lit_int),
                        ..
                    }) = **box_from
                    {
                        lit_int.base10_parse::<usize>().unwrap()
                    } else {
                        return forloop_with_body(new_body);
                    }
                } else {
                    0
                };

                // Parse mb_box_to
                let end = if let Some(ref box_to) = *mb_box_to {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Int(ref lit_int),
                        ..
                    }) = **box_to
                    {
                        lit_int.base10_parse::<usize>().unwrap()
                    } else {
                        return forloop_with_body(new_body);
                    }
                } else {
                    // we need to know where the limit is to know how much to unroll by.
                    return forloop_with_body(new_body);
                } + if let RangeLimits::Closed(_) = limits {
                    1
                } else {
                    0
                };

                let mut stmts = Vec::new();
                for i in begin..end {
                    let declare_i: Stmt = parse_quote! {
                        #[allow(non_upper_case_globals)]
                        const #idx: usize = #i;
                    };
                    let mut augmented_body = new_body.clone();
                    augmented_body.stmts.insert(0, declare_i);
                    stmts.push(parse_quote! { #augmented_body });
                }
                let block = Block {
                    brace_token: Brace::default(),
                    stmts,
                };
                Expr::Block(ExprBlock {
                    attrs: attrs.clone(),
                    label: label.clone(),
                    block,
                })
            } else {
                forloop_with_body(new_body)
            }
        } else {
            forloop_with_body(new_body)
        }
    } else if let Expr::If(if_expr) = expr {
        let ExprIf {
            ref cond,
            ref then_branch,
            ref else_branch,
            ..
        } = *if_expr;
        Expr::If(ExprIf {
            cond: Box::new(unroll(&**cond)),
            then_branch: unroll_in_block(&*then_branch),
            else_branch: else_branch.as_ref().map(|x| (x.0, Box::new(unroll(&*x.1)))),
            ..(*if_expr).clone()
        })
    } else if let Expr::Let(let_expr) = expr {
        let ExprLet { ref expr, .. } = *let_expr;
        Expr::Let(ExprLet {
            expr: Box::new(unroll(&**expr)),
            ..(*let_expr).clone()
        })
    } else if let Expr::Block(expr_block) = expr {
        let ExprBlock { ref block, .. } = *expr_block;
        Expr::Block(ExprBlock {
            block: unroll_in_block(&*block),
            ..(*expr_block).clone()
        })
    } else {
        (*expr).clone()
    }
}
