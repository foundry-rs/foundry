use pretty_assertions::assert_eq;

use comfy_table::*;

#[test]
/// Create a table with a custom delimiter on Table, Column and Cell level.
/// The first column should be split with the table's delimiter.
/// The first cell of the second column should be split with the custom column delimiter
/// The second cell of the second column should be split with the custom cell delimiter
fn full_custom_delimiters() {
    let mut table = Table::new();

    table
        .set_header(vec!["Header1", "Header2"])
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_delimiter('-')
        .set_width(40)
        .add_row(vec![
            "This shouldn't be split with any logic, since there's no matching delimiter",
            "Test-Test-Test-Test-Test-This_should_only_be_splitted_by_underscore_and not by space or hyphens",
        ]);

    // Give the bottom right cell a special delimiter
    table.add_row(vec![
        Cell::new("Test_Test_Test_Test_Test_This-should-only-be-splitted-by-hyphens-not by space or underscore",),
        Cell::new(
            "Test-Test-Test-Test-Test-Test_Test_Test_Test_Test_Test_Test_This/should/only/be/splitted/by/backspace/and not by space or hyphens or anything else.",
        )
        .set_delimiter('/'),
    ]);

    let column = table.column_mut(1).unwrap();
    column.set_delimiter('_');

    println!("{table}");
    let expected = "
+-------------------+------------------+
| Header1           | Header2          |
+======================================+
| This shouldn't be | Test-Test-Test-T |
|  split with any l | est-Test-This    |
| ogic, since there | should_only_be   |
| 's no matching de | splitted_by      |
| limiter           | underscore_and n |
|                   | ot by space or h |
|                   | yphens           |
|-------------------+------------------|
| Test_Test_Test_Te | Test-Test-Test-T |
| st_Test_This      | est-Test-Test_Te |
| should-only-be    | st_Test_Test_Tes |
| splitted-by       | t_Test_Test_This |
| hyphens-not by sp | should/only/be   |
| ace or underscore | splitted/by      |
|                   | backspace/and no |
|                   | t by space or hy |
|                   | phens or anythin |
|                   | g else.          |
+-------------------+------------------+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
