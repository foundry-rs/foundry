/// Represents a color.
///
/// This type is a simplified re-implementation of crossterm's Color enum.
/// See [crossterm::style::color](https://docs.rs/crossterm/latest/crossterm/style/enum.Color.html)
///
/// # Platform-specific Notes
///
/// The following list of 16 base colors are available for almost all terminals (Windows 7 and 8 included).
///
/// | Light      | Dark          |
/// | :--------- | :------------ |
/// | `DarkGrey` | `Black`       |
/// | `Red`      | `DarkRed`     |
/// | `Green`    | `DarkGreen`   |
/// | `Yellow`   | `DarkYellow`  |
/// | `Blue`     | `DarkBlue`    |
/// | `Magenta`  | `DarkMagenta` |
/// | `Cyan`     | `DarkCyan`    |
/// | `White`    | `Grey`        |
///
/// Most UNIX terminals and Windows 10 consoles support additional colors.
/// See [Color::Rgb] or [Color::AnsiValue] for more info.
///
/// Usage:
///
/// Check [crate::Cell::bg], [crate::Cell::fg] and  on how to use it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub enum Color {
    /// Resets the terminal color.
    Reset,

    /// Black color.
    Black,

    /// Dark grey color.
    DarkGrey,

    /// Light red color.
    Red,

    /// Dark red color.
    DarkRed,

    /// Light green color.
    Green,

    /// Dark green color.
    DarkGreen,

    /// Light yellow color.
    Yellow,

    /// Dark yellow color.
    DarkYellow,

    /// Light blue color.
    Blue,

    /// Dark blue color.
    DarkBlue,

    /// Light magenta color.
    Magenta,

    /// Dark magenta color.
    DarkMagenta,

    /// Light cyan color.
    Cyan,

    /// Dark cyan color.
    DarkCyan,

    /// White color.
    White,

    /// Grey color.
    Grey,

    /// An RGB color. See [RGB color model](https://en.wikipedia.org/wiki/RGB_color_model) for more info.
    ///
    /// Most UNIX terminals and Windows 10 supported only.
    /// See [Platform-specific notes](enum.Color.html#platform-specific-notes) for more info.
    Rgb { r: u8, g: u8, b: u8 },

    /// An ANSI color. See [256 colors - cheat sheet](https://jonasjacek.github.io/colors/) for more info.
    ///
    /// Most UNIX terminals and Windows 10 supported only.
    /// See [Platform-specific notes](enum.Color.html#platform-specific-notes) for more info.
    AnsiValue(u8),
}

/// Map the internal mirrored [Color] enum to the actually used [crossterm::style::Color].
pub(crate) fn map_color(color: Color) -> crossterm::style::Color {
    match color {
        Color::Reset => crossterm::style::Color::Reset,
        Color::Black => crossterm::style::Color::Black,
        Color::DarkGrey => crossterm::style::Color::DarkGrey,
        Color::Red => crossterm::style::Color::Red,
        Color::DarkRed => crossterm::style::Color::DarkRed,
        Color::Green => crossterm::style::Color::Green,
        Color::DarkGreen => crossterm::style::Color::DarkGreen,
        Color::Yellow => crossterm::style::Color::Yellow,
        Color::DarkYellow => crossterm::style::Color::DarkYellow,
        Color::Blue => crossterm::style::Color::Blue,
        Color::DarkBlue => crossterm::style::Color::DarkBlue,
        Color::Magenta => crossterm::style::Color::Magenta,
        Color::DarkMagenta => crossterm::style::Color::DarkMagenta,
        Color::Cyan => crossterm::style::Color::Cyan,
        Color::DarkCyan => crossterm::style::Color::DarkCyan,
        Color::White => crossterm::style::Color::White,
        Color::Grey => crossterm::style::Color::Grey,
        Color::Rgb { r, g, b } => crossterm::style::Color::Rgb { r, g, b },
        Color::AnsiValue(value) => crossterm::style::Color::AnsiValue(value),
    }
}
