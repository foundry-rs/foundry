#[cfg(feature = "tty")]
use crate::{Attribute, Color};

use crate::style::CellAlignment;

/// A stylable table cell with content.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    /// The content is a list of strings.\
    /// This is done to make working with newlines more easily.\
    /// When creating a new [Cell], the given content is split by newline.
    pub(crate) content: Vec<String>,
    /// The delimiter which is used to split the text into consistent pieces.\
    /// The default is ` `.
    pub(crate) delimiter: Option<char>,
    pub(crate) alignment: Option<CellAlignment>,
    #[cfg(feature = "tty")]
    pub(crate) fg: Option<Color>,
    #[cfg(feature = "tty")]
    pub(crate) bg: Option<Color>,
    #[cfg(feature = "tty")]
    pub(crate) attributes: Vec<Attribute>,
}

impl Cell {
    /// Create a new Cell
    #[allow(clippy::needless_pass_by_value)]
    pub fn new<T: ToString>(content: T) -> Self {
        let content = content.to_string();
        #[cfg_attr(not(feature = "custom_styling"), allow(unused_mut))]
        let mut split_content: Vec<String> = content.split('\n').map(ToString::to_string).collect();

        // Correct ansi codes so style is terminated and resumed around the split
        #[cfg(feature = "custom_styling")]
        crate::utils::formatting::content_split::fix_style_in_split_str(&mut split_content);

        Self {
            content: split_content,
            delimiter: None,
            alignment: None,
            #[cfg(feature = "tty")]
            fg: None,
            #[cfg(feature = "tty")]
            bg: None,
            #[cfg(feature = "tty")]
            attributes: Vec::new(),
        }
    }

    /// Return a copy of the content contained in this cell.
    pub fn content(&self) -> String {
        self.content.join("\n")
    }

    /// Set the delimiter used to split text for this cell. \
    /// Normal text uses spaces (` `) as delimiters. This is necessary to help comfy-table
    /// understand the concept of _words_.
    #[must_use]
    pub fn set_delimiter(mut self, delimiter: char) -> Self {
        self.delimiter = Some(delimiter);

        self
    }

    /// Set the alignment of content for this cell.
    ///
    /// Setting this overwrites alignment settings of the
    /// [Column](crate::column::Column::set_cell_alignment) for this specific cell.
    /// ```
    /// use comfy_table::CellAlignment;
    /// use comfy_table::Cell;
    ///
    /// let mut cell = Cell::new("Some content")
    ///     .set_alignment(CellAlignment::Center);
    /// ```
    #[must_use]
    pub fn set_alignment(mut self, alignment: CellAlignment) -> Self {
        self.alignment = Some(alignment);

        self
    }

    /// Set the foreground text color for this cell.
    ///
    /// Look at [Color](crate::Color) for a list of all possible Colors.
    /// ```
    /// use comfy_table::Color;
    /// use comfy_table::Cell;
    ///
    /// let mut cell = Cell::new("Some content")
    ///     .fg(Color::Red);
    /// ```
    #[cfg(feature = "tty")]
    #[must_use]
    pub fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);

        self
    }

    /// Set the background color for this cell.
    ///
    /// Look at [Color](crate::Color) for a list of all possible Colors.
    /// ```
    /// use comfy_table::Color;
    /// use comfy_table::Cell;
    ///
    /// let mut cell = Cell::new("Some content")
    ///     .bg(Color::Red);
    /// ```
    #[cfg(feature = "tty")]
    #[must_use]
    pub fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);

        self
    }

    /// Add a styling attribute to the content cell.\
    /// Those can be **bold**, _italic_, blinking and many more.
    ///
    /// Look at [Attribute](crate::Attribute) for a list of all possible Colors.
    /// ```
    /// use comfy_table::Attribute;
    /// use comfy_table::Cell;
    ///
    /// let mut cell = Cell::new("Some content")
    ///     .add_attribute(Attribute::Bold);
    /// ```
    #[cfg(feature = "tty")]
    #[must_use]
    pub fn add_attribute(mut self, attribute: Attribute) -> Self {
        self.attributes.push(attribute);

        self
    }

    /// Same as add_attribute, but you can pass a vector of [Attributes](Attribute)
    #[cfg(feature = "tty")]
    #[must_use]
    pub fn add_attributes(mut self, mut attribute: Vec<Attribute>) -> Self {
        self.attributes.append(&mut attribute);

        self
    }
}

/// Convert anything with [ToString] to a new [Cell].
///
/// ```
/// # use comfy_table::Cell;
/// let cell: Cell = "content".into();
/// let cell: Cell = 5u32.into();
/// ```
impl<T: ToString> From<T> for Cell {
    fn from(content: T) -> Self {
        Self::new(content)
    }
}

/// A simple wrapper type for a `Vec<Cell>`.
///
/// This wrapper is needed to support generic conversions between iterables and `Vec<Cell>`.
/// Check the trait implementations for more docs.
pub struct Cells(pub Vec<Cell>);

/// Allow the conversion of a type to a [Cells], which is a simple vector of cells.
///
/// By default this is implemented for all Iterators over items implementing [ToString].
///
/// ```
/// use comfy_table::{Row, Cells};
///
/// let cells_string: Cells = vec!["One", "Two", "Three"].into();
/// let cells_integer: Cells = vec![1, 2, 3, 4].into();
/// ```
impl<T> From<T> for Cells
where
    T: IntoIterator,
    T::Item: Into<Cell>,
{
    fn from(cells: T) -> Self {
        Self(cells.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_generation() {
        let content = "This is\nsome multiline\nstring".to_string();
        let cell = Cell::new(content.clone());

        assert_eq!(cell.content(), content);
    }
}
