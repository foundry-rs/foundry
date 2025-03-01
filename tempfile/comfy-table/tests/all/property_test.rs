use ::proptest::prelude::*;
use comfy_table::ColumnConstraint::*;
use comfy_table::Width::*;
use comfy_table::*;

/// Pick any of the three existing ContentArrangement types for the table.
fn content_arrangement() -> impl Strategy<Value = ContentArrangement> {
    prop_oneof![
        Just(ContentArrangement::Disabled),
        Just(ContentArrangement::Dynamic),
        Just(ContentArrangement::DynamicFullWidth),
    ]
}

/// Each cell can have any alignment.
fn cell_alignment() -> impl Strategy<Value = Option<CellAlignment>> {
    prop_oneof![
        Just(None),
        Just(Some(CellAlignment::Left)),
        Just(Some(CellAlignment::Right)),
        Just(Some(CellAlignment::Center)),
    ]
}

/// Any Column can have any constellation of ColumnConstraints
fn column_constraint() -> impl Strategy<Value = Option<ColumnConstraint>> {
    prop_oneof![
        Just(None),
        Just(Some(ColumnConstraint::ContentWidth)),
        Just(Some(ColumnConstraint::Hidden)),
        any::<u16>().prop_map(|width| { Some(Absolute(Fixed(width))) }),
        any::<u16>().prop_map(|width| { Some(LowerBoundary(Fixed(width))) }),
        any::<u16>().prop_map(|width| { Some(UpperBoundary(Fixed(width))) }),
        (0u16..200u16).prop_map(|percentage| { Some(Absolute(Percentage(percentage))) }),
        (0u16..200u16).prop_map(|percentage| { Some(LowerBoundary(Percentage(percentage))) }),
        (0u16..200u16).prop_map(|percentage| { Some(UpperBoundary(Percentage(percentage))) }),
    ]
}

/// We test the Row::max_height with a few values.
fn max_height() -> impl Strategy<Value = Option<usize>> {
    prop_oneof![
        Just(None),
        Just(Some(0)),
        Just(Some(1)),
        Just(Some(5)),
        Just(Some(100))
    ]
}

prop_compose! {
    /// Returns the dimensions of the table, i.e. the amount of rows and columns.
    fn dimensions()(columns in 1u16..10u16, rows in 1u16..10u16)
                    -> (u16, u16) {
       (columns, rows)
   }
}

/// Returns all data needed to build the final table.
/// 1. A matrix of cells Row[Column[Cell]].
/// 2. Constraints for all columns.
/// 3. The alignment for each cell.
/// 3. The alignment for each column.
#[allow(clippy::type_complexity)]
fn columns_and_rows() -> impl Strategy<
    Value = (
        Vec<Vec<String>>,
        Vec<Option<ColumnConstraint>>,
        Vec<Option<CellAlignment>>,
        Vec<Option<CellAlignment>>,
    ),
> {
    dimensions().prop_flat_map(|(column_count, row_count)| {
        let mut rows = Vec::new();
        let mut cell_alignments = Vec::new();
        for _i in 0..row_count {
            // Create the max amount of possibly needed cell alignments
            for _j in 0..column_count {
                cell_alignments.push(cell_alignment());
            }
            // Add a strategy that creates random cell content with a length of 0 to column_count
            //
            // UTF-8 characters completely break table alignment in edge-case situations (e.g. 1 space columns).
            // UTF-8 characters can be multiple characters wide, which conflicts with the 1 space
            // column fallback, as well as fixed-width-, percental- and max-column-constraints.
            // As a result, we cannot check this with proptest, as this is inherently broken.
            rows.push(::proptest::collection::vec(
                "[A-Za-z_]*",
                0..column_count as usize,
            ));
        }
        let mut constraints = Vec::new();
        let mut column_alignments = Vec::new();
        for _i in 0..column_count {
            constraints.push(column_constraint());
            column_alignments.push(cell_alignment());
        }

        (rows, constraints, cell_alignments, column_alignments)
    })
}

/// We test the Row::max_height with a few values.
fn table_width() -> impl Strategy<Value = u16> {
    0..1000u16
}

prop_compose! {
    /// The ultimate test
    /// This creates a table from a combination of all "random" selectors above.
    fn table()
        (arrangement in content_arrangement(),
        max_height in max_height(),
        (rows, constraints, cell_alignments, column_alignments) in columns_and_rows()) -> Table {

        let mut table = Table::new();
        if let Some(height) = max_height {
            for row in table.row_iter_mut() {
                row.max_height(height);
            }
        }

        let mut cell_alignments = cell_alignments.iter();
        for row in rows.iter() {
            // Convert a vector of Strings to a vector of Cells and
            // set the content alignment for each cell
            let row: Vec<Cell> = row.iter().map(|content| {
                let mut cell = Cell::new(content.clone());
                if let Some(alignment) = cell_alignments.next().unwrap() {
                    cell = cell.set_alignment(*alignment);
                }
                cell
            }).collect();

            // Add the row to the table
            table.add_row(row);
        }

        for (column_index, column) in table.column_iter_mut().enumerate() {
            // Add the alignment for all columns
            let alignment = column_alignments.get(column_index).unwrap();
            if let Some(alignment) = alignment {
                column.set_cell_alignment(*alignment);
            }

            // Set constraints for all columns
            let constraint = constraints.get(column_index).unwrap();
            if let Some(constraint) = constraint {
                column.set_constraint(*constraint);
            }

        }

        table.set_content_arrangement(arrangement);
        table
    }
}

