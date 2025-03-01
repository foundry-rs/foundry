#![warn(unused, future_incompatible, nonstandard_style, rust_2018_idioms)]
#![forbid(unsafe_code)]

use num_bigint::{BigInt, Sign};
use proc_macro::TokenStream;
use std::str::FromStr;
use syn::{Expr, Lit};

fn parse_string(input: TokenStream) -> Option<String> {
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

fn str_to_limbs(num: &str) -> (bool, Vec<String>) {
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
            format!("{}u64", this)
        })
        .collect::<Vec<_>>();

    let sign_is_positive = sign != Sign::Minus;
    (sign_is_positive, limbs)
}

#[proc_macro]
pub fn to_sign_and_limbs(input: TokenStream) -> TokenStream {
    let num = parse_string(input).expect("expected decimal string");
    let (is_positive, limbs) = str_to_limbs(&num);

    let limbs: String = limbs.join(", ");
    let limbs_and_sign = format!("({}", is_positive) + ", [" + &limbs + "])";
    let tuple: Expr = syn::parse_str(&limbs_and_sign).unwrap();
    quote::quote!(#tuple).into()
}

#[test]
fn test_str_to_limbs() {
    let (is_positive, limbs) = str_to_limbs("-5");
    assert!(!is_positive);
    assert_eq!(&limbs, &["5u64".to_string()]);

    let (is_positive, limbs) = str_to_limbs("100");
    assert!(is_positive);
    assert_eq!(&limbs, &["100u64".to_string()]);

    let large_num = -((1i128 << 64) + 101234001234i128);
    let (is_positive, limbs) = str_to_limbs(&large_num.to_string());
    assert!(!is_positive);
    assert_eq!(&limbs, &["101234001234u64".to_string(), "1u64".to_string()]);

    let num = "80949648264912719408558363140637477264845294720710499478137287262712535938301461879813459410946";
    let (is_positive, limbs) = str_to_limbs(&num.to_string());
    assert!(is_positive);
    let expected_limbs = [
        format!("{}u64", 0x8508c00000000002u64),
        format!("{}u64", 0x452217cc90000000u64),
        format!("{}u64", 0xc5ed1347970dec00u64),
        format!("{}u64", 0x619aaf7d34594aabu64),
        format!("{}u64", 0x9b3af05dd14f6ecu64),
    ];
    assert_eq!(&limbs, &expected_limbs);
}
