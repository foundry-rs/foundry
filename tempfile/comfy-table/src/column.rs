use crate::style::{CellAlignment, ColumnConstraint};

/// A representation of a table's column.
/// Useful for styling and specifying constraints how big a column should be.
///
/// 1. Content padding for cells in this column
/// 2. Constraints on how wide this column shall be
/// 3. Default alignment for cells in this column
///
/// Columns are generated when adding rows or a header to a table.\
/// As a result columns can only be modified after the table is populated by some data.
///
/// ```
/// use comfy_table::{Width::*, CellAlignment, ColumnConstraint::*, Table};
///
/// let mut table = Table::new();
/// table.set_header(&vec!["one", "two"]);
///
/// let mut column = table.column_mut(1).expect("This should be column two");
///
/// // Set the max width for all cells of this column to 20 characters.
/// column.set_constraint(UpperBoundary(Fixed(20)));
///
/// // Set the left padding to 5 spaces and the right padding to 1 space
/// column.set_padding((5, 1));
///
/// // Align content in all cells of this column to the center of the cell.
/// column.set_cell_alignment(CellAlignment::Center);
/// ```
#[derive(Debug, Clone)]
pub struct Column {
    /// The index of the column
    pub index: usize,
    /// Left/right padding for each cell of this column in spaces
    pub(crate) padding: (u16, u16),
    /// The delimiter which is used to split the text into consistent pieces.
    /// Default is ` `.
    pub(crate) delimiter: Option<char>,
    /// Define the [CellAlignment] for all cells of this column
    pub(crate) cell_alignment: Option<CellAlignment>,
    pub(crate) constraint: Option<ColumnConstraint>,
}

impl Column {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            padding: (1, 1),
            delimiter: None,
            constraint: None,
            cell_alignment: None,
        }
    }

    /// Set the padding for all cells of this column.
    ///
    /// Padding is provided in the form of (left, right).\
    /// Default is `(1, 1)`.
    pub fn set_padding(&mut self, padding: (u16, u16)) -> &mut Self {
        self.padding = padding;

        self
    }

    /// Convenience helper that returns the total width of the combined padding.
    pub fn padding_width(&self) -> u16 {
        self.padding.0.saturating_add(self.padding.1)
    }

    /// Set the delimiter used to split text for this column's cells.
    ///
    /// A custom delimiter on a cell in will overwrite the column's delimiter.
    /// Normal text uses spaces (` `) as delimiters. This is necessary to help comfy-table
    /// understand the concept of _words_.
    pub fn set_delimiter(&mut self, delimiter: char) -> &mut Self {
        self.delimiter = Some(delimiter);

        self
    }

    /// Constraints allow to influence the auto-adjustment behavior of columns.\
    /// This can be useful to counter undesired auto-adjustment of content in tables.
    pub fn set_constraint(&mut self, constraint: ColumnConstraint) -> &mut Self {
        self.constraint = Some(constraint);

        self
    }

    /// Get the constraint that is used for this column.
    pub fn constraint(&self) -> Option<&ColumnConstraint> {
        self.constraint.as_ref()
    }

    /// Remove any constraint on this column
    pub fn remove_constraint(&mut self) -> &mut Self {
        self.constraint = None;

        self
    }

    /// Returns weather the columns is hidden via [ColumnConstraint::Hidden].
    pub fn is_hidden(&self) -> bool {
        matches!(self.constraint, Some(ColumnConstraint::Hidden))
    }

    /// Set the alignment for content inside of cells for this column.\
    /// **Note:** Alignment on a cell will always overwrite the column's setting.
    pub fn set_cell_alignment(&mut self, alignment: CellAlignment) {
        self.cell_alignment = Some(alignment);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column() {
        let mut column = Column::new(0);
        column.set_padding((0, 0));

        column.set_constraint(ColumnConstraint::ContentWidth);
        assert_eq!(column.constraint(), Some(&ColumnConstraint::ContentWidth));

        column.remove_constraint();
        assert_eq!(column.constraint(), None);
    }
}
