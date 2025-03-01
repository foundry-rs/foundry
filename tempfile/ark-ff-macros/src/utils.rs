use std::str::FromStr;

use num_bigint::{BigInt, Sign};
use proc_macro::TokenStream;
use syn::{Expr, Lit};

pub fn parse_string(input: TokenStream) -> Option<String> {
    let input: Expr = syn::parse(input).unwrap();
    let input = if let Expr::Group(syn::ExprGroup { expr, .. }) = input {
        expr
    } else {
        panic!("could not parse");
    };
    match *input {
        Expr::Lit(expr_lit) => match expr_lit.lit {
            Lit::Str(s) => Some(s.value()),
            _ => None,
        },
        _ => None,
    }
}

pub fn str_to_limbs(num: &str) -> (bool, Vec<String>) {
    let (sign, limbs) = str_to_limbs_u64(num);
    (sign, limbs.into_iter().map(|l| format!("{l}u64")).collect())
}

pub fn str_to_limbs_u64(num: &str) -> (bool, Vec<u64>) {
    let (sign, digits) = BigInt::from_str(num)
        .expect("could not parse to bigint")
        .to_radix_le(16);
    let limbs = digits
        .chunks(16)
        .map(|chunk| {
            let mut this = 0u64;
            for (i, hexit) in chunk.iter().enumerate() {
                this += (*hexit as u64) << (4 * i);
            }
            this
        })
        .collect::<Vec<_>>();

    let sign_is_positive = sign != Sign::Minus;
    (sign_is_positive, limbs)
}