proptest! {
    #![proptest_config({
        let mut config = ProptestConfig::with_cases(512);
        config.max_shrink_iters = 5000;
        config
    })]
    #[test]
    fn random_tables(mut table in table(), table_width in table_width()) {
        table.set_width(table_width);

        // Make sure the table builds without any panics
        let formatted = table.to_string();

        // We'll take a look at each individual line to ensure they all share some properties.
        let lines: Vec<String> = formatted.split_terminator('\n').map(|line| line.to_owned()).collect();
        let mut line_iter = lines.iter();

        // ----- Table width check ------

        // Get the length of the very first line.
        // We're lateron going to ensure, that all lines have the same length.
        let line_length = if let Some(line) = line_iter.next() {
            line.trim().len()
        } else {
            0
        };

        // Make sure all lines have the same length
        for line in line_iter {
            if line.len() != line_length {
                return build_error(&formatted, "Each line of a printed table has to have the same length!");
            }
        }

        // Make sure that the table is within its width, if arrangement isn't enabled.
        // This is a bit tricky.
        // A table can be larger than the specified width, if the user forces it to be larger.
        #[cfg(feature = "integration_test")]
        {
            let current_arrangement = table.content_arrangement();
            match current_arrangement {
                ContentArrangement::Disabled => (),
                _ => {
                    let expected_max = determine_max_table_width(&table);

                    // A line can be a bit longer than u16::MAX due to formatting and borders.
                    let actual: u16 = line_length.try_into().unwrap_or(u16::MAX);
                    if actual > expected_max {
                        return build_error(
                            &formatted,
                            &format!("Expected table to be smaller than line length!\n\
                            Actual: {actual}, Expected max: {expected_max}\n\
                            Arrangement: {current_arrangement:?}"
                        ));
                    }
                }
            }
        }

        #[cfg(feature = "integration_test")]
        // Only run this test, if the `integration_test` is enabled.
        // Without this flag, we don't have access to some util functions in comfy_table, that
        // aren't exposed by default.
        enforce_constraints(&table, formatted, lines)?
    }
}

fn build_error(table: &str, context: &str) -> Result<(), TestCaseError> {
    Err(TestCaseError::Fail(
        format!("\n{context}:\n{table}\n").into(),
    ))
}

/// The user can actually force a table to be longer than the specified `table.width()`
/// by specifying [ColumnConstraint]s.
#[cfg(feature = "integration_test")]
fn determine_max_table_width(table: &Table) -> u16 {
    use comfy_table::utils::arrangement::helper::count_border_columns;
    let table_width = table.width().unwrap();

    // The max value that will be enforced by constraints.
    // We start with `2` for the side borders.
    let visible_columns = table
        .column_iter()
        .filter(|column| !column.is_hidden())
        .count();

    // Initialize the value for the min width enforced by constraints.
    // Borders may exist, but they are not included in constraints, which is why we have to
    // explicitly add them.
    let mut constraint_min_width: u16 = count_border_columns(table, visible_columns)
        .try_into()
        .unwrap_or(u16::MAX);

    // Get the max content widths for each column.
    // This is necessary for the `ContentWidth` constraint.
    let max_content_widths = table.column_max_content_widths();

    // Calculate the enforced widths by any constraints.
    for (index, column) in table.column_iter().enumerate() {
        if let Some(constraint) = column.constraint() {
            match constraint {
                ColumnConstraint::ContentWidth => {
                    constraint_min_width = constraint_min_width
                        .saturating_add(max_content_widths[index])
                        .saturating_add(column.padding_width());
                }
                ColumnConstraint::Absolute(width) => {
                    constraint_min_width = constraint_min_width
                        .saturating_add(absolute_width(table, width))
                        .saturating_add(column.padding_width());
                }
                ColumnConstraint::LowerBoundary(width)
                | ColumnConstraint::Boundaries { lower: width, .. } => {
                    constraint_min_width = constraint_min_width
                        .saturating_add(absolute_width(table, width))
                        .saturating_add(column.padding_width());
                }
                ColumnConstraint::Hidden => {}
                _ => {
                    // Add the padding and the min-width of `1` for this column
                    constraint_min_width = constraint_min_width
                        .saturating_add(column.padding_width())
                        .saturating_add(1);
                }
            }
        } else {
            // Add the padding + 1 space for all columns without constraints.
            constraint_min_width = constraint_min_width
                .saturating_add(column.padding_width())
                .saturating_add(1);
        }
    }

    std::cmp::max(table_width, constraint_min_width)
}

