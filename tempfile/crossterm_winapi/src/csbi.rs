use std::fmt;
use std::mem::zeroed;

use winapi::um::wincon::CONSOLE_SCREEN_BUFFER_INFO;

use super::{Coord, Size, WindowPositions};

/// Information about a console screen buffer.
///
/// This wraps
/// [`CONSOLE_SCREEN_BUFFER_INFO`](https://docs.microsoft.com/en-us/windows/console/console-screen-buffer-info-str).
// TODO: replace the innards of this type with our own, more friendly types, like Coord.
// This will obviously be a breaking change.
#[derive(Clone)]
pub struct ScreenBufferInfo(pub CONSOLE_SCREEN_BUFFER_INFO);

// TODO: replace this with a derive ASAP
impl fmt::Debug for ScreenBufferInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ScreenBufferInfo")
            .field("dwSize", &self.buffer_size())
            .field("dwCursorPosition", &self.cursor_pos())
            .field("wAttributes", &self.attributes()) // TODO: hex print this
            .field("srWindow", &self.terminal_window())
            .field(
                "dwMaximumWindowSize",
                &Size::from(self.0.dwMaximumWindowSize),
            )
            .finish()
    }
}

impl ScreenBufferInfo {
    /// Create a new console screen buffer without all zeroed properties.
    pub fn new() -> ScreenBufferInfo {
        ScreenBufferInfo(unsafe { zeroed() })
    }

    /// Get the size of the screen buffer.
    ///
    /// Will take `dwSize` from the current screen buffer and convert it into a [`Size`].
    pub fn buffer_size(&self) -> Size {
        Size::from(self.0.dwSize)
    }

    /// Get the size of the terminal display window.
    ///
    /// Will calculate the width and height from `srWindow` and convert it into a [`Size`].
    pub fn terminal_size(&self) -> Size {
        Size::new(
            self.0.srWindow.Right - self.0.srWindow.Left,
            self.0.srWindow.Bottom - self.0.srWindow.Top,
        )
    }

    /// Get the position and size of the terminal display window.
    ///
    /// Will take `srWindow` and convert it into the `WindowPositions` type.
    pub fn terminal_window(&self) -> WindowPositions {
        WindowPositions::from(self.0)
    }

    /// Get the current attributes of the characters that are being written to the console.
    ///
    /// Will take `wAttributes` from the current screen buffer.
    pub fn attributes(&self) -> u16 {
        self.0.wAttributes
    }

    /// Get the current column and row of the terminal cursor in the screen buffer.
    ///
    /// Will take `dwCursorPosition` from the current screen buffer.
    pub fn cursor_pos(&self) -> Coord {
        Coord::from(self.0.dwCursorPosition)
    }
}

impl From<CONSOLE_SCREEN_BUFFER_INFO> for ScreenBufferInfo {
    fn from(csbi: CONSOLE_SCREEN_BUFFER_INFO) -> Self {
        ScreenBufferInfo(csbi)
    }
}
