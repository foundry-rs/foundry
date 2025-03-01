use criterion::{criterion_group, criterion_main, Criterion};

use comfy_table::presets::UTF8_FULL;
use comfy_table::*;
use rand::distr::Alphanumeric;
use rand::Rng;

/// Create a dynamic 10x500 Table with width 300 and unevenly distributed content.
/// There are no constraint, the content simply has to be formatted to fit as good as possible into
/// the given space.
fn build_huge_table() {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::DynamicFullWidth)
        .set_width(300)
        .set_header(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);

    let mut rng = rand::rng();
    for _ in 0..500 {
        let mut row = Vec::new();
        for _ in 0..10 {
            let string_length = rng.random_range(2..100);
            let random_string: String = (&mut rng)
                .sample_iter(&Alphanumeric)
                .take(string_length)
                .map(char::from)
                .collect();
            row.push(random_string);
        }
        table.add_row(row);
    }

    // Build the table.
    let _ = table.lines();
}

pub fn build_tables(crit: &mut Criterion) {
    crit.bench_function("Huge table", |b| b.iter(build_huge_table));
}

criterion_group!(benches, build_tables);
criterion_main!(benches);