/// Enforce that Column constraints are enforced as expected in `Dynamic` mode.
#[cfg(feature = "integration_test")]
fn enforce_constraints(
    table: &Table,
    formatted: String,
    lines: Vec<String>,
) -> Result<(), TestCaseError> {
    let content_arrangement = table.content_arrangement();
    // Don't run the following for disabled or full-width arrangement.
    // These constraints kind of mess with all kinds of assertions we can make, which is why we
    // skip them.
    match content_arrangement {
        ContentArrangement::Dynamic => (),
        _ => return Ok(()),
    }

    // Extract the constraints for each table
    // Also remove hidden columns
    let constraints: Vec<Option<ColumnConstraint>> = table
        .column_iter()
        .map(|col| col.constraint().cloned())
        .filter(|constraint| !matches!(constraint, Some(ColumnConstraint::Hidden)))
        .collect();

    let line_iter = lines.iter();

    for line in line_iter {
        // Split the line along the column delimiter.
        // This allows us to ensure that each column is inside its constraints.
        let line_parts: Vec<String> = line.split('|').map(|col| col.to_string()).collect();

        // Skip the line if there're fewer vertical delimiters than columns + borders.
        // If that's the case, we're currently looking at a border or a delimiter line.
        if line_parts.len() < (constraints.len() + 2) {
            continue;
        }

        // The left and right borders will produce empty strings, let's filter those out.
        let line_parts: Vec<String> = line_parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect();

        for (index, (part, constraint)) in line_parts.iter().zip(constraints.iter()).enumerate() {
            let constraint = match constraint {
                Some(constraint) => constraint,
                // No constraint, we're good to go.
                None => continue,
            };
            // Get the actual length of the part.
            let actual = part.len();

            match constraint {
                ColumnConstraint::Hidden => panic!("This shouldn't happen"),
                // No need to check, if the column can be as wide as the content.
                ColumnConstraint::ContentWidth => continue,
                // Absolute width is defined.
                ColumnConstraint::Absolute(absolute) => {
                    let mut expected = absolute_width(table, absolute);
                    // The minimal amount of chars per column (with default padding)
                    // is 3 chars. 2 padding + 1 char content.
                    if expected < 3 {
                        expected = 3;
                    }
                    if actual != expected.into() {
                        return build_error(
                            &formatted,
                            &format!(
                                "Column {index} for should have absolute width of {expected}.\n\
                                Actual width is {actual}.\n\
                                {absolute:?} for line '{line}', part '{part}'"
                            ),
                        );
                    }
                }
                ColumnConstraint::LowerBoundary(lower) => {
                    let expected_lower = absolute_width(table, lower);
                    if actual < expected_lower.into() {
                        return build_error(
                            &formatted,
                            &format!(
                                "Column {index} has a lower bound of {expected_lower}.\n\
                                Actual width is {actual}.\n\
                                {lower:?} for line '{line}', part '{part}'"
                            ),
                        );
                    }
                }
                ColumnConstraint::UpperBoundary(upper) => {
                    let mut expected_upper = absolute_width(table, upper);
                    // The minimal amount of chars per column (with default padding)
                    // is 3 chars. 2 padding + 1 char content.
                    if expected_upper < 3 {
                        expected_upper = 3;
                    }

                    if actual > expected_upper.into() {
                        return build_error(
                            &formatted,
                            &format!(
                                "Column {index} has a upper bound of {expected_upper}.\n\
                                Actual width is {actual}.\n\
                                {upper:?} for line '{line}', part '{part}'"
                            ),
                        );
                    }
                }
                ColumnConstraint::Boundaries { lower, upper } => {
                    let expected_lower = absolute_width(table, lower);
                    let mut expected_upper = absolute_width(table, upper);
                    // The minimal amount of chars per column (with default padding)
                    // is 3 chars. 2 padding + 1 char content.
                    if expected_upper < 3 {
                        expected_upper = 3;
                    }

                    if actual < expected_lower.into() {
                        return build_error(
                            &formatted,
                            &format!(
                                "Column {index} has a lower bound of {expected_lower}.\n\
                                Actual width is {actual}.\n\
                                {lower:?} for line '{line}', part '{part}'"
                            ),
                        );
                    }

                    if actual > expected_upper.into() {
                        return build_error(
                            &formatted,
                            &format!(
                                "Column {index} has a upper bound of {expected_upper}.\n\
                                Actual width is {actual}.\n\
                                {upper:?} for line '{line}', part '{part}'"
                            ),
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

/// Resolve an absolute value from a given boundary
#[cfg(feature = "integration_test")]
pub fn absolute_width(table: &Table, width: &Width) -> u16 {
    use comfy_table::utils::arrangement::constraint::absolute_value_from_width;

    let visible_columns = table
        .column_iter()
        .filter(|column| !column.is_hidden())
        .count();

    let computed_width = absolute_value_from_width(table, width, visible_columns)
        .expect("Expected table to have a width");

    std::cmp::max(1, computed_width)
}
