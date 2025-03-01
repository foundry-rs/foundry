use std::collections::BTreeMap;

use super::ColumnDisplayInfo;
use crate::style::ContentArrangement;
use crate::table::Table;

pub mod constraint;
mod disabled;
mod dynamic;
pub mod helper;

type DisplayInfos = BTreeMap<usize, ColumnDisplayInfo>;

/// Determine the width of each column depending on the content of the given table.
/// The results uses Option<usize>, since users can choose to hide columns.
pub fn arrange_content(table: &Table) -> Vec<ColumnDisplayInfo> {
    let table_width = table.width().map(usize::from);
    let mut infos = BTreeMap::new();

    let max_content_widths = table.column_max_content_widths();

    // Check if we can already resolve some constraints.
    // This step also populates the ColumnDisplayInfo structs.
    let visible_columns = helper::count_visible_columns(&table.columns);
    for column in table.columns.iter() {
        if column.constraint.is_some() {
            constraint::evaluate(
                table,
                visible_columns,
                &mut infos,
                column,
                max_content_widths[column.index],
            );
        }
    }
    #[cfg(feature = "debug")]
    println!("After initial constraints: {infos:#?}");

    // Fallback to `ContentArrangement::Disabled`, if we don't have any information
    // on how wide the table should be.
    let table_width = if let Some(table_width) = table_width {
        table_width
    } else {
        disabled::arrange(table, &mut infos, visible_columns, &max_content_widths);
        return infos.into_values().collect();
    };

    match &table.arrangement {
        ContentArrangement::Disabled => {
            disabled::arrange(table, &mut infos, visible_columns, &max_content_widths)
        }
        ContentArrangement::Dynamic | ContentArrangement::DynamicFullWidth => {
            dynamic::arrange(table, &mut infos, table_width, &max_content_widths);
        }
    }

    infos.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_arrangement() {
        let mut table = Table::new();
        table.set_header(vec!["head", "head", "head"]);
        table.add_row(vec!["__", "fivef", "sixsix"]);

        let display_infos = arrange_content(&table);

        // The width should be the width of the rows + padding
        let widths: Vec<u16> = display_infos.iter().map(ColumnDisplayInfo::width).collect();
        assert_eq!(widths, vec![6, 7, 8]);
    }

    #[test]
    fn test_discover_columns() {
        let mut table = Table::new();
        table.add_row(vec!["one", "two"]);

        // Get the first row and add a new cell, which would create a new column.
        let row = table.row_mut(0).unwrap();
        row.add_cell("three".into());

        // The table cannot know about the new cell yet, which is why we expect two columns.
        assert_eq!(table.columns.len(), 2);

        // After scanning for new columns however, it should show up.
        table.discover_columns();
        assert_eq!(table.columns.len(), 3);
    }
}
