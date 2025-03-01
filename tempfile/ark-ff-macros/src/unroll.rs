//! An attribute-like procedural macro for unrolling for loops with integer
//! literal bounds.
//!
//! This crate provides the [`unroll_for_loops`](../attr.unroll_for_loops.html)
//! attribute-like macro that can be applied to functions containing for-loops
//! with integer bounds. This macro looks for loops to unroll and unrolls them
//! at compile time.
//!
//!
//! ## Usage
//!
//! Just add `#[unroll_for_loops]` above the function whose for loops you would
//! like to unroll. Currently all for loops with integer literal bounds will be
//! unrolled, although this macro currently can't see inside complex code (e.g.
//! for loops within closures).
//!
//!
//! ## Example
//!
//! The following function computes a matrix-vector product and returns the
//! result as an array. Both of the inner for-loops are unrolled when
//! `#[unroll_for_loops]` is applied.
//!
//! ```rust
//! use ark_ff_macros::unroll_for_loops;
//!
//! #[unroll_for_loops(12)]
//! fn mtx_vec_mul(mtx: &[[f64; 5]; 5], vec: &[f64; 5]) -> [f64; 5] {
//!     let mut out = [0.0; 5];
//!     for col in 0..5 {
//!         for row in 0..5 {
//!             out[row] += mtx[col][row] * vec[col];
//!         }
//!     }
//!     out
//! }
//!
//! fn mtx_vec_mul_2(mtx: &[[f64; 5]; 5], vec: &[f64; 5]) -> [f64; 5] {
//!     let mut out = [0.0; 5];
//!     for col in 0..5 {
//!         for row in 0..5 {
//!             out[row] += mtx[col][row] * vec[col];
//!         }
//!     }
//!     out
//! }
//! let a = [[1.0, 2.0, 3.0, 4.0, 5.0]; 5];
//! let b = [7.9, 4.8, 3.8, 4.22, 5.2];
//! assert_eq!(mtx_vec_mul(&a, &b), mtx_vec_mul_2(&a, &b));
//! ```
//!
//! This code was adapted from the [`unroll`](https://crates.io/crates/unroll) crate.

use std::borrow::Borrow;

use syn::{
    parse_quote, token::Brace, Block, Expr, ExprBlock, ExprForLoop, ExprIf, ExprLet, ExprRange,
    Pat, PatIdent, RangeLimits, Stmt,
};

/// Routine to unroll for loops within a block
pub(crate) fn unroll_in_block(block: &Block, unroll_by: usize) -> Block {
    let &Block {
        ref brace_token,
        ref stmts,
    } = block;
    let mut new_stmts = Vec::new();
    for stmt in stmts.iter() {
        if let Stmt::Expr(expr) = stmt {
            new_stmts.push(Stmt::Expr(unroll(expr, unroll_by)));
        } else if let Stmt::Semi(expr, semi) = stmt {
            new_stmts.push(Stmt::Semi(unroll(expr, unroll_by), *semi));
        } else {
            new_stmts.push((*stmt).clone());
        }
    }
    Block {
        brace_token: *brace_token,
        stmts: new_stmts,
    }
}

/// Routine to unroll a for loop statement, or return the statement unchanged if
/// it's not a for loop.
fn unroll(expr: &Expr, unroll_by: usize) -> Expr {
    // impose a scope that we can break out of so we can return stmt without copying it.
    if let Expr::ForLoop(for_loop) = expr {
        let ExprForLoop {
            ref attrs,
            ref label,
            ref pat,
            expr: ref range,
            ref body,
            ..
        } = *for_loop;

        let new_body = unroll_in_block(body, unroll_by);

        let forloop_with_body = |body| {
            Expr::ForLoop(ExprForLoop {
                body,
                ..(*for_loop).clone()
            })
        };

        if let Pat::Ident(PatIdent {
            by_ref,
            mutability,
            ident,
            subpat,
            ..
        }) = pat
        {
            // Don't know how to deal with these so skip and return the original.
            if !by_ref.is_none() || !mutability.is_none() || !subpat.is_none() {
                return forloop_with_body(new_body);
            }
            let idx = ident; // got the index variable name

            if let Expr::Range(ExprRange {
                from, limits, to, ..
            }) = range.borrow()
            {
                // Parse `from` in `from..to`.
                let begin = match from {
                    Some(e) => e.clone(),
                    _ => Box::new(parse_quote!(0usize)),
                };
                let end = match to {
                    Some(e) => e.clone(),
                    _ => return forloop_with_body(new_body),
                };
                let end_is_closed = if let RangeLimits::Closed(_) = limits {
                    1usize
                } else {
                    0
                };
                let end: Expr = parse_quote!(#end + #end_is_closed);

                let preamble: Vec<Stmt> = parse_quote! {
                    let total_iters: usize = (#end).checked_sub(#begin).unwrap_or(0);
                    let num_loops = total_iters / #unroll_by;
                    let remainder = total_iters % #unroll_by;
                };
                let mut block = Block {
                    brace_token: Brace::default(),
                    stmts: preamble,
                };
                let mut loop_expr: ExprForLoop = parse_quote! {
                    for #idx in (0..num_loops) {
                        let mut #idx = #begin + #idx * #unroll_by;
                    }
                };
                let loop_block: Vec<Stmt> = parse_quote! {
                    if #idx < #end {
                        #new_body
                    }
                    #idx += 1;
                };
                let loop_body = (0..unroll_by).flat_map(|_| loop_block.clone());
                loop_expr.body.stmts.extend(loop_body);
                block.stmts.push(Stmt::Expr(Expr::ForLoop(loop_expr)));

                // idx = num_loops * unroll_by;
                block
                    .stmts
                    .push(parse_quote! { let mut #idx = #begin + num_loops * #unroll_by; });
                // if idx < remainder + num_loops * unroll_by { ... }
                let post_loop_block: Vec<Stmt> = parse_quote! {
                    if #idx < #end {
                        #new_body
                    }
                    #idx += 1;
                };
                let post_loop = (0..unroll_by).flat_map(|_| post_loop_block.clone());
                block.stmts.extend(post_loop);

                let mut attrs = attrs.clone();
                attrs.extend(vec![parse_quote!(#[allow(unused)])]);
                Expr::Block(ExprBlock {
                    attrs,
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
            cond: Box::new(unroll(cond, unroll_by)),
            then_branch: unroll_in_block(then_branch, unroll_by),
            else_branch: else_branch
                .as_ref()
                .map(|x| (x.0, Box::new(unroll(&x.1, unroll_by)))),
            ..(*if_expr).clone()
        })
    } else if let Expr::Let(let_expr) = expr {
        let ExprLet { ref expr, .. } = *let_expr;
        Expr::Let(ExprLet {
            expr: Box::new(unroll(expr, unroll_by)),
            ..(*let_expr).clone()
        })
    } else if let Expr::Block(expr_block) = expr {
        let ExprBlock { ref block, .. } = *expr_block;
        Expr::Block(ExprBlock {
            block: unroll_in_block(block, unroll_by),
            ..(*expr_block).clone()
        })
    } else {
        (*expr).clone()
    }
}

#[test]
fn test_expand() {
    use quote::ToTokens;
    let for_loop: Block = parse_quote! {{
        let mut sum = 0;
        for i in 0..8 {
            sum += i;
        }
    }};
    println!("{}", unroll_in_block(&for_loop, 12).to_token_stream());
}
