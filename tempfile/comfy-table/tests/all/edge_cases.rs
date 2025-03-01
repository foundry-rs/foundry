use comfy_table::*;

/// A gigantic table can generated, even if it's longer than the longest supported width.
#[test]
fn giant_table() {
    let mut table = Table::new();
    table.set_header(["a".repeat(1_000_000)]);
    table.add_row(["a".repeat(1_000_000)]);

    table.to_string();
}

/// No panic, even if there's a ridiculous amount of padding.
#[test]
fn max_padding() {
    let mut table = Table::new();
    table.add_row(["test"]);
    let column = table.column_mut(0).unwrap();

    column.set_padding((u16::MAX, u16::MAX));

    table.to_string();
}
