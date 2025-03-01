use std::fmt;
use std::mem::zeroed;

use winapi::um::wincontypes::CONSOLE_FONT_INFO;

use crate::Size;

/// Information about the font.
///
/// This wraps
/// [`CONSOLE_FONT_INFO`](https://learn.microsoft.com/en-us/windows/console/console-font-info-str).
#[derive(Clone)]
pub struct FontInfo(pub CONSOLE_FONT_INFO);

// TODO: replace this with a derive ASAP
impl fmt::Debug for FontInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("FontInfo")
            .field("dwFontSize", &self.size())
            .field("nFont", &self.index())
            .finish()
    }
}

impl FontInfo {
    /// Create a new font info without all zeroed properties.
    pub fn new() -> FontInfo {
        FontInfo(unsafe { zeroed() })
    }

    /// Get the size of the font.
    ///
    /// Will take `dwFontSize` from the current font info and convert it into a [`Size`].
    pub fn size(&self) -> Size {
        Size::from(self.0.dwFontSize)
    }

    /// Get the index of the font in the system's console font table.
    pub fn index(&self) -> u32 {
        self.0.nFont
    }
}
