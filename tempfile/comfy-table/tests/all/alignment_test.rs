use pretty_assertions::assert_eq;

use comfy_table::*;

#[test]
/// Cell alignment can be specified on Columns and Cells
/// Alignment settings on Cells overwrite the settings of Columns
fn cell_alignment() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec![
            "Very long line Test",
            "Very long line Test",
            "Very long line Test",
        ])
        .add_row(vec![
            Cell::new("Right").set_alignment(CellAlignment::Right),
            Cell::new("Left").set_alignment(CellAlignment::Left),
            Cell::new("Center").set_alignment(CellAlignment::Center),
        ])
        .add_row(vec!["Left", "Center", "Right"]);

    let alignment = [
        CellAlignment::Left,
        CellAlignment::Center,
        CellAlignment::Right,
    ];

    // Add the alignment to their respective column
    for (column_index, column) in table.column_iter_mut().enumerate() {
        let alignment = alignment.get(column_index).unwrap();
        column.set_cell_alignment(*alignment);
    }

    println!("{table}");
    let expected = "
+---------------------+---------------------+---------------------+
| Header1             |       Header2       |             Header3 |
+=================================================================+
| Very long line Test | Very long line Test | Very long line Test |
|---------------------+---------------------+---------------------|
|               Right | Left                |        Center       |
|---------------------+---------------------+---------------------|
| Left                |        Center       |               Right |
+---------------------+---------------------+---------------------+";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
