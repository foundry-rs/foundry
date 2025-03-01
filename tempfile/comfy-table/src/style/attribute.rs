/// Represents an attribute.
///
/// # Platform-specific Notes
///
/// * Only UNIX and Windows 10 terminals do support text attributes.
/// * Keep in mind that not all terminals support all attributes.
/// * Crossterm implements almost all attributes listed in the
///   [SGR parameters](https://en.wikipedia.org/wiki/ANSI_escape_code#SGR_parameters).
///
/// | Attribute | Windows | UNIX | Notes |
/// | :-- | :--: | :--: | :-- |
/// | `Reset` | ✓ | ✓ | |
/// | `Bold` | ✓ | ✓ | |
/// | `Dim` | ✓ | ✓ | |
/// | `Italic` | ? | ? | Not widely supported, sometimes treated as inverse. |
/// | `Underlined` | ✓ | ✓ | |
/// | `SlowBlink` | ? | ? | Not widely supported, sometimes treated as inverse. |
/// | `RapidBlink` | ? | ? | Not widely supported. MS-DOS ANSI.SYS; 150+ per minute. |
/// | `Reverse` | ✓ | ✓ | |
/// | `Hidden` | ✓ | ✓ | Also known as Conceal. |
/// | `Fraktur` | ✗ | ✓ | Legible characters, but marked for deletion. |
/// | `DefaultForegroundColor` | ? | ? | Implementation specific (according to standard). |
/// | `DefaultBackgroundColor` | ? | ? | Implementation specific (according to standard). |
/// | `Framed` | ? | ? | Not widely supported. |
/// | `Encircled` | ? | ? | This should turn on the encircled attribute. |
/// | `OverLined` | ? | ? | This should draw a line at the top of the text. |
///
/// Usage:
///
/// Check [crate::Cell::add_attribute] on how to use it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum Attribute {
    /// Resets all the attributes.
    Reset,
    /// Increases the text intensity.
    Bold,
    /// Decreases the text intensity.
    Dim,
    /// Emphasises the text.
    Italic,
    /// Underlines the text.
    Underlined,

    // Other types of underlining
    /// Double underlines the text.
    DoubleUnderlined,
    /// Undercurls the text.
    Undercurled,
    /// Underdots the text.
    Underdotted,
    /// Underdashes the text.
    Underdashed,

    /// Makes the text blinking (< 150 per minute).
    SlowBlink,
    /// Makes the text blinking (>= 150 per minute).
    RapidBlink,
    /// Swaps foreground and background colors.
    Reverse,
    /// Hides the text (also known as Conceal).
    Hidden,
    /// Crosses the text.
    CrossedOut,
    /// Sets the [Fraktur](https://en.wikipedia.org/wiki/Fraktur) typeface.
    ///
    /// Mostly used for [mathematical alphanumeric symbols](https://en.wikipedia.org/wiki/Mathematical_Alphanumeric_Symbols).
    Fraktur,
    /// Turns off the `Bold` attribute. - Inconsistent - Prefer to use NormalIntensity
    NoBold,
    /// Switches the text back to normal intensity (no bold, italic).
    NormalIntensity,
    /// Turns off the `Italic` attribute.
    NoItalic,
    /// Turns off the `Underlined` attribute.
    NoUnderline,
    /// Turns off the text blinking (`SlowBlink` or `RapidBlink`).
    NoBlink,
    /// Turns off the `Reverse` attribute.
    NoReverse,
    /// Turns off the `Hidden` attribute.
    NoHidden,
    /// Turns off the `CrossedOut` attribute.
    NotCrossedOut,
    /// Makes the text framed.
    Framed,
    /// Makes the text encircled.
    Encircled,
    /// Draws a line at the top of the text.
    OverLined,
    /// Turns off the `Frame` and `Encircled` attributes.
    NotFramedOrEncircled,
    /// Turns off the `OverLined` attribute.
    NotOverLined,
}

/// Map the internal mirrored [Attribute] to the actually used [crossterm::style::Attribute]
pub(crate) fn map_attribute(attribute: Attribute) -> crossterm::style::Attribute {
    match attribute {
        Attribute::Reset => crossterm::style::Attribute::Reset,
        Attribute::Bold => crossterm::style::Attribute::Bold,
        Attribute::Dim => crossterm::style::Attribute::Dim,
        Attribute::Italic => crossterm::style::Attribute::Italic,
        Attribute::Underlined => crossterm::style::Attribute::Underlined,
        Attribute::DoubleUnderlined => crossterm::style::Attribute::DoubleUnderlined,
        Attribute::Undercurled => crossterm::style::Attribute::Undercurled,
        Attribute::Underdotted => crossterm::style::Attribute::Underdotted,
        Attribute::Underdashed => crossterm::style::Attribute::Underdashed,
        Attribute::SlowBlink => crossterm::style::Attribute::SlowBlink,
        Attribute::RapidBlink => crossterm::style::Attribute::RapidBlink,
        Attribute::Reverse => crossterm::style::Attribute::Reverse,
        Attribute::Hidden => crossterm::style::Attribute::Hidden,
        Attribute::CrossedOut => crossterm::style::Attribute::CrossedOut,
        Attribute::Fraktur => crossterm::style::Attribute::Fraktur,
        Attribute::NoBold => crossterm::style::Attribute::NoBold,
        Attribute::NormalIntensity => crossterm::style::Attribute::NormalIntensity,
        Attribute::NoItalic => crossterm::style::Attribute::NoItalic,
        Attribute::NoUnderline => crossterm::style::Attribute::NoUnderline,
        Attribute::NoBlink => crossterm::style::Attribute::NoBlink,
        Attribute::NoReverse => crossterm::style::Attribute::NoReverse,
        Attribute::NoHidden => crossterm::style::Attribute::NoHidden,
        Attribute::NotCrossedOut => crossterm::style::Attribute::NotCrossedOut,
        Attribute::Framed => crossterm::style::Attribute::Framed,
        Attribute::Encircled => crossterm::style::Attribute::Encircled,
        Attribute::OverLined => crossterm::style::Attribute::OverLined,
        Attribute::NotFramedOrEncircled => crossterm::style::Attribute::NotFramedOrEncircled,
        Attribute::NotOverLined => crossterm::style::Attribute::NotOverLined,
    }
}
