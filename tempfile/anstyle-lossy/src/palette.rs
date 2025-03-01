//! Popular color palettes for [`anstyle::AnsiColor`]
//!
//! Based on [wikipedia](https://en.wikipedia.org/wiki/ANSI_escape_code#3-bit_and_4-bit)
use anstyle::RgbColor as Rgb;

/// A color palette for rendering 4-bit [`anstyle::AnsiColor`]
#[allow(clippy::exhaustive_structs)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Palette(pub RawPalette);
type RawPalette = [Rgb; 16];

impl Palette {
    /// Look up the [`anstyle::RgbColor`] in the palette
    pub const fn get(&self, color: anstyle::AnsiColor) -> Rgb {
        let color = anstyle::Ansi256Color::from_ansi(color);
        *self.get_ansi256_ref(color)
    }
    const fn get_ansi256_ref(&self, color: anstyle::Ansi256Color) -> &Rgb {
        let index = color.index() as usize;
        &self.0[index]
    }

    pub(crate) const fn rgb_from_ansi(&self, color: anstyle::AnsiColor) -> anstyle::RgbColor {
        self.get(color)
    }

    pub(crate) const fn rgb_from_index(&self, index: u8) -> Option<anstyle::RgbColor> {
        let index = index as usize;
        if index < self.0.len() {
            Some(self.0[index])
        } else {
            None
        }
    }

    pub(crate) const fn find_match(&self, color: anstyle::RgbColor) -> anstyle::AnsiColor {
        let mut best_index = 0;
        let mut best_distance = crate::distance(color, self.0[best_index]);

        let mut index = best_index + 1;
        while index < self.0.len() {
            let distance = crate::distance(color, self.0[index]);
            if distance < best_distance {
                best_index = index;
                best_distance = distance;
            }

            index += 1;
        }

        if let Some(color) = anstyle::Ansi256Color(best_index as u8).into_ansi() {
            color
        } else {
            // Panic
            #[allow(clippy::no_effect)]
            ["best_index is out of bounds"][best_index];
            // Make compiler happy
            anstyle::AnsiColor::Black
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        DEFAULT
    }
}

impl std::ops::Index<anstyle::AnsiColor> for Palette {
    type Output = Rgb;

    #[inline]
    fn index(&self, color: anstyle::AnsiColor) -> &Rgb {
        let color = anstyle::Ansi256Color::from_ansi(color);
        self.get_ansi256_ref(color)
    }
}

impl From<RawPalette> for Palette {
    fn from(raw: RawPalette) -> Self {
        Self(raw)
    }
}

/// Platform-specific default
#[cfg(not(windows))]
pub use VGA as DEFAULT;

/// Platform-specific default
#[cfg(windows)]
pub use WIN10_CONSOLE as DEFAULT;

/// Typical colors that are used when booting PCs and leaving them in text mode
pub const VGA: Palette = Palette([
    Rgb(0, 0, 0),
    Rgb(170, 0, 0),
    Rgb(0, 170, 0),
    Rgb(170, 85, 0),
    Rgb(0, 0, 170),
    Rgb(170, 0, 170),
    Rgb(0, 170, 170),
    Rgb(170, 170, 170),
    Rgb(85, 85, 85),
    Rgb(255, 85, 85),
    Rgb(85, 255, 85),
    Rgb(255, 255, 85),
    Rgb(85, 85, 255),
    Rgb(255, 85, 255),
    Rgb(85, 255, 255),
    Rgb(255, 255, 255),
]);

/// Campbell theme, used as of Windows 10 version 1709.
pub const WIN10_CONSOLE: Palette = Palette([
    Rgb(12, 12, 12),
    Rgb(197, 15, 31),
    Rgb(19, 161, 14),
    Rgb(193, 156, 0),
    Rgb(0, 55, 218),
    Rgb(136, 23, 152),
    Rgb(58, 150, 221),
    Rgb(204, 204, 204),
    Rgb(118, 118, 118),
    Rgb(231, 72, 86),
    Rgb(22, 198, 12),
    Rgb(249, 241, 165),
    Rgb(59, 120, 255),
    Rgb(180, 0, 158),
    Rgb(97, 214, 214),
    Rgb(242, 242, 242),
]);
