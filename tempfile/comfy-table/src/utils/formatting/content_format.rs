#[cfg(feature = "tty")]
use crossterm::style::{style, Stylize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::content_split::measure_text_width;
use super::content_split::split_line;

use crate::cell::Cell;
use crate::row::Row;
use crate::style::CellAlignment;
#[cfg(feature = "tty")]
use crate::style::{map_attribute, map_color};
use crate::table::Table;
use crate::utils::ColumnDisplayInfo;

pub fn delimiter(cell: &Cell, info: &ColumnDisplayInfo, table: &Table) -> char {
    // Determine, which delimiter should be used
    if let Some(delimiter) = cell.delimiter {
        delimiter
    } else if let Some(delimiter) = info.delimiter {
        delimiter
    } else if let Some(delimiter) = table.delimiter {
        delimiter
    } else {
        ' '
    }
}

/// Returns the formatted content of the table.
/// The content is organized in the following structure
///
/// tc stands for table content and represents the returned value
/// ``` text
///      column1          column2
/// row1 tc[0][0][0]      tc[0][0][1] <-line1
///      tc[0][1][0]      tc[0][1][1] <-line2
///      tc[0][2][0]      tc[0][2][1] <-line3
///
/// row2 tc[1][0][0]      tc[1][0][1] <-line1
///      tc[1][1][0]      tc[1][1][1] <-line2
///      tc[1][2][0]      tc[1][2][1] <-line3
/// ```
///
/// The strings for each row will be padded and aligned according to their respective column.
pub fn format_content(table: &Table, display_info: &[ColumnDisplayInfo]) -> Vec<Vec<Vec<String>>> {
    // The content of the whole table
    let mut table_content = Vec::with_capacity(table.rows.len() + 1);

    // Format table header if it exists
    if let Some(header) = table.header() {
        table_content.push(format_row(header, display_info, table));
    }

    for row in table.rows.iter() {
        table_content.push(format_row(row, display_info, table));
    }
    table_content
}

pub fn format_row(
    row: &Row,
    display_infos: &[ColumnDisplayInfo],
    table: &Table,
) -> Vec<Vec<String>> {
    // The content of this specific row
    let mut temp_row_content = Vec::with_capacity(display_infos.len());

    let mut cell_iter = row.cells.iter();
    // Now iterate over all cells and handle them according to their alignment
    for info in display_infos.iter() {
        if info.is_hidden {
            cell_iter.next();
            continue;
        }
        // Each cell is divided into several lines divided by newline
        // Every line that's too long will be split into multiple lines
        let mut cell_lines = Vec::new();

        // Check if the row has as many cells as the table has columns.
        // If that's not the case, create a new cell with empty spaces.
        let cell = if let Some(cell) = cell_iter.next() {
            cell
        } else {
            cell_lines.push(" ".repeat(info.width().into()));
            temp_row_content.push(cell_lines);
            continue;
        };

        // The delimiter is configurable, determine which one should be used for this cell.
        let delimiter = delimiter(cell, info, table);

        // Iterate over each line and split it into multiple lines if necessary.
        // Newlines added by the user will be preserved.
        for line in cell.content.iter() {
            if measure_text_width(line) > info.content_width.into() {
                let mut parts = split_line(line, info, delimiter);
                cell_lines.append(&mut parts);
            } else {
                cell_lines.push(line.into());
            }
        }

        // Remove all unneeded lines of this cell, if the row's height is capped to a certain
        // amount of lines and there're too many lines in this cell.
        // This then truncates and inserts a '...' string at the end of the last line to indicate
        // that the cell has been truncated.
        if let Some(lines) = row.max_height {
            if cell_lines.len() > lines {
                // We already have to many lines. Cut off the surplus lines.
                let _ = cell_lines.split_off(lines);

                // Directly access the last line.
                let last_line = cell_lines
                    .get_mut(lines - 1)
                    .expect("We know it's this long.");

                // Truncate any ansi codes, as the following cutoff might break ansi code
                // otherwise anyway. This could be handled smarter, but it's simple and just works.
                #[cfg(feature = "custom_styling")]
                {
                    let stripped = console::strip_ansi_codes(last_line).to_string();
                    *last_line = stripped;
                }

                let max_width: usize = info.content_width.into();
                let indicator_width = table.truncation_indicator.width();

                let mut truncate_at = 0;
                // Start the accumulated_width with the indicator_width, which is the minimum width
                // we may show anyway.
                let mut accumulated_width = indicator_width;
                let mut full_string_fits = false;

                // Leave these print statements in here in case we ever have to debug this annoying
                // stuff again.
                //println!("\nSTART:");
                //println!("\nMax width: {max_width}, Indicator width: {indicator_width}");
                //println!("Full line hex: {last_line}");
                //println!(
                //    "Full line hex: {}",
                //    last_line
                //        .as_bytes()
                //        .iter()
                //        .map(|byte| format!("{byte:02x}"))
                //        .collect::<Vec<String>>()
                //        .join(", ")
                //);

                // Iterate through the UTF-8 graphemes.
                // Check the `split_long_word` inline function docs to see why we're using
                // graphemes.
                // **Note:** The `index` here is the **byte** index. So we cannot just
                //    String::truncate afterwards. We have to convert to a byte vector to perform
                //    the truncation first.
                let mut grapheme_iter = last_line.grapheme_indices(true).peekable();
                while let Some((index, grapheme)) = grapheme_iter.next() {
                    // Leave these print statements in here in case we ever have to debug this
                    // annoying stuff again
                    //println!(
                    //    "Current index: {index}, Next grapheme: {grapheme} (width: {})",
                    //    grapheme.width()
                    //);
                    //println!(
                    //    "Next grapheme hex: {}",
                    //    grapheme
                    //        .as_bytes()
                    //        .iter()
                    //        .map(|byte| format!("{byte:02x}"))
                    //        .collect::<Vec<String>>()
                    //        .join(", ")
                    //);

                    // Immediately save where to truncate in case this grapheme doesn't fit.
                    // The index is just before the current grapheme actually starts.
                    truncate_at = index;
                    // Check if the next grapheme would break the boundary of the allowed line
                    // length.
                    let new_width = accumulated_width + grapheme.width();
                    //println!(
                    //    "Next width: {new_width}/{max_width} ({accumulated_width} + {})",
                    //    grapheme.width()
                    //);
                    if new_width > max_width {
                        //println!(
                        //    "Breaking: {:?}",
                        //    accumulated_width + grapheme.width() > max_width
                        //);
                        break;
                    }

                    // The grapheme seems to fit. Save the index and check the next one.
                    accumulated_width += grapheme.width();

                    // This is a special case.
                    // We reached the last char, meaning that full last line + the indicator fit.
                    if grapheme_iter.peek().is_none() {
                        full_string_fits = true
                    }
                }

                // Only do any truncation logic if the line doesn't fit.
                if !full_string_fits {
                    // Truncate the string at the byte index just behind the last valid grapheme
                    // and overwrite the last line with the new truncated string.
                    let mut last_line_bytes = last_line.clone().into_bytes();
                    last_line_bytes.truncate(truncate_at);
                    let new_last_line = String::from_utf8(last_line_bytes)
                        .expect("We cut at an exact char boundary");
                    *last_line = new_last_line;
                }

                // Push the truncation indicator.
                last_line.push_str(&table.truncation_indicator);
            }
        }

        // Iterate over all generated lines of this cell and align them
        let cell_lines = cell_lines
            .iter()
            .map(|line| align_line(table, info, cell, line.to_string()));

        temp_row_content.push(cell_lines.collect());
    }

    // Right now, we have a different structure than desired.
    // The content is organized by `row->cell->line`.
    // We want to remove the cell from our datastructure, since this makes the next step a lot easier.
    // In the end it should look like this: `row->lines->column`.
    // To achieve this, we calculate the max amount of lines for the current row.
    // Afterwards, we iterate over each cell and convert the current structure to the desired one.
    // This step basically transforms this:
    //  tc[0][0][0]     tc[0][1][0]
    //  tc[0][0][1]     tc[0][1][1]
    //  tc[0][0][2]     This part of the line is missing
    //
    // to this:
    //  tc[0][0][0]     tc[0][0][1]
    //  tc[0][1][0]     tc[0][1][1]
    //  tc[0][2][0]     tc[0][2][1] <- Now filled with placeholder (spaces)
    let max_lines = temp_row_content.iter().map(Vec::len).max().unwrap_or(0);
    let mut row_content = Vec::with_capacity(max_lines * display_infos.len());

    // Each column should have `max_lines` for this row.
    // Cells content with fewer lines will simply be topped up with empty strings.
    for index in 0..max_lines {
        let mut line = Vec::with_capacity(display_infos.len());
        let mut cell_iter = temp_row_content.iter();

        for info in display_infos.iter() {
            if info.is_hidden {
                continue;
            }
            let cell = cell_iter.next().unwrap();
            match cell.get(index) {
                // The current cell has content for this line. Append it
                Some(content) => line.push(content.clone()),
                // The current cell doesn't have content for this line.
                // Fill with a placeholder (empty spaces)
                None => line.push(" ".repeat(info.width().into())),
            }
        }
        row_content.push(line);
    }

    row_content
}

/// Apply the alignment for a column. Alignment can be either Left/Right/Center.
/// In every case all lines will be exactly the same character length `info.width - padding long`
/// This is needed, so we can simply insert it into the border frame later on.
/// Padding is applied in this function as well.
#[allow(unused_variables)]
fn align_line(table: &Table, info: &ColumnDisplayInfo, cell: &Cell, mut line: String) -> String {
    let content_width = info.content_width;
    let remaining: usize = usize::from(content_width).saturating_sub(measure_text_width(&line));

    // Apply the styling before aligning the line, if the user requests it.
    // That way non-delimiter whitespaces won't have stuff like underlines.
    #[cfg(feature = "tty")]
    if table.should_style() && table.style_text_only {
        line = style_line(line, cell);
    }

    // Determine the alignment of the column cells.
    // Cell settings overwrite the columns Alignment settings.
    // Default is Left
    let alignment = if let Some(alignment) = cell.alignment {
        alignment
    } else if let Some(alignment) = info.cell_alignment {
        alignment
    } else {
        CellAlignment::Left
    };

    // Apply left/right/both side padding depending on the alignment of the column
    match alignment {
        CellAlignment::Left => {
            line += &" ".repeat(remaining);
        }
        CellAlignment::Right => {
            line = " ".repeat(remaining) + &line;
        }
        CellAlignment::Center => {
            let left_padding = (remaining as f32 / 2f32).ceil() as usize;
            let right_padding = (remaining as f32 / 2f32).floor() as usize;
            line = " ".repeat(left_padding) + &line + &" ".repeat(right_padding);
        }
    }

    line = pad_line(&line, info);

    #[cfg(feature = "tty")]
    if table.should_style() && !table.style_text_only {
        return style_line(line, cell);
    }

    line
}

/// Apply the column's padding to this line
fn pad_line(line: &str, info: &ColumnDisplayInfo) -> String {
    let mut padded_line = String::new();

    padded_line += &" ".repeat(info.padding.0.into());
    padded_line += line;
    padded_line += &" ".repeat(info.padding.1.into());

    padded_line
}

#[cfg(feature = "tty")]
fn style_line(line: String, cell: &Cell) -> String {
    // Just return the line, if there's no need to style.
    if cell.fg.is_none() && cell.bg.is_none() && cell.attributes.is_empty() {
        return line;
    }

    let mut content = style(line);

    // Apply text color
    if let Some(color) = cell.fg {
        content = content.with(map_color(color));
    }

    // Apply background color
    if let Some(color) = cell.bg {
        content = content.on(map_color(color));
    }

    for attribute in cell.attributes.iter() {
        content = content.attribute(map_attribute(*attribute));
    }

    content.to_string()
}
