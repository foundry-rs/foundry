pub mod arrangement;
pub mod formatting;

use crate::style::{CellAlignment, ColumnConstraint};
use crate::{Column, Table};

use arrangement::arrange_content;
use formatting::borders::draw_borders;
use formatting::content_format::format_content;

/// This struct is ONLY used when table.to_string() is called.
/// It's purpose is to store intermediate results, information on how to
/// arrange the table and other convenience variables.
///
/// The idea is to have a place for all this intermediate stuff, without
/// actually touching the Column struct.
#[derive(Debug)]
pub struct ColumnDisplayInfo {
    pub padding: (u16, u16),
    pub delimiter: Option<char>,
    /// The actual allowed content width after arrangement
    pub content_width: u16,
    /// The content alignment of cells in this column
    pub cell_alignment: Option<CellAlignment>,
    is_hidden: bool,
}

impl ColumnDisplayInfo {
    pub fn new(column: &Column, mut content_width: u16) -> Self {
        // The min contend width may only be 1
        if content_width == 0 {
            content_width = 1;
        }
        Self {
            padding: column.padding,
            delimiter: column.delimiter,
            content_width,
            cell_alignment: column.cell_alignment,
            is_hidden: matches!(column.constraint, Some(ColumnConstraint::Hidden)),
        }
    }

    pub fn width(&self) -> u16 {
        self.content_width
            .saturating_add(self.padding.0)
            .saturating_add(self.padding.1)
    }
}

pub fn build_table(table: &Table) -> impl Iterator<Item = String> {
    let display_info = arrange_content(table);
    let content = format_content(table, &display_info);
    draw_borders(table, &content, &display_info).into_iter()
}
