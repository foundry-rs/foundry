use pretty_assertions::assert_eq;

use comfy_table::ColumnConstraint::*;
use comfy_table::Width::*;
use comfy_table::{ContentArrangement, Row, Table};

use crate::all::assert_table_line_width;

/// Individual rows can be configured to have a max height.
/// Everything beyond that line height should be truncated.
#[test]
fn table_with_truncate() {
    let mut table = Table::new();
    let mut first_row: Row = Row::from(vec![
        "This is a very long line with a lot of text",
        "This is anotherverylongtextwithlongwords text",
        "smol",
    ]);
    first_row.max_height(4);

    let mut second_row = Row::from(vec![
        "Now let's\nadd a really long line in the middle of the cell \n and add more multi line stuff",
        "This is another text",
        "smol",
    ]);
    second_row.max_height(4);

    table
        .set_header(vec!["Header1", "Header2", "Head"])
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(35)
        .add_row(first_row)
        .add_row(second_row);

    // The first column will be wider than 6 chars.
    // The second column's content is wider than 6 chars. There should be a '...'.
    let second_column = table.column_mut(1).unwrap();
    second_column.set_constraint(Absolute(Fixed(8)));

    // The third column's content is less than 6 chars width. There shouldn't be a '...'.
    let third_column = table.column_mut(2).unwrap();
    third_column.set_constraint(Absolute(Fixed(7)));

    println!("{table}");
    let expected = "
+----------------+--------+-------+
| Header1        | Header | Head  |
|                | 2      |       |
+=================================+
| This is a very | This   | smol  |
| long line with | is ano |       |
| a lot of text  | therve |       |
|                | ryl... |       |
|----------------+--------+-------|
| Now let's      | This   | smol  |
| add a really   | is ano |       |
| long line in   | ther   |       |
| the middle ... | text   |       |
+----------------+--------+-------+";
    println!("{expected}");
    assert_table_line_width(&table, 35);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn table_with_truncate_indicator() {
    let mut table = Table::new();
    let mut first_row: Row = Row::from(vec![
        "This is a very long line with a lot of text",
        "This is anotherverylongtextwithlongwords text",
        "smol",
    ]);
    first_row.max_height(4);

    let mut second_row = Row::from(vec![
        "Now let's\nadd a really long line in the middle of the cell \n and add more multi line stuff",
        "This is another text",
        "smol",
    ]);
    second_row.max_height(4);

    table
        .set_header(vec!["Header1", "Header2", "Head"])
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_truncation_indicator("…")
        .set_width(35)
        .add_row(first_row)
        .add_row(second_row);

    // The first column will be wider than 6 chars.
    // The second column's content is wider than 6 chars. There should be a '…'.
    let second_column = table.column_mut(1).unwrap();
    second_column.set_constraint(Absolute(Fixed(8)));

    // The third column's content is less than 6 chars width. There shouldn't be a '…'.
    let third_column = table.column_mut(2).unwrap();
    third_column.set_constraint(Absolute(Fixed(7)));

    println!("{table}");
    let expected = "
+----------------+--------+-------+
| Header1        | Header | Head  |
|                | 2      |       |
+=================================+
| This is a very | This   | smol  |
| long line with | is ano |       |
| a lot of text  | therve |       |
|                | rylon… |       |
|----------------+--------+-------|
| Now let's      | This   | smol  |
| add a really   | is ano |       |
| long line in   | ther   |       |
| the middle of… | text   |       |
+----------------+--------+-------+";
    println!("{expected}");
    assert_table_line_width(&table, 35);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn table_with_composite_utf8_strings() {
    let mut table = Table::new();

    table
        .set_header(vec!["Header1"])
        .set_width(20)
        .add_row(vec!["あいうえおかきくけこさしすせそたちつてと"])
        .set_content_arrangement(comfy_table::ContentArrangement::Dynamic);

    for row in table.row_iter_mut() {
        row.max_height(1); // 2 -> also panics, 3 -> ok
    }

    println!("{table}");
    let expected = "
+------------------+
| Header1          |
+==================+
| あいうえおか...  |
+------------------+";
    println!("{expected}");
    assert_table_line_width(&table, 20);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn table_with_composite_utf8_strings_2_lines() {
    let mut table = Table::new();

    table
        .set_header(vec!["Header1"])
        .set_width(20)
        .add_row(vec!["あいうえおかきくけこさしすせそたちつてと"])
        .set_content_arrangement(comfy_table::ContentArrangement::Dynamic);

    for row in table.row_iter_mut() {
        row.max_height(2);
    }

    println!("{table}");
    let expected = "
+------------------+
| Header1          |
+==================+
| あいうえおかきく |
| けこさしすせ...  |
+------------------+";
    println!("{expected}");
    assert_table_line_width(&table, 20);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn table_with_composite_utf8_emojis() {
    let mut table = Table::new();

    table
        .set_header(vec!["Header1"])
        .set_width(15)
        .add_row(vec![
            "🙂‍↕️🙂‍↕️🙂‍↕️🙂‍↕️🙂‍↕️🙂‍↕️last_line.into_bytes().truncate(truncate_at)",
        ])
        .set_content_arrangement(comfy_table::ContentArrangement::Dynamic);

    for row in table.row_iter_mut() {
        row.max_height(1); // 2 -> also panics, 3 -> ok
    }

    println!("{table}");
    let expected = "
+-------------+
| Header1     |
+=============+
| 🙂‍↕️🙂‍↕️🙂‍↕️🙂‍↕️... |
+-------------+";
    println!("{expected}");
    assert_table_line_width(&table, 15);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
