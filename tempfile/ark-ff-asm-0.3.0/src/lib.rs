#![warn(unused, future_incompatible, nonstandard_style, rust_2018_idioms)]
#![forbid(unsafe_code)]
#![recursion_limit = "128"]

use proc_macro::TokenStream;
use syn::{
    parse::{Parse, ParseStream},
    Expr, Item, ItemFn,
};

#[macro_use]
mod utils;
use utils::*;

mod context;
use context::*;

mod unroll;

use std::cell::RefCell;

const MAX_REGS: usize = 6;

/// Attribute used to unroll for loops found inside a function block.
#[proc_macro_attribute]
pub fn unroll_for_loops(_meta: TokenStream, input: TokenStream) -> TokenStream {
    let item: Item = syn::parse(input).expect("Failed to parse input.");

    if let Item::Fn(item_fn) = item {
        let new_block = {
            let &ItemFn {
                block: ref box_block,
                ..
            } = &item_fn;
            unroll::unroll_in_block(&**box_block)
        };
        let new_item = Item::Fn(ItemFn {
            block: Box::new(new_block),
            ..item_fn
        });
        quote::quote! ( #new_item ).into()
    } else {
        quote::quote! ( #item ).into()
    }
}

struct AsmMulInput {
    num_limbs: Box<Expr>,
    a: Expr,
    b: Expr,
}

impl Parse for AsmMulInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let input = input
            .parse_terminated::<_, syn::Token![,]>(Expr::parse)?
            .into_iter()
            .collect::<Vec<_>>();
        let num_limbs = input[0].clone();
        let a = input[1].clone();
        let b = input[2].clone();

        let num_limbs = if let Expr::Group(syn::ExprGroup { expr, .. }) = num_limbs {
            expr
        } else {
            Box::new(num_limbs)
        };
        let output = Self { num_limbs, a, b };
        Ok(output)
    }
}

#[proc_macro]
pub fn x86_64_asm_mul(input: TokenStream) -> TokenStream {
    let AsmMulInput { num_limbs, a, b } = syn::parse_macro_input!(input);
    let num_limbs = if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(ref lit_int),
        ..
    }) = &*num_limbs
    {
        lit_int.base10_parse::<usize>().unwrap()
    } else {
        panic!("The number of limbs must be a literal");
    };
    if num_limbs <= 6 && num_limbs <= 3 * MAX_REGS {
        let impl_block = generate_impl(num_limbs, true);

        let inner_ts: Expr = syn::parse_str(&impl_block).unwrap();
        let ts = quote::quote! {
            let a = &mut #a;
            let b = &#b;
            #inner_ts
        };
        ts.into()
    } else {
        TokenStream::new()
    }
}

struct AsmSquareInput {
    num_limbs: Box<Expr>,
    a: Expr,
}

impl Parse for AsmSquareInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let input = input
            .parse_terminated::<_, syn::Token![,]>(Expr::parse)?
            .into_iter()
            .collect::<Vec<_>>();
        let num_limbs = input[0].clone();
        let a = input[1].clone();

        let num_limbs = if let Expr::Group(syn::ExprGroup { expr, .. }) = num_limbs {
            expr
        } else {
            Box::new(num_limbs)
        };
        let output = Self { num_limbs, a };
        Ok(output)
    }
}

#[proc_macro]
pub fn x86_64_asm_square(input: TokenStream) -> TokenStream {
    let AsmSquareInput { num_limbs, a } = syn::parse_macro_input!(input);
    let num_limbs = if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(ref lit_int),
        ..
    }) = &*num_limbs
    {
        lit_int.base10_parse::<usize>().unwrap()
    } else {
        panic!("The number of limbs must be a literal");
    };
    if num_limbs <= 6 && num_limbs <= 3 * MAX_REGS {
        let impl_block = generate_impl(num_limbs, false);

        let inner_ts: Expr = syn::parse_str(&impl_block).unwrap();
        let ts = quote::quote! {
            let a = &mut #a;
            #inner_ts
        };
        ts.into()
    } else {
        TokenStream::new()
    }
}

