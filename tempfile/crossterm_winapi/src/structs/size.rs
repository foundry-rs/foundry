//! This module provides a type that represents some size.
//! For example, in WinAPI we have `COORD` to represent screen/buffer size but this is a little inconvenient.
//! This module provides some trait implementations who will make parsing and working with `COORD` easier.

use winapi::um::wincon::COORD;

/// This is type represents the size of something in width and height.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct Size {
    pub width: i16,
    pub height: i16,
}

impl Size {
    /// Create a new size instance by passing in the width and height.
    pub fn new(width: i16, height: i16) -> Size {
        Size { width, height }
    }
}

impl From<COORD> for Size {
    fn from(coord: COORD) -> Self {
        Size::new(coord.X, coord.Y)
    }
}

impl Into<(u16, u16)> for Size {
    fn into(self) -> (u16, u16) {
        (self.width as u16, self.height as u16)
    }
}
