use rayon::prelude::*;
use std::hint::black_box;

#[path = "../bin/cmd/wallet/mod.rs"]
#[allow(unused)]
mod wallet;
use wallet::vanity::*;

#[divan::bench]
fn vanity_wallet_generator() -> GeneratedWallet {
    generate_wallet()
}

#[divan::bench(args = [&[0][..]])]
fn vanity_match(bencher: divan::Bencher<'_, '_>, arg: &[u8]) {
    let matcher = create_matcher(LeftHexMatcher { left: arg.to_vec() });
    bencher.bench_local(|| wallet_generator().find_any(|x| black_box(matcher(x))));
}

fn main() {
    divan::main();
}
