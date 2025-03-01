use pretty_assertions::assert_eq;

use comfy_table::*;

#[test]
/// Columns can set a custom padding.
/// Ensure these settings are working.
fn custom_padding() {
    let mut table = Table::new();
    table
        .set_header(vec![
            Cell::new("Header1"),
            Cell::new("Header2"),
            Cell::new("Header3"),
        ])
        .add_row(vec!["One One", "One Two", "One Three"])
        .add_row(vec!["Two One", "Two Two", "Two Three"])
        .add_row(vec!["Three One", "Three Two", "Three Three"]);

    let column = table.column_mut(0).unwrap();
    column.set_padding((5, 5));
    let column = table.column_mut(2).unwrap();
    column.set_padding((0, 0));

    println!("{table}");
    let expected = "
+-------------------+-----------+-----------+
|     Header1       | Header2   |Header3    |
+===========================================+
|     One One       | One Two   |One Three  |
|-------------------+-----------+-----------|
|     Two One       | Two Two   |Two Three  |
|-------------------+-----------+-----------|
|     Three One     | Three Two |Three Three|
+-------------------+-----------+-----------+";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
