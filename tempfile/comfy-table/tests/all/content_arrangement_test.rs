use comfy_table::ColumnConstraint;
use comfy_table::Width;
use pretty_assertions::assert_eq;

use comfy_table::{ContentArrangement, Table};

use super::assert_table_line_width;

/// Test the robustness of the dynamic table arrangement.
#[test]
fn simple_dynamic_table() {
    let mut table = Table::new();
    table.set_header(vec!["Header1", "Header2", "Head"])
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(25)
        .add_row(vec![
            "This is a very long line with a lot of text",
            "This is anotherverylongtextwithlongwords text",
            "smol",
        ])
        .add_row(vec![
            "This is another text",
            "Now let's\nadd a really long line in the middle of the cell \n and add more multi line stuff",
            "smol",
        ]);

    println!("{table}");
    let expected = "
+--------+-------+------+
| Header | Heade | Head |
| 1      | r2    |      |
+=======================+
| This   | This  | smol |
| is a   | is    |      |
| very   | anoth |      |
| long   | erver |      |
| line   | ylong |      |
| with a | textw |      |
| lot of | ithlo |      |
| text   | ngwor |      |
|        | ds    |      |
|        | text  |      |
|--------+-------+------|
| This   | Now   | smol |
| is ano | let's |      |
| ther   | add a |      |
| text   | reall |      |
|        | y     |      |
|        | long  |      |
|        | line  |      |
|        | in    |      |
|        | the   |      |
|        | middl |      |
|        | e of  |      |
|        | the   |      |
|        | cell  |      |
|        | and   |      |
|        | add   |      |
|        | more  |      |
|        | multi |      |
|        | line  |      |
|        | stuff |      |
+--------+-------+------+";
    println!("{expected}");
    assert_table_line_width(&table, 25);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

/// This table checks the scenario, where a column has a big max_width, but a lot of the assigned
/// space doesn't get used after splitting the lines. This happens mostly when there are
/// many long words in a single column.
/// The remaining space should rather be distributed to other cells.
#[test]
fn distribute_space_after_split() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Head"])
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(80)
        .add_row(vec![
            "This is a very long line with a lot of text",
            "This is text with a anotherverylongtexttesttest",
            "smol",
        ]);

    println!("{table}");
    let expected = "
+-----------------------------------------+-----------------------------+------+
| Header1                                 | Header2                     | Head |
+==============================================================================+
| This is a very long line with a lot of  | This is text with a         | smol |
| text                                    | anotherverylongtexttesttest |      |
+-----------------------------------------+-----------------------------+------+";
    println!("{expected}");

    assert_table_line_width(&table, 80);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

/// A single column get's split and a lot of the available isn't used afterward.
/// The remaining space should be cut away, making the table more compact.
#[test]
fn unused_space_after_split() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1"])
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(30)
        .add_row(vec!["This is text with a anotherverylongtext"]);

    println!("{table}");
    let expected = "
+---------------------+
| Header1             |
+=====================+
| This is text with a |
| anotherverylongtext |
+---------------------+";
    println!("{expected}");
    assert_table_line_width(&table, 23);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

/// The full width of a table should be used, even if the space isn't used.
#[test]
fn dynamic_full_width_after_split() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1"])
        .set_content_arrangement(ContentArrangement::DynamicFullWidth)
        .set_width(50)
        .add_row(vec!["This is text with a anotherverylongtexttesttestaa"]);

    println!("{table}");
    let expected = "
+------------------------------------------------+
| Header1                                        |
+================================================+
| This is text with a                            |
| anotherverylongtexttesttestaa                  |
+------------------------------------------------+";
    println!("{expected}");
    assert_table_line_width(&table, 50);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

/// This table checks the scenario, where a column has a big max_width, but a lot of the assigned
/// space isn't used after splitting the lines.
/// The remaining space should rather distributed between all cells.
#[test]
fn dynamic_full_width() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "smol"])
        .set_content_arrangement(ContentArrangement::DynamicFullWidth)
        .set_width(80)
        .add_row(vec!["This is a short line", "small", "smol"]);

    println!("{table}");
    let expected = "
+-----------------------------------+----------------------+-------------------+
| Header1                           | Header2              | smol              |
+==============================================================================+
| This is a short line              | small                | smol              |
+-----------------------------------+----------------------+-------------------+";
    println!("{expected}");
    assert_table_line_width(&table, 80);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

