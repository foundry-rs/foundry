use pretty_assertions::assert_eq;

use comfy_table::*;

#[test]
fn test_col_count_header() {
    let mut table = Table::new();

    table.set_header(vec!["Col 1", "Col 2", "Col 3"]);
    assert_eq!(table.column_count(), 3);

    table.set_header(vec!["Col 1", "Col 2", "Col 3", "Col 4"]);
    assert_eq!(table.column_count(), 4);

    table.set_header(vec!["Col I", "Col II"]);
    assert_eq!(table.column_count(), 4);
}

#[test]
fn test_col_count_row() {
    let mut table = Table::new();

    table.add_row(vec!["Foo", "Bar"]);
    assert_eq!(table.column_count(), 2);

    table.add_row(vec!["Bar", "Foo", "Baz"]);
    assert_eq!(table.column_count(), 3);
}

#[test]
fn test_row_count() {
    let mut table = Table::new();
    assert_eq!(table.row_count(), 0);

    table.add_row(vec!["Foo", "Bar"]);
    assert_eq!(table.row_count(), 1);

    table.add_row(vec!["Bar", "Foo", "Baz"]);
    assert_eq!(table.row_count(), 2);

    table.add_row_if(|_, _| false, vec!["Baz", "Bar", "Foo"]);
    assert_eq!(table.row_count(), 2);

    table.add_row_if(|_, _| true, vec!["Foo", "Baz", "Bar"]);
    assert_eq!(table.row_count(), 3);
}

#[test]
fn test_is_empty() {
    let mut table = Table::new();
    assert_eq!(table.is_empty(), true);

    table.add_row(vec!["Foo", "Bar"]);
    assert_eq!(table.is_empty(), false);
}
