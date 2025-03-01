use comfy_table::ColumnConstraint::*;
use comfy_table::Width::*;
use comfy_table::*;
use pretty_assertions::assert_eq;

use super::assert_table_line_width;

fn get_constraint_table() -> Table {
    let mut table = Table::new();
    table
        .set_header(vec!["smol", "Header2", "Header3"])
        .add_row(vec![
            "smol",
            "This is another text",
            "This is the third text",
        ])
        .add_row(vec![
            "smol",
            "Now\nadd some\nmulti line stuff",
            "This is awesome",
        ]);

    table
}

#[test]
/// Ensure max-, min- and fixed-width constraints are respected
fn fixed_max_min_constraints() {
    let mut table = get_constraint_table();

    table.set_constraints(vec![
        LowerBoundary(Fixed(10)),
        UpperBoundary(Fixed(8)),
        Absolute(Fixed(10)),
    ]);

    println!("{table}");
    let expected = "
+----------+--------+----------+
| smol     | Header | Header3  |
|          | 2      |          |
+==============================+
| smol     | This   | This is  |
|          | is ano | the      |
|          | ther   | third    |
|          | text   | text     |
|----------+--------+----------|
| smol     | Now    | This is  |
|          | add    | awesome  |
|          | some   |          |
|          | multi  |          |
|          | line   |          |
|          | stuff  |          |
+----------+--------+----------+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());

    // Now try this again when using dynamic content arrangement
    // The table tries to arrange to 28 characters,
    // but constraints enforce a width of at least 10+10+2+1+4 = 27
    // min_width + max_width + middle_padding + middle_min_width + borders
    // Since the left and right column are fixed, the middle column should only get a width of 2
    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(28);

    println!("{table}");
    let expected = "
+----------+----+----------+
| smol     | He | Header3  |
|          | ad |          |
|          | er |          |
|          | 2  |          |
+==========================+
| smol     | Th | This is  |
|          | is | the      |
|          | is | third    |
|          | an | text     |
|          | ot |          |
|          | he |          |
|          | r  |          |
|          | te |          |
|          | xt |          |
|----------+----+----------|
| smol     | No | This is  |
|          | w  | awesome  |
|          | ad |          |
|          | d  |          |
|          | so |          |
|          | me |          |
|          | mu |          |
|          | lt |          |
|          | i  |          |
|          | li |          |
|          | ne |          |
|          | st |          |
|          | uf |          |
|          | f  |          |
+----------+----+----------+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
/// Max and Min constraints won't be considered, if they are unnecessary
/// This is true for normal and dynamic arrangement tables.
fn unnecessary_max_min_constraints() {
    let mut table = get_constraint_table();

    table.set_constraints(vec![LowerBoundary(Fixed(1)), UpperBoundary(Fixed(30))]);

    println!("{table}");
    let expected = "
+------+----------------------+------------------------+
| smol | Header2              | Header3                |
+======================================================+
| smol | This is another text | This is the third text |
|------+----------------------+------------------------|
| smol | Now                  | This is awesome        |
|      | add some             |                        |
|      | multi line stuff     |                        |
+------+----------------------+------------------------+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());

    // Now test for dynamic content arrangement
    table.set_content_arrangement(ContentArrangement::Dynamic);
    println!("{table}");
    let expected = "
