use pretty_assertions::assert_eq;

use comfy_table::*;

#[test]
fn add_predicate_single_true() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec![
            "This is a text",
            "This is another text",
            "This is the third text",
        ])
        .add_row_if(
            |_, _| true,
            &vec![
                "This is another text",
                "Now\nadd some\nmulti line stuff",
                "This is awesome",
            ],
        );

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
fn add_predicate_single_false() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec![
            "This is a text",
            "This is another text",
            "This is the third text",
        ])
        .add_row_if(
            |_, _| false,
            &vec![
                "This is another text",
                "Now\nadd some\nmulti line stuff",
                "This is awesome",
            ],
        );

    println!("{table}");
    let expected = "
+----------------+----------------------+------------------------+
| Header1        | Header2              | Header3                |
+================================================================+
| This is a text | This is another text | This is the third text |
+----------------+----------------------+------------------------+";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn add_predicate_single_mixed() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec![
            "This is a text",
            "This is another text",
            "This is the third text",
        ])
        .add_row_if(
            |_, _| false,
            &vec!["I won't get displayed", "Me neither", "Same here!"],
        )
        .add_row_if(
            |_, _| true,
            &vec![
                "This is another text",
                "Now\nadd some\nmulti line stuff",
                "This is awesome",
            ],
        );

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
fn add_predicate_single_wrong_row_count() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec![
            "This is a text",
            "This is another text",
            "This is the third text",
        ])
        .add_row_if(
            |_, row| row.len() == 2,
            &vec![
                "This is another text",
                "Now\nadd some\nmulti line stuff",
                "This is awesome",
            ],
        );

    println!("{table}");
    let expected = "
+----------------+----------------------+------------------------+
| Header1        | Header2              | Header3                |
+================================================================+
| This is a text | This is another text | This is the third text |
+----------------+----------------------+------------------------+";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn add_predicate_multi_true() {
    let mut table = Table::new();
    let rows = vec![
        Row::from(&vec![
            "This is a text",
            "This is another text",
            "This is the third text",
        ]),
        Row::from(&vec![
            "This is another text",
            "Now\nadd some\nmulti line stuff",
            "This is awesome",
        ]),
    ];

    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_rows_if(|_, _| true, rows);

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
fn add_predicate_multi_false() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec![
            "This is a text",
            "This is another text",
            "This is the third text",
        ])
        .add_rows_if(
            |_, _| false,
            vec![Row::from(&vec![
                "This is another text",
                "Now\nadd some\nmulti line stuff",
                "This is awesome",
            ])],
        );

    println!("{table}");
    let expected = "
+----------------+----------------------+------------------------+
| Header1        | Header2              | Header3                |
+================================================================+
| This is a text | This is another text | This is the third text |
+----------------+----------------------+------------------------+";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn add_predicate_multi_mixed() {
    let mut table = Table::new();
    let rows = vec![
        Row::from(&vec![
            "This is a text",
            "This is another text",
            "This is the third text",
        ]),
        Row::from(&vec![
            "This is another text",
            "Now\nadd some\nmulti line stuff",
            "This is awesome",
        ]),
    ];

    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_rows_if(|_, _| true, rows)
        .add_rows_if(
            |_, _| false,
            vec![Row::from(&vec![
                "I won't get displayed",
                "Me neither",
                "Same here!",
            ])],
        );

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
fn add_predicate_multi_wrong_rows_count() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec![
            "This is a text",
            "This is another text",
            "This is the third text",
        ])
        .add_rows_if(
            |_, rows| rows.len() == 2,
            vec![Row::from(&vec![
                "This is another text",
                "Now\nadd some\nmulti line stuff",
                "This is awesome",
            ])],
        );

    println!("{table}");
    let expected = "
+----------------+----------------------+------------------------+
| Header1        | Header2              | Header3                |
+================================================================+
| This is a text | This is another text | This is the third text |
+----------------+----------------------+------------------------+";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
