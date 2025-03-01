use pretty_assertions::assert_eq;

use comfy_table::*;

#[test]
fn simple_table() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec![
            "This is a text",
            "This is another text",
            "This is the third text",
        ])
        .add_row(vec![
            "This is another text",
            "Now\nadd some\nmulti line stuff",
            "This is awesome",
        ]);

    println!("{table}");
    let expected = "
+----------------------+----------------------+------------------------+
| Header1              | Header2              | Header3                |
+======================================================================+
| This is a text       | This is another text | This is the third text |
|----------------------+----------------------+------------------------|
| This is another text | Now                  | This is awesome        |
|                      | add some             |                        |
|                      | multi line stuff     |                        |
+----------------------+----------------------+------------------------+";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn missing_column_table() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec!["One One", "One Two", "One Three"])
        .add_row(vec!["Two One", "Two Two"])
        .add_row(vec!["Three One"]);

    println!("{table}");
    let expected = "
+-----------+---------+-----------+
| Header1   | Header2 | Header3   |
+=================================+
| One One   | One Two | One Three |
|-----------+---------+-----------|
| Two One   | Two Two |           |
|-----------+---------+-----------|
| Three One |         |           |
+-----------+---------+-----------+";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn single_column_table() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1"])
        .add_row(vec!["One One"])
        .add_row(vec!["Two One"])
        .add_row(vec!["Three One"]);

    println!("{table}");
    let expected = "
+-----------+
| Header1   |
+===========+
| One One   |
|-----------|
| Two One   |
|-----------|
| Three One |
+-----------+";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn lines() {
    let mut t = Table::new();
    t.set_header(["heading 1", "heading 2", "heading 3"]);
    t.add_row(["test 1,1", "test 1,2", "test 1,3"]);
    t.add_row(["test 2,1", "test 2,2", "test 2,3"]);
    t.add_row(["test 3,1", "test 3,2", "test 3,3"]);

    let actual = t.lines();
    let expected = &[
        "+-----------+-----------+-----------+",
        "| heading 1 | heading 2 | heading 3 |",
        "+===================================+",
        "| test 1,1  | test 1,2  | test 1,3  |",
        "|-----------+-----------+-----------|",
        "| test 2,1  | test 2,2  | test 2,3  |",
        "|-----------+-----------+-----------|",
        "| test 3,1  | test 3,2  | test 3,3  |",
        "+-----------+-----------+-----------+",
    ];

    assert_eq!(actual.collect::<Vec<String>>(), expected);
}