+------+----------------------+------------------------+
| smol | Header2              | Header3                |
+======================================================+
| smol | This is another text | This is the third text |
|------+----------------------+------------------------|
| smol | Now                  | This is awesome        |
|      | add some             |                        |
|      | multi line stuff     |                        |
+------+----------------------+------------------------+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
/// The user can specify constraints that result in bigger width than actually provided
/// This is allowed, but results in a wider table than actually aimed for.
/// Anyway we still try to fit everything as good as possible, which of course breaks stuff.
fn constraints_bigger_than_table_width() {
    let mut table = get_constraint_table();

    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(28)
        .set_constraints(vec![
            UpperBoundary(Fixed(50)),
            LowerBoundary(Fixed(30)),
            ContentWidth,
        ]);

    println!("{table}");
    let expected = "
+---+------------------------------+------------------------+
| s | Header2                      | Header3                |
| m |                              |                        |
| o |                              |                        |
| l |                              |                        |
+===========================================================+
| s | This is another text         | This is the third text |
| m |                              |                        |
| o |                              |                        |
| l |                              |                        |
|---+------------------------------+------------------------|
| s | Now                          | This is awesome        |
| m | add some                     |                        |
| o | multi line stuff             |                        |
| l |                              |                        |
+---+------------------------------+------------------------+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
/// Test correct usage of the Percentage constraint.
/// Percentage allows to set a fixed width.
fn percentage() {
    let mut table = get_constraint_table();

    // Set a percentage of 20% for the first column.
    // The the rest should arrange accordingly.
    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(40)
        .set_constraints(vec![Absolute(Percentage(20))]);

    println!("{table}");
    let expected = "
+-------+---------------+--------------+
| smol  | Header2       | Header3      |
+======================================+
| smol  | This is       | This is the  |
|       | another text  | third text   |
|-------+---------------+--------------|
| smol  | Now           | This is      |
|       | add some      | awesome      |
|       | multi line    |              |
|       | stuff         |              |
+-------+---------------+--------------+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
/// A single percentage constraint should be 100% at most.
fn max_100_percentage() {
    let mut table = Table::new();
    table
        .set_header(vec!["smol"])
        .add_row(vec!["smol"])
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(40)
        .set_constraints(vec![Absolute(Percentage(200))]);

    println!("{table}");
    let expected = "
+--------------------------------------+
| smol                                 |
+======================================+
| smol                                 |
+--------------------------------------+";
    println!("{expected}");
    assert_table_line_width(&table, 40);
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn percentage_second() {
    let mut table = get_constraint_table();

    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(40)
        .set_constraints(vec![
            LowerBoundary(Percentage(40)),
            UpperBoundary(Percentage(30)),
            Absolute(Percentage(30)),
        ]);

    println!("{table}");
    let expected = "
+--------------+----------+----------+
| smol         | Header2  | Header3  |
+====================================+
| smol         | This is  | This is  |
|              | another  | the      |
|              | text     | third    |
|              |          | text     |
|--------------+----------+----------|
| smol         | Now      | This is  |
|              | add some | awesome  |
|              | multi    |          |
|              | line     |          |
|              | stuff    |          |
+--------------+----------+----------+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn max_percentage() {
    let mut table = get_constraint_table();

    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(40)
        .set_constraints(vec![
            ContentWidth,
            UpperBoundary(Percentage(30)),
            Absolute(Percentage(30)),
        ]);

    println!("{table}");
    let expected = "
+------+----------+----------+
| smol | Header2  | Header3  |
+============================+
| smol | This is  | This is  |
|      | another  | the      |
|      | text     | third    |
|      |          | text     |
|------+----------+----------|
| smol | Now      | This is  |
|      | add some | awesome  |
|      | multi    |          |
|      | line     |          |
|      | stuff    |          |
+------+----------+----------+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
/// Ensure that both min and max in [Boundaries] is respected
fn min_max_boundary() {
    let mut table = get_constraint_table();

    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(40)
        .set_constraints(vec![
            Boundaries {
                lower: Percentage(50),
                upper: Fixed(2),
            },
            Boundaries {
                lower: Fixed(15),
                upper: Percentage(50),
            },
            Absolute(Percentage(30)),
        ]);

    println!("{table}");
    let expected = "
+------------------+---------------+----------+
| smol             | Header2       | Header3  |
+=============================================+
| smol             | This is       | This is  |
|                  | another text  | the      |
|                  |               | third    |
|                  |               | text     |
|------------------+---------------+----------|
| smol             | Now           | This is  |
|                  | add some      | awesome  |
|                  | multi line    |          |
|                  | stuff         |          |
+------------------+---------------+----------+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[rstest::rstest]
#[case(ContentArrangement::Dynamic)]
#[case(ContentArrangement::Disabled)]
/// Empty table with zero width constraint.
fn empty_table(#[case] arrangement: ContentArrangement) {
    let mut table = Table::new();
    table
        .add_row(vec![""])
        .set_content_arrangement(arrangement)
        .set_constraints(vec![Absolute(Fixed(0))]);

    println!("{table}");
    let expected = "
+---+
|   |
+---+";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
