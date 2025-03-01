#![warn(
    unused,
    future_incompatible,
    nonstandard_style,
    rust_2018_idioms,
    rust_2021_compatibility
)]
#![forbid(unsafe_code)]
#![recursion_limit = "128"]

use proc_macro::TokenStream;
use syn::{
    parse::{Parse, ParseStream},
    Expr,
};

mod context;
use context::*;

use std::cell::RefCell;

const MAX_REGS: usize = 6;

struct AsmMulInput {
    num_limbs: Box<Expr>,
    a: Expr,
    b: Expr,
}

impl Parse for AsmMulInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let input = input
            .parse_terminated::<_, syn::token::Comma>(Expr::parse)?
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
            .parse_terminated::<_, syn::token::Comma>(Expr::parse)?
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

fn construct_asm_mul(ctx: &Context<'_>, limbs: usize) -> Vec<String> {
    let r: Vec<AssemblyVar> = Context::R.iter().map(|r| (*r).into()).collect();
    let rax: AssemblyVar = Context::RAX.into();
    let rcx: AssemblyVar = Context::RCX.into();
    let rdx: AssemblyVar = Context::RDX.into();
    let rsi: AssemblyVar = Context::RSI.into();
    let a: AssemblyVar = ctx.get_decl("a").into();
    let b: AssemblyVar = ctx.get_decl_with_fallback("b", "a").into(); // "b" is not available during squaring.
    let modulus: AssemblyVar = ctx.get_decl("modulus").into();
    let mod_inv: AssemblyVar = ctx.get_decl("mod_inv").into();

    let asm_instructions = RefCell::new(Vec::new());

    let comment = |comment: &str| {
        asm_instructions
            .borrow_mut()
            .push(format!("// {}", comment));
    };

    macro_rules! mulxq {
        ($a: expr, $b: expr, $c: expr) => {
            asm_instructions
                .borrow_mut()
                .push(format!("mulxq {}, {}, {}", &$a, &$b, &$c));
        };
    }

    macro_rules! adcxq {
        ($a: expr, $b: expr) => {
            asm_instructions
                .borrow_mut()
                .push(format!("adcxq {}, {}", &$a, &$b));
        };
    }

    macro_rules! adoxq {
        ($a: expr, $b: expr) => {
            asm_instructions
                .borrow_mut()
                .push(format!("adoxq {}, {}", &$a, &$b));
        };
    }

    macro_rules! movq {
        ($a: expr, $b: expr) => {{
            asm_instructions
                .borrow_mut()
                .push(format!("movq {}, {}", &$a, &$b));
        }};
    }

    macro_rules! xorq {
        ($a: expr, $b: expr) => {
            asm_instructions
                .borrow_mut()
                .push(format!("xorq {}, {}", &$a, &$b))
        };
    }

    macro_rules! movq_zero {
        ($a: expr) => {
            asm_instructions
                .borrow_mut()
                .push(format!("movq $0, {}", &$a))
        };
    }

    macro_rules! mul_1 {
        ($a:expr, $b:ident, $limbs:expr) => {
            comment("Mul 1 start");
            movq!($a, rdx);
            mulxq!($b[0], r[0], r[1]);
            for j in 1..$limbs - 1 {
                mulxq!($b[j], rax, r[((j + 1) % $limbs)]);
                adcxq!(rax, r[j]);
            }
            mulxq!($b[$limbs - 1], rax, rcx);
            movq_zero!(rsi);
            adcxq!(rax, r[$limbs - 1]);
            adcxq!(rsi, rcx);
            comment("Mul 1 end")
        };
    }

    macro_rules! mul_add_1 {
        ($a:ident, $b:ident, $i:ident, $limbs:expr) => {
            comment(&format!("mul_add_1 start for iteration {}", $i));
            movq!($a[$i], rdx);
            for j in 0..$limbs - 1 {
                mulxq!($b[j], rax, rsi);
                adcxq!(rax, r[(j + $i) % $limbs]);
                adoxq!(rsi, r[(j + $i + 1) % $limbs]);
            }
            mulxq!($b[$limbs - 1], rax, rcx);
            movq_zero!(rsi);
            adcxq!(rax, r[($i + $limbs - 1) % $limbs]);
            adoxq!(rsi, rcx);
            adcxq!(rsi, rcx);
            comment(&format!("mul_add_1 end for iteration {}", $i));
        };
    }

    macro_rules! mul_add_shift_1 {
        ($a:ident, $mod_inv:ident, $i:ident, $limbs:expr) => {
            comment(&format!("mul_add_shift_1 start for iteration {}", $i));
            movq!($mod_inv, rdx);
            mulxq!(r[$i], rdx, rax);
            mulxq!($a[0], rax, rsi);
            adcxq!(r[$i % $limbs], rax);
            adoxq!(rsi, r[($i + 1) % $limbs]);
            for j in 1..$limbs - 1 {
                mulxq!($a[j], rax, rsi);
                adcxq!(rax, r[(j + $i) % $limbs]);
                adoxq!(rsi, r[(j + $i + 1) % $limbs]);
            }
            mulxq!($a[$limbs - 1], rax, r[$i % $limbs]);
            movq_zero!(rsi);
            adcxq!(rax, r[($i + $limbs - 1) % $limbs]);
            adoxq!(rcx, r[$i % $limbs]);
            adcxq!(rsi, r[$i % $limbs]);
            comment(&format!("mul_add_shift_1 end for iteration {}", $i));
        };
    }
    {
        let a1 = a.memory_accesses(limbs);
        let b1 = b.memory_accesses(limbs);
        let m1 = modulus.memory_accesses(limbs);

        xorq!(rcx, rcx);
        for i in 0..limbs {
            if i == 0 {
                mul_1!(a1[0], b1, limbs);
            } else {
                mul_add_1!(a1, b1, i, limbs);
            }
            mul_add_shift_1!(m1, mod_inv, i, limbs);
        }

        comment("Moving results into `a`");
        for i in 0..limbs {
            movq!(r[i], a1[i]);
        }
    }
    asm_instructions.into_inner()
}

fn generate_impl(num_limbs: usize, is_mul: bool) -> String {
    let mut ctx = Context::new();
    ctx.add_declaration("a", "a");
    if is_mul {
        ctx.add_declaration("b", "b");
    }
    ctx.add_declaration("modulus", "&Self::MODULUS.0");
    ctx.add_declaration("mod_inv", "Self::INV");

    if num_limbs > MAX_REGS {
        ctx.add_buffer(2 * num_limbs);
        ctx.add_declaration("buf", "&mut spill_buffer");
    }

    let asm_instructions = construct_asm_mul(&ctx, num_limbs);

    ctx.add_asm(&asm_instructions);
    ctx.add_clobbers(
        [Context::RAX, Context::RCX, Context::RSI, Context::RDX]
            .iter()
            .copied(),
    );
    ctx.add_clobbers(Context::R.iter().take(std::cmp::min(num_limbs, 8)).copied());
    ctx.build()
}

mod tests {
    #[test]
    fn expand_muls() {
        let impl_block = super::generate_impl(4, true);
        println!("{}", impl_block);
    }
}