/// Test that a table is displayed in its full width, if the `table.width` is set to the exact
/// width the table has, if it's fully expanded.
///
/// The same should be the case for values that are larger than this width.
#[test]
fn dynamic_exact_width() {
    let header = vec!["a\n---\ni64", "b\n---\ni64", "b_squared\n---\nf64"];
    let rows = vec![
        vec!["1", "2", "4.0"],
        vec!["3", "4", "16.0"],
        vec!["5", "6", "36.0"],
    ];

    for width in 25..40 {
        let mut table = Table::new();
        let table = table
            .load_preset(comfy_table::presets::UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_width(width);

        table.set_header(header.clone()).add_rows(rows.clone());

        println!("{table}");
        let expected = "
┌─────┬─────┬───────────┐
│ a   ┆ b   ┆ b_squared │
│ --- ┆ --- ┆ ---       │
│ i64 ┆ i64 ┆ f64       │
╞═════╪═════╪═══════════╡
│ 1   ┆ 2   ┆ 4.0       │
├╌╌╌╌╌┼╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┤
│ 3   ┆ 4   ┆ 16.0      │
├╌╌╌╌╌┼╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┤
│ 5   ┆ 6   ┆ 36.0      │
└─────┴─────┴───────────┘";
        println!("{expected}");
        assert_table_line_width(table, 25);
        assert_eq!(expected, "\n".to_string() + &table.to_string());
    }
}

/// Test that the formatting works as expected, if the table is slightly smaller than the max width
/// of the table.
#[test]
fn dynamic_slightly_smaller() {
    let header = vec!["a\n---\ni64", "b\n---\ni64", "b_squared\n---\nf64"];
    let rows = vec![
        vec!["1", "2", "4.0"],
        vec!["3", "4", "16.0"],
        vec!["5", "6", "36.0"],
    ];

    let mut table = Table::new();
    let table = table
        .load_preset(comfy_table::presets::UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(24);

    table.set_header(header.clone()).add_rows(rows.clone());

    println!("{table}");
    let expected = "
┌─────┬─────┬──────────┐
│ a   ┆ b   ┆ b_square │
│ --- ┆ --- ┆ d        │
│ i64 ┆ i64 ┆ ---      │
│     ┆     ┆ f64      │
╞═════╪═════╪══════════╡
│ 1   ┆ 2   ┆ 4.0      │
├╌╌╌╌╌┼╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┤
│ 3   ┆ 4   ┆ 16.0     │
├╌╌╌╌╌┼╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┤
│ 5   ┆ 6   ┆ 36.0     │
└─────┴─────┴──────────┘";
    println!("{expected}");
    assert_table_line_width(table, 24);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

/// This failed on a python integration test case in the polars project.
/// This a regression test.
#[test]
fn polar_python_test_tbl_width_chars() {
    let header = vec![
        "a really long col\n---\ni64",
        "b\n---\nstr",
        "this is 10\n---\ni64",
    ];
    let rows = vec![
        vec!["1", "", "4"],
        vec!["2", "this is a string value that will...", "5"],
        vec!["3", "null", "6"],
    ];

    let mut table = Table::new();
    let table = table
        .load_preset(comfy_table::presets::UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(100)
        .set_header(header)
        .add_rows(rows)
        .set_constraints(vec![
            ColumnConstraint::LowerBoundary(Width::Fixed(12)),
            ColumnConstraint::LowerBoundary(Width::Fixed(5)),
            ColumnConstraint::LowerBoundary(Width::Fixed(10)),
        ]);

    println!("{table}");
    let expected = "
┌───────────────────┬─────────────────────────────────────┬────────────┐
│ a really long col ┆ b                                   ┆ this is 10 │
│ ---               ┆ ---                                 ┆ ---        │
│ i64               ┆ str                                 ┆ i64        │
╞═══════════════════╪═════════════════════════════════════╪════════════╡
│ 1                 ┆                                     ┆ 4          │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ 2                 ┆ this is a string value that will... ┆ 5          │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ 3                 ┆ null                                ┆ 6          │
└───────────────────┴─────────────────────────────────────┴────────────┘";
    println!("{expected}");
    assert_table_line_width(table, 72);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
