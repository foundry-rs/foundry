use crate::style::TableComponent;
use crate::table::Table;
use crate::utils::ColumnDisplayInfo;

pub(crate) fn draw_borders(
    table: &Table,
    rows: &[Vec<Vec<String>>],
    display_info: &[ColumnDisplayInfo],
) -> Vec<String> {
    // We know how many lines there should be. Initialize the vector with the rough correct amount.
    // We might over allocate a bit, but that's better than under allocating.
    let mut lines = if let Some(capacity) = rows.first().map(|lines| lines.len()) {
        // Lines * 2 -> Lines + delimiters
        // + 5 -> header delimiters + header + bottom/top borders
        Vec::with_capacity(capacity * 2 + 5)
    } else {
        Vec::new()
    };

    if should_draw_top_border(table) {
        lines.push(draw_top_border(table, display_info));
    }

    draw_rows(&mut lines, rows, table, display_info);

    if should_draw_bottom_border(table) {
        lines.push(draw_bottom_border(table, display_info));
    }

    lines
}

fn draw_top_border(table: &Table, display_info: &[ColumnDisplayInfo]) -> String {
    let left_corner = table.style_or_default(TableComponent::TopLeftCorner);
    let top_border = table.style_or_default(TableComponent::TopBorder);
    let intersection = table.style_or_default(TableComponent::TopBorderIntersections);
    let right_corner = table.style_or_default(TableComponent::TopRightCorner);

    let mut line = String::new();
    // We only need the top left corner, if we need to draw a left border
    if should_draw_left_border(table) {
        line += &left_corner;
    }

    // Build the top border line depending on the columns' width.
    // Also add the border intersections.
    let mut first = true;
    for info in display_info.iter() {
        // Only add something, if the column isn't hidden
        if !info.is_hidden {
            if !first {
                line += &intersection;
            }
            line += &top_border.repeat(info.width().into());
            first = false;
        }
    }

    // We only need the top right corner, if we need to draw a right border
    if should_draw_right_border(table) {
        line += &right_corner;
    }

    line
}

fn draw_rows(
    lines: &mut Vec<String>,
    rows: &[Vec<Vec<String>>],
    table: &Table,
    display_info: &[ColumnDisplayInfo],
) {
    // Iterate over all rows
    let mut row_iter = rows.iter().enumerate().peekable();
    while let Some((row_index, row)) = row_iter.next() {
        // Concatenate the line parts and insert the vertical borders if needed
        for line_parts in row.iter() {
            lines.push(embed_line(line_parts, table));
        }

        // Draw the horizontal header line if desired, otherwise continue to the next iteration
        if row_index == 0 && table.header.is_some() {
            if should_draw_header(table) {
                lines.push(draw_horizontal_lines(table, display_info, true));
            }
            continue;
        }

        // Draw a horizontal line, if we desired and if we aren't in the last row of the table.
        if row_iter.peek().is_some() && should_draw_horizontal_lines(table) {
            lines.push(draw_horizontal_lines(table, display_info, false));
        }
    }
}

// Takes the parts of a single line, surrounds them with borders and adds vertical lines.
fn embed_line(line_parts: &[String], table: &Table) -> String {
    let vertical_lines = table.style_or_default(TableComponent::VerticalLines);
    let left_border = table.style_or_default(TableComponent::LeftBorder);
    let right_border = table.style_or_default(TableComponent::RightBorder);

    let mut line = String::new();
    if should_draw_left_border(table) {
        line += &left_border;
    }

    let mut part_iter = line_parts.iter().peekable();
    while let Some(part) = part_iter.next() {
        line += part;
        if should_draw_vertical_lines(table) && part_iter.peek().is_some() {
            line += &vertical_lines;
        } else if should_draw_right_border(table) && part_iter.peek().is_none() {
            line += &right_border;
        }
    }

    line
}

