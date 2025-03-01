use comfy_table::*;
use pretty_assertions::assert_eq;

fn get_table() -> Table {
    let mut table = Table::new();
    table
        .load_preset(presets::UTF8_FULL)
        .set_header(vec![
            "hidden_header",
            "smol",
            "hidden_header",
            "Two_hidden_headers_in_a_row",
            "Header2",
            "Header3",
            "hidden_header",
        ])
        .add_row(vec![
            "start_hidden",
            "smol",
            "middle_hidden",
            "two_hidden_headers_in_a_row",
            "This is another text",
            "This is the third text",
            "end_hidden",
        ])
        .add_row(vec![
            "asdf",
            "smol",
            "asdf",
            "asdf",
            "Now\nadd some\nmulti line stuff",
            "This is awesome",
            "asdf",
        ]);

    // Hide the first, third and 6th column
    table
        .column_mut(0)
        .unwrap()
        .set_constraint(ColumnConstraint::Hidden);
    table
        .column_mut(2)
        .unwrap()
        .set_constraint(ColumnConstraint::Hidden);
    table
        .column_mut(3)
        .unwrap()
        .set_constraint(ColumnConstraint::Hidden);

    table
        .column_mut(6)
        .unwrap()
        .set_constraint(ColumnConstraint::Hidden);

    table
}

/// Make sure hidden columns won't be displayed
#[test]
fn hidden_columns() {
    let table = get_table();
    println!("{table}");
    let expected = "
┌──────┬──────────────────────┬────────────────────────┐
│ smol ┆ Header2              ┆ Header3                │
╞══════╪══════════════════════╪════════════════════════╡
│ smol ┆ This is another text ┆ This is the third text │
├╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ smol ┆ Now                  ┆ This is awesome        │
│      ┆ add some             ┆                        │
│      ┆ multi line stuff     ┆                        │
└──────┴──────────────────────┴────────────────────────┘";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

/// Make sure dynamic adjustment still works with hidden columns
#[test]
fn hidden_columns_with_dynamic_adjustment() {
    let mut table = get_table();
    table.set_width(25);
    table.set_content_arrangement(ContentArrangement::Dynamic);

    println!("{table}");
    let expected = "
┌──────┬────────┬───────┐
│ smol ┆ Header ┆ Heade │
│      ┆ 2      ┆ r3    │
╞══════╪════════╪═══════╡
│ smol ┆ This   ┆ This  │
│      ┆ is ano ┆ is    │
│      ┆ ther   ┆ the   │
│      ┆ text   ┆ third │
│      ┆        ┆ text  │
├╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌┤
│ smol ┆ Now    ┆ This  │
│      ┆ add    ┆ is    │
│      ┆ some   ┆ aweso │
│      ┆ multi  ┆ me    │
│      ┆ line   ┆       │
│      ┆ stuff  ┆       │
└──────┴────────┴───────┘";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

/// Nothing breaks, if all columns are hidden
#[test]
fn only_hidden_columns() {
    let mut table = get_table();
    table.set_constraints(vec![
        ColumnConstraint::Hidden,
        ColumnConstraint::Hidden,
        ColumnConstraint::Hidden,
        ColumnConstraint::Hidden,
        ColumnConstraint::Hidden,
        ColumnConstraint::Hidden,
    ]);

    println!("{table}");
    let expected = "
┌┐
╞╡
├┤
└┘";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
