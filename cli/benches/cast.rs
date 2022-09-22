use criterion::{black_box, criterion_group, criterion_main, Criterion};
use foundry_cli::cmd::cast::wallet::vanity::*;
use rayon::prelude::*;
use std::time::Duration;

/// Benches `cast wallet vanity`
///
/// Left or right matchers, with or without nonce do not change the outcome.
///
/// Regex matchers get optimised away even with a black_box.
fn vanity(c: &mut Criterion) {
    let mut g = c.benchmark_group("vanity");

    g.sample_size(500);
    g.noise_threshold(0.04);
    g.measurement_time(Duration::from_secs(30));

    g.bench_function("wallet generator", |b| b.iter(|| black_box(generate_wallet())));

    // 1

    g.sample_size(150);
    g.noise_threshold(0.02);

    g.bench_function("match 1", |b| {
        let m = LeftHexMatcher { left: v(0, 1) };
        let matcher = create_matcher(m);
        b.iter(|| wallet_generator().find_any(|x| black_box(matcher(x))))
    });

    // 2

    g.sample_size(10);
    g.noise_threshold(0.01);
    g.measurement_time(Duration::from_secs(60));

    g.bench_function("match 2", |b| {
        let m = LeftHexMatcher { left: v(0, 2) };
        let matcher = create_matcher(m);
        b.iter(|| wallet_generator().find_any(|x| black_box(matcher(x))))
    });

    g.finish();
}

fn v(byte: u8, times: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(times);
    for _ in 0..times {
        v.push(byte)
    }
    v
}

// fn r(byte: u8, times: usize) -> Regex {
//     Regex::new(format!("{}{{{}}}", byte, times).as_str()).unwrap()
// }

criterion_group!(vanity_benches, vanity);
criterion_main!(vanity_benches);