fn generate_llvm_asm_mul_string(
    a: &str,
    b: &str,
    modulus: &str,
    zero: &str,
    mod_prime: &str,
    limbs: usize,
) -> String {
    let llvm_asm_string = RefCell::new(String::new());

    let begin = || llvm_asm_string.borrow_mut().push_str("\"");

    let end = || {
        llvm_asm_string.borrow_mut().push_str(
            "
                                \"",
        )
    };

    let _comment = |comment: &str| {
        llvm_asm_string
            .borrow_mut()
            .push_str(&format!("         // {}", comment));
    };

    let mulxq = |a: &str, b: &str, c: &str| {
        llvm_asm_string.borrow_mut().push_str(&format!(
            "
                                mulxq {}, {}, {}",
            a, b, c
        ));
    };

    let adcxq = |a: &str, b: &str| {
        llvm_asm_string.borrow_mut().push_str(&format!(
            "
                                adcxq {}, {}",
            a, b
        ));
    };

    let adoxq = |a: &str, b: &str| {
        llvm_asm_string.borrow_mut().push_str(&format!(
            "
                                adoxq {}, {}",
            a, b
        ));
    };

    let movq = |a: &str, b: &str| {
        llvm_asm_string.borrow_mut().push_str(&format!(
            "
                                movq {}, {}",
            a, b
        ));
    };

    let xorq = |a: &str, b: &str| {
        llvm_asm_string.borrow_mut().push_str(&format!(
            "
                                xorq {}, {}",
            a, b
        ));
    };

    macro_rules! mul_1 {
        ($a:expr, $b:ident, $zero:ident, $limbs:expr) => {
            movq($a, RDX);
            mulxq($b[0], R[0], R[1]);
            for j in 1..$limbs - 1 {
                mulxq($b[j], RAX, R[((j + 1) % $limbs)]);
                adcxq(RAX, R[j]);
            }
            mulxq($b[$limbs - 1], RAX, RCX);
            movq($zero, RBX);
            adcxq(RAX, R[$limbs - 1]);
            adcxq(RBX, RCX);
        };
    }

    macro_rules! mul_add_1 {
        ($a:ident, $b:ident, $zero:ident, $i:ident, $limbs:expr) => {
            movq($a[$i], RDX);
            for j in 0..$limbs - 1 {
                mulxq($b[j], RAX, RBX);
                adcxq(RAX, R[(j + $i) % $limbs]);
                adoxq(RBX, R[(j + $i + 1) % $limbs]);
            }
            mulxq($b[$limbs - 1], RAX, RCX);
            movq($zero, RBX);
            adcxq(RAX, R[($i + $limbs - 1) % $limbs]);
            adoxq(RBX, RCX);
            adcxq(RBX, RCX);
        };
    }

    macro_rules! mul_add_shift_1 {
        ($a:ident, $mod_prime:ident, $zero:ident, $i:ident, $limbs:expr) => {
            movq($mod_prime, RDX);
            mulxq(R[$i], RDX, RAX);
            mulxq($a[0], RAX, RBX);
            adcxq(R[$i % $limbs], RAX);
            adoxq(RBX, R[($i + 1) % $limbs]);
            for j in 1..$limbs - 1 {
                mulxq($a[j], RAX, RBX);
                adcxq(RAX, R[(j + $i) % $limbs]);
                adoxq(RBX, R[(j + $i + 1) % $limbs]);
            }
            mulxq($a[$limbs - 1], RAX, R[$i % $limbs]);
            movq($zero, RBX);
            adcxq(RAX, R[($i + $limbs - 1) % $limbs]);
            adoxq(RCX, R[$i % $limbs]);
            adcxq(RBX, R[$i % $limbs]);
        };
    }
    begin();
    {
        reg!(a0, a1, a, limbs);
        reg!(b0, b1, b, limbs);
        reg!(m, m1, modulus, limbs);

        xorq(RCX, RCX);
        for i in 0..limbs {
            if i == 0 {
                mul_1!(a1[0], b1, zero, limbs);
            } else {
                mul_add_1!(a1, b1, zero, i, limbs);
            }
            mul_add_shift_1!(m1, mod_prime, zero, i, limbs);
        }

        for i in 0..limbs {
            movq(R[i], a1[i]);
        }
    }
    end();
    llvm_asm_string.into_inner()
}

fn generate_impl(num_limbs: usize, is_mul: bool) -> String {
    let mut ctx = Context::new();
    ctx.add_declaration("a", "r", "a");
    if is_mul {
        ctx.add_declaration("b", "r", "b");
    }
    ctx.add_declaration("modulus", "r", "&P::MODULUS.0");
    ctx.add_declaration("0", "i", "0u64");
    ctx.add_declaration("mod_prime", "i", "P::INV");

    if num_limbs > MAX_REGS {
        ctx.add_buffer(2 * num_limbs);
        ctx.add_declaration("buf", "r", "&mut spill_buffer");
    }

    let llvm_asm_string = generate_llvm_asm_mul_string(
        &ctx.clone().get("a"),
        &ctx.clone().try_get("b", "a"),
        &ctx.clone().get("modulus"),
        &ctx.clone().get("0"),
        &ctx.clone().get("mod_prime"),
        num_limbs,
    );

    ctx.add_llvm_asm(llvm_asm_string);
    ctx.add_clobber_from_vec(vec!["rcx", "rbx", "rdx", "rax"]);
    for clobber in REG_CLOBBER.iter().take(std::cmp::min(num_limbs, 8)) {
        ctx.add_clobber(clobber);
    }
    ctx.add_clobber_from_vec(vec!["cc", "memory"]);
    ctx.build();
    format!("{{ {} }}", ctx.get_string())
}
