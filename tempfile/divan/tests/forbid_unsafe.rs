// Exhaustively tests that macros work when linting against `unsafe`.

#![forbid(unsafe_code)]

use divan::Bencher;

const CONST_VALUES: [usize; 3] = [1, 5, 10];

#[divan::bench]
fn freestanding() {}

#[divan::bench(types = [i32, &str])]
fn freestanding_generic_type<T>() {}

#[divan::bench(consts = [1, 5, 10])]
fn freestanding_generic_const1<const N: usize>() {}

#[divan::bench(consts = CONST_VALUES)]
fn freestanding_generic_const2<const N: usize>() {}

#[divan::bench(types = [i32, &str], consts = [1, 5, 10])]
fn freestanding_generic_type_const1<T, const N: usize>() {}

#[divan::bench(types = [i32, &str], consts = CONST_VALUES)]
fn freestanding_generic_type_const2<T, const N: usize>() {}

#[divan::bench]
fn contextual(_: Bencher) {}

#[divan::bench(types = [i32, &str])]
fn contextual_generic_type<T>(_: Bencher) {}

#[divan::bench(consts = [1, 5, 10])]
fn contextual_generic_const_1<const N: usize>(_: Bencher) {}

#[divan::bench(consts = CONST_VALUES)]
fn contextual_generic_const_2<const N: usize>(_: Bencher) {}

#[divan::bench(types = [i32, &str], consts = [1, 5, 10])]
fn contextual_generic_type_const_1<T, const N: usize>(_: Bencher) {}

#[divan::bench(types = [i32, &str], consts = CONST_VALUES)]
fn contextual_generic_type_const_2<T, const N: usize>(_: Bencher) {}

#[divan::bench_group]
mod group {
    use super::*;

    #[divan::bench]
    fn freestanding() {}

    #[divan::bench(types = [i32, &str])]
    fn freestanding_generic_type<T>() {}

    #[divan::bench(consts = [1, 5, 10])]
    fn freestanding_generic_const1<const N: usize>() {}

    #[divan::bench(consts = CONST_VALUES)]
    fn freestanding_generic_const2<const N: usize>() {}

    #[divan::bench(types = [i32, &str], consts = [1, 5, 10])]
    fn freestanding_generic_type_const1<T, const N: usize>() {}

    #[divan::bench(types = [i32, &str], consts = CONST_VALUES)]
    fn freestanding_generic_type_const2<T, const N: usize>() {}

    #[divan::bench]
    fn contextual(_: Bencher) {}

    #[divan::bench(types = [i32, &str])]
    fn contextual_generic_type<T>(_: Bencher) {}

    #[divan::bench(consts = [1, 5, 10])]
    fn contextual_generic_const1<const N: usize>(_: Bencher) {}

    #[divan::bench(consts = CONST_VALUES)]
    fn contextual_generic_const2<const N: usize>(_: Bencher) {}

    #[divan::bench(types = [i32, &str], consts = [1, 5, 10])]
    fn contextual_generic_type_const1<T, const N: usize>(_: Bencher) {}

    #[divan::bench(types = [i32, &str], consts = CONST_VALUES)]
    fn contextual_generic_type_const2<T, const N: usize>(_: Bencher) {}
}