// The horizontal line that separates between rows.
fn draw_horizontal_lines(
    table: &Table,
    display_info: &[ColumnDisplayInfo],
    header: bool,
) -> String {
    // Styling depends on whether we're currently on the header line or not.
    let (left_intersection, horizontal_lines, middle_intersection, right_intersection) = if header {
        (
            table.style_or_default(TableComponent::LeftHeaderIntersection),
            table.style_or_default(TableComponent::HeaderLines),
            table.style_or_default(TableComponent::MiddleHeaderIntersections),
            table.style_or_default(TableComponent::RightHeaderIntersection),
        )
    } else {
        (
            table.style_or_default(TableComponent::LeftBorderIntersections),
            table.style_or_default(TableComponent::HorizontalLines),
            table.style_or_default(TableComponent::MiddleIntersections),
            table.style_or_default(TableComponent::RightBorderIntersections),
        )
    };

    let mut line = String::new();
    // We only need the bottom left corner, if we need to draw a left border
    if should_draw_left_border(table) {
        line += &left_intersection;
    }

    // Append the middle lines depending on the columns' widths.
    // Also add the middle intersections.
    let mut first = true;
    for info in display_info.iter() {
        // Only add something, if the column isn't hidden
        if !info.is_hidden {
            if !first {
                line += &middle_intersection;
            }
            line += &horizontal_lines.repeat(info.width().into());
            first = false;
        }
    }

    // We only need the bottom right corner, if we need to draw a right border
    if should_draw_right_border(table) {
        line += &right_intersection;
    }

    line
}

fn draw_bottom_border(table: &Table, display_info: &[ColumnDisplayInfo]) -> String {
    let left_corner = table.style_or_default(TableComponent::BottomLeftCorner);
    let bottom_border = table.style_or_default(TableComponent::BottomBorder);
    let middle_intersection = table.style_or_default(TableComponent::BottomBorderIntersections);
    let right_corner = table.style_or_default(TableComponent::BottomRightCorner);

    let mut line = String::new();
    // We only need the bottom left corner, if we need to draw a left border
    if should_draw_left_border(table) {
        line += &left_corner;
    }

    // Add the bottom border lines depending on column width
    // Also add the border intersections.
    let mut first = true;
    for info in display_info.iter() {
        // Only add something, if the column isn't hidden
        if !info.is_hidden {
            if !first {
                line += &middle_intersection;
            }
            line += &bottom_border.repeat(info.width().into());
            first = false;
        }
    }

    // We only need the bottom right corner, if we need to draw a right border
    if should_draw_right_border(table) {
        line += &right_corner;
    }

    line
}

fn should_draw_top_border(table: &Table) -> bool {
    if table.style_exists(TableComponent::TopLeftCorner)
        || table.style_exists(TableComponent::TopBorder)
        || table.style_exists(TableComponent::TopBorderIntersections)
        || table.style_exists(TableComponent::TopRightCorner)
    {
        return true;
    }

    false
}

fn should_draw_bottom_border(table: &Table) -> bool {
    if table.style_exists(TableComponent::BottomLeftCorner)
        || table.style_exists(TableComponent::BottomBorder)
        || table.style_exists(TableComponent::BottomBorderIntersections)
        || table.style_exists(TableComponent::BottomRightCorner)
    {
        return true;
    }

    false
}

pub fn should_draw_left_border(table: &Table) -> bool {
    if table.style_exists(TableComponent::TopLeftCorner)
        || table.style_exists(TableComponent::LeftBorder)
        || table.style_exists(TableComponent::LeftBorderIntersections)
        || table.style_exists(TableComponent::LeftHeaderIntersection)
        || table.style_exists(TableComponent::BottomLeftCorner)
    {
        return true;
    }

    false
}

pub fn should_draw_right_border(table: &Table) -> bool {
    if table.style_exists(TableComponent::TopRightCorner)
        || table.style_exists(TableComponent::RightBorder)
        || table.style_exists(TableComponent::RightBorderIntersections)
        || table.style_exists(TableComponent::RightHeaderIntersection)
        || table.style_exists(TableComponent::BottomRightCorner)
    {
        return true;
    }

    false
}

fn should_draw_horizontal_lines(table: &Table) -> bool {
    if table.style_exists(TableComponent::LeftBorderIntersections)
        || table.style_exists(TableComponent::HorizontalLines)
        || table.style_exists(TableComponent::MiddleIntersections)
        || table.style_exists(TableComponent::RightBorderIntersections)
    {
        return true;
    }

    false
}

pub fn should_draw_vertical_lines(table: &Table) -> bool {
    if table.style_exists(TableComponent::TopBorderIntersections)
        || table.style_exists(TableComponent::MiddleHeaderIntersections)
        || table.style_exists(TableComponent::VerticalLines)
        || table.style_exists(TableComponent::MiddleIntersections)
        || table.style_exists(TableComponent::BottomBorderIntersections)
    {
        return true;
    }

    false
}

fn should_draw_header(table: &Table) -> bool {
    if table.style_exists(TableComponent::LeftHeaderIntersection)
        || table.style_exists(TableComponent::HeaderLines)
        || table.style_exists(TableComponent::MiddleHeaderIntersections)
        || table.style_exists(TableComponent::RightHeaderIntersection)
    {
        return true;
    }

    false
}
