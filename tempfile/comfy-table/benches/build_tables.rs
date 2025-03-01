use criterion::{criterion_group, criterion_main, Criterion};

use comfy_table::presets::UTF8_FULL;
use comfy_table::ColumnConstraint::*;
use comfy_table::Width::*;
use comfy_table::*;

/// Build the readme table
#[cfg(feature = "tty")]
fn build_readme_table() {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(80)
        .set_header(vec![
            Cell::new("Header1").add_attribute(Attribute::Bold),
            Cell::new("Header2").fg(Color::Green),
            Cell::new("Header3"),
        ])
        .add_row(vec![
            Cell::new("This is a bold text").add_attribute(Attribute::Bold),
            Cell::new("This is a green text").fg(Color::Green),
            Cell::new("This one has black background").bg(Color::Black),
        ])
        .add_row(vec![
            Cell::new("Blinky boi").add_attribute(Attribute::SlowBlink),
            Cell::new("This table's content is dynamically arranged. The table is exactly 80 characters wide.\nHere comes a reallylongwordthatshoulddynamicallywrap"),
            Cell::new("COMBINE ALL THE THINGS")
            .fg(Color::Green)
            .bg(Color::Black)
            .add_attributes(vec![
                Attribute::Bold,
                Attribute::SlowBlink,
            ])
        ]);

    // Build the table.
    let _ = table.lines();
}

#[cfg(not(feature = "tty"))]
fn build_readme_table() {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(80)
        .set_header(vec![
            Cell::new("Header1"),
            Cell::new("Header2"),
            Cell::new("Header3"),
        ])
        .add_row(vec![
            Cell::new("This is a bold text"),
            Cell::new("This is a green text"),
            Cell::new("This one has black background"),
        ])
        .add_row(vec![
            Cell::new("Blinky boi"),
            Cell::new("This table's content is dynamically arranged. The table is exactly 80 characters wide.\nHere comes a reallylongwordthatshoulddynamicallywrap"),
            Cell::new("COMBINE ALL THE THINGS"),
        ]);

    // Build the table.
    let _ = table.lines();
}

/// Create a dynamic 10x10 Table with width 400 and unevenly distributed content.
/// On top of that, most of the columns have some kind of constraint.
fn build_big_table() {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::DynamicFullWidth)
        .set_width(400)
        .set_header(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);

    // Create a 10x10 grid
    for row_index in 0..10 {
        let mut row = Vec::new();
        for column in 0..10 {
            row.push("SomeWord ".repeat((column + row_index * 2) % 10));
        }
        table.add_row(row);
    }

    table.set_constraints(vec![
        UpperBoundary(Fixed(20)),
        LowerBoundary(Fixed(40)),
        Absolute(Fixed(5)),
        Absolute(Percentage(3)),
        Absolute(Percentage(3)),
        Boundaries {
            lower: Fixed(30),
            upper: Percentage(10),
        },
    ]);

    // Build the table.
    let _ = table.lines();
}

pub fn build_tables(crit: &mut Criterion) {
    crit.bench_function("Readme table", |b| b.iter(build_readme_table));

    crit.bench_function("Big table", |b| b.iter(build_big_table));
}

criterion_group!(benches, build_tables);
criterion_main!(benches);
