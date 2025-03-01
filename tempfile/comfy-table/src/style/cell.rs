/// This can be set on [columns](crate::Column::set_cell_alignment) and [cells](crate::Cell::set_alignment).
///
/// Determines how content of cells should be aligned.
///
/// ```text
/// +----------------------+
/// | Header1              |
/// +======================+
/// | Left                 |
/// |----------------------+
/// |        center        |
/// |----------------------+
/// |                right |
/// +----------------------+
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CellAlignment {
    Left,
    Right,
    Center,
}
