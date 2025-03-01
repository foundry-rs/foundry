use super::DisplayInfos;
use crate::utils::formatting::borders::{
    should_draw_left_border, should_draw_right_border, should_draw_vertical_lines,
};
use crate::{Cell, Column, Table};

/// The ColumnDisplayInfo works with a fixed value for content width.
/// However, if a column is supposed to get a absolute width, we have to make sure that
/// the padding on top of the content width doesn't get larger than the specified absolute width.
///
/// For this reason, we take the targeted width, subtract the column's padding and make sure that
/// the content width is always a minimum of 1
pub fn absolute_width_with_padding(column: &Column, width: u16) -> u16 {
    let mut content_width = width
        .saturating_sub(column.padding.0)
        .saturating_sub(column.padding.1);
    if content_width == 0 {
        content_width = 1;
    }

    content_width
}

/// Return the amount of visible columns
pub fn count_visible_columns(columns: &[Column]) -> usize {
    columns.iter().filter(|column| !column.is_hidden()).count()
}

/// Return the amount of visible columns that haven't been checked yet.
///
/// - `column_count` is the total amount of columns that are visible, calculated
///   with [count_visible_columns].
/// - `infos` are all columns that have already been fixed in size or are hidden.
pub fn count_remaining_columns(column_count: usize, infos: &DisplayInfos) -> usize {
    column_count - infos.iter().filter(|(_, info)| !info.is_hidden).count()
}

/// Return the amount of border columns, that will be visible in the final table output.
pub fn count_border_columns(table: &Table, visible_columns: usize) -> usize {
    let mut lines = 0;
    // Remove space occupied by borders from remaining_width
    if should_draw_left_border(table) {
        lines += 1;
    }
    if should_draw_right_border(table) {
        lines += 1;
    }
    if should_draw_vertical_lines(table) {
        lines += visible_columns.saturating_sub(1);
    }

    lines
}

/// Get the delimiter for a Cell.
/// Priority is in decreasing order: Cell -> Column -> Table.
pub fn delimiter(table: &Table, column: &Column, cell: &Cell) -> char {
    // Determine, which delimiter should be used
    if let Some(delimiter) = cell.delimiter {
        delimiter
    } else if let Some(delimiter) = column.delimiter {
        delimiter
    } else if let Some(delimiter) = table.delimiter {
        delimiter
    } else {
        ' '
    }
}
