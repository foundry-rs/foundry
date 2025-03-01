//! This module provides a type that represents some rectangle.
//! For example, in WinAPI we have `SMALL_RECT` to represent a window size but this is a little inconvenient.
//! This module provides some trait implementations who will make parsing and working with `SMALL_RECT` easier.

use winapi::um::wincon::{CONSOLE_SCREEN_BUFFER_INFO, SMALL_RECT};

/// This is a wrapper for the locations of a rectangle.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct WindowPositions {
    /// The rectangle's offset from the left.
    pub left: i16,
    /// The rectangle's offset from the right.
    pub right: i16,
    /// The rectangle's offset from the bottom.
    pub bottom: i16,
    /// The rectangle's offset from the top.
    pub top: i16,
}

impl From<CONSOLE_SCREEN_BUFFER_INFO> for WindowPositions {
    fn from(csbi: CONSOLE_SCREEN_BUFFER_INFO) -> Self {
        csbi.srWindow.into()
    }
}

impl From<WindowPositions> for SMALL_RECT {
    fn from(positions: WindowPositions) -> Self {
        SMALL_RECT {
            Top: positions.top,
            Right: positions.right,
            Bottom: positions.bottom,
            Left: positions.left,
        }
    }
}

impl From<SMALL_RECT> for WindowPositions {
    fn from(rect: SMALL_RECT) -> Self {
        WindowPositions {
            left: rect.Left,
            right: rect.Right,
            bottom: rect.Bottom,
            top: rect.Top,
        }
    }
}
