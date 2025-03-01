use std::slice::Iter;

use crate::{
    cell::{Cell, Cells},
    utils::formatting::content_split::measure_text_width,
};

/// Each row contains [Cells](crate::Cell) and can be added to a [Table](crate::Table).
#[derive(Clone, Debug, Default)]
pub struct Row {
    /// Index of the row.
    /// This will be set as soon as the row is added to the table.
    pub(crate) index: Option<usize>,
    pub(crate) cells: Vec<Cell>,
    pub(crate) max_height: Option<usize>,
}

impl Row {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a cell to the row.
    ///
    /// **Attention:**
    /// If a row has already been added to a table and you add more cells to it
    /// than there're columns currently know to the [Table](crate::Table) struct,
    /// these columns won't be known to the table unless you call
    /// [crate::Table::discover_columns].
    ///
    /// ```rust
    /// use comfy_table::{Row, Cell};
    ///
    /// let mut row = Row::new();
    /// row.add_cell(Cell::new("One"));
    /// ```
    pub fn add_cell(&mut self, cell: Cell) -> &mut Self {
        self.cells.push(cell);

        self
    }

    /// Truncate content of cells which occupies more than X lines of space.
    ///
    /// ```
    /// use comfy_table::{Row, Cell};
    ///
    /// let mut row = Row::new();
    /// row.max_height(5);
    /// ```
    pub fn max_height(&mut self, lines: usize) -> &mut Self {
        self.max_height = Some(lines);

        self
    }

    /// Get the longest content width for all cells of this row
    pub(crate) fn max_content_widths(&self) -> Vec<usize> {
        // Iterate over all cells
        self.cells
            .iter()
            .map(|cell| {
                // Iterate over all content strings and return a vector of string widths.
                // Each entry represents the longest string width for a cell.
                cell.content
                    .iter()
                    .map(|string| measure_text_width(string))
                    .max()
                    .unwrap_or(0)
            })
            .collect()
    }

    /// Get the amount of cells on this row.
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Returns an iterator over all cells of this row
    pub fn cell_iter(&self) -> Iter<Cell> {
        self.cells.iter()
    }
}

/// Create a Row from any `Into<Cells>`. \
/// [Cells] is a simple wrapper around a `Vec<Cell>`.
///
/// Check the [From] implementations on [Cell] for more information.
///
/// ```rust
/// use comfy_table::{Row, Cell};
///
/// let row = Row::from(vec!["One", "Two", "Three",]);
/// let row = Row::from(vec![
///    Cell::new("One"),
///    Cell::new("Two"),
///    Cell::new("Three"),
/// ]);
/// let row = Row::from(vec![1, 2, 3, 4]);
/// ```
impl<T: Into<Cells>> From<T> for Row {
    fn from(cells: T) -> Self {
        Self {
            index: None,
            cells: cells.into().0,
            max_height: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_correct_max_content_width() {
        let row = Row::from(vec![
            "",
            "four",
            "fivef",
            "sixsix",
            "11 but with\na newline",
        ]);

        let max_content_widths = row.max_content_widths();

        assert_eq!(max_content_widths, vec![0, 4, 5, 6, 11]);
    }

    #[test]
    fn test_some_functions() {
        let cells = ["one", "two", "three"];
        let mut row = Row::new();
        for cell in cells.iter() {
            row.add_cell(Cell::new(cell));
        }
        assert_eq!(row.cell_count(), cells.len());

        let mut cell_content_iter = cells.iter();
        for cell in row.cell_iter() {
            assert_eq!(
                cell.content(),
                cell_content_iter.next().unwrap().to_string()
            );
        }
    }
}
