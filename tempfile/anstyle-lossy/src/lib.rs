//! Lossy conversion between ANSI Color Codes

#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![warn(missing_docs)]
#![warn(clippy::print_stderr)]
#![warn(clippy::print_stdout)]

pub mod palette;

use anstyle::RgbColor as Rgb;

/// Lossily convert from any color to RGB
///
/// As the palette for 4-bit colors is terminal/user defined, a [`palette::Palette`] must be
/// provided to match against.
pub const fn color_to_rgb(color: anstyle::Color, palette: palette::Palette) -> anstyle::RgbColor {
    match color {
        anstyle::Color::Ansi(color) => ansi_to_rgb(color, palette),
        anstyle::Color::Ansi256(color) => xterm_to_rgb(color, palette),
        anstyle::Color::Rgb(color) => color,
    }
}

/// Lossily convert from any color to 256-color
///
/// As the palette for 4-bit colors is terminal/user defined, a [`palette::Palette`] must be
/// provided to match against.
pub const fn color_to_xterm(color: anstyle::Color) -> anstyle::Ansi256Color {
    match color {
        anstyle::Color::Ansi(color) => anstyle::Ansi256Color::from_ansi(color),
        anstyle::Color::Ansi256(color) => color,
        anstyle::Color::Rgb(color) => rgb_to_xterm(color),
    }
}

/// Lossily convert from any color to 4-bit color
///
/// As the palette for 4-bit colors is terminal/user defined, a [`palette::Palette`] must be
/// provided to match against.
pub const fn color_to_ansi(color: anstyle::Color, palette: palette::Palette) -> anstyle::AnsiColor {
    match color {
        anstyle::Color::Ansi(color) => color,
        anstyle::Color::Ansi256(color) => xterm_to_ansi(color, palette),
        anstyle::Color::Rgb(color) => rgb_to_ansi(color, palette),
    }
}

/// Lossily convert from 4-bit color to RGB
///
/// As the palette for 4-bit colors is terminal/user defined, a [`palette::Palette`] must be
/// provided to match against.
pub const fn ansi_to_rgb(
    color: anstyle::AnsiColor,
    palette: palette::Palette,
) -> anstyle::RgbColor {
    palette.rgb_from_ansi(color)
}

/// Lossily convert from 256-color to RGB
///
/// As 256-color palette is a superset of 4-bit colors and since the palette for 4-bit colors is
/// terminal/user defined, a [`palette::Palette`] must be provided to match against.
pub const fn xterm_to_rgb(
    color: anstyle::Ansi256Color,
    palette: palette::Palette,
) -> anstyle::RgbColor {
    match palette.rgb_from_index(color.0) {
        Some(rgb) => rgb,
        None => XTERM_COLORS[color.0 as usize],
    }
}

/// Lossily convert from the 256-color palette to 4-bit color
///
/// As the palette for 4-bit colors is terminal/user defined, a [`palette::Palette`] must be
/// provided to match against.
pub const fn xterm_to_ansi(
    color: anstyle::Ansi256Color,
    palette: palette::Palette,
) -> anstyle::AnsiColor {
    match color.0 {
        0 => anstyle::AnsiColor::Black,
        1 => anstyle::AnsiColor::Red,
        2 => anstyle::AnsiColor::Green,
        3 => anstyle::AnsiColor::Yellow,
        4 => anstyle::AnsiColor::Blue,
        5 => anstyle::AnsiColor::Magenta,
        6 => anstyle::AnsiColor::Cyan,
        7 => anstyle::AnsiColor::White,
        8 => anstyle::AnsiColor::BrightBlack,
        9 => anstyle::AnsiColor::BrightRed,
        10 => anstyle::AnsiColor::BrightGreen,
        11 => anstyle::AnsiColor::BrightYellow,
        12 => anstyle::AnsiColor::BrightBlue,
        13 => anstyle::AnsiColor::BrightMagenta,
        14 => anstyle::AnsiColor::BrightCyan,
        15 => anstyle::AnsiColor::BrightWhite,
        _ => {
            let rgb = XTERM_COLORS[color.0 as usize];
            palette.find_match(rgb)
        }
    }
}

/// Lossily convert an RGB value to a 4-bit color
///
/// As the palette for 4-bit colors is terminal/user defined, a [`palette::Palette`] must be
/// provided to match against.
pub const fn rgb_to_ansi(
    color: anstyle::RgbColor,
    palette: palette::Palette,
) -> anstyle::AnsiColor {
    palette.find_match(color)
}

/// Lossily convert an RGB value to the 256-color palette
pub const fn rgb_to_xterm(color: anstyle::RgbColor) -> anstyle::Ansi256Color {
    // Skip placeholders
    let index = find_xterm_match(color);
    anstyle::Ansi256Color(index as u8)
}

const fn find_xterm_match(color: anstyle::RgbColor) -> usize {
    let mut best_index = 16;
    let mut best_distance = distance(color, XTERM_COLORS[best_index]);

    let mut index = best_index + 1;
    while index < XTERM_COLORS.len() {
        let distance = distance(color, XTERM_COLORS[index]);
        if distance < best_distance {
            best_index = index;
            best_distance = distance;
        }

        index += 1;
    }

    best_index
}

/// Low-cost approximation from <https://www.compuphase.com/cmetric.htm>, modified to avoid sqrt
pub(crate) const fn distance(c1: anstyle::RgbColor, c2: anstyle::RgbColor) -> u32 {
    let c1_r = c1.r() as i32;
    let c1_g = c1.g() as i32;
    let c1_b = c1.b() as i32;
    let c2_r = c2.r() as i32;
    let c2_g = c2.g() as i32;
    let c2_b = c2.b() as i32;

    let r_sum = c1_r + c2_r;
    let r_delta = c1_r - c2_r;
    let g_delta = c1_g - c2_g;
    let b_delta = c1_b - c2_b;

    let r = (2 * 512 + r_sum) * r_delta * r_delta;
    let g = 4 * g_delta * g_delta * (1 << 8);
    let b = (2 * 767 - r_sum) * b_delta * b_delta;

    (r + g + b) as u32
}

const XTERM_COLORS: [anstyle::RgbColor; 256] = [
    // Placeholders to make the index work.  See instead `palette` for these fields
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    Rgb(0, 0, 0),
    // 6x6x6 cube.  One each axis, the six indices map to [0, 95, 135, 175,
    // 215, 255] RGB component values.
    Rgb(0, 0, 0),
    Rgb(0, 0, 95),
    Rgb(0, 0, 135),
    Rgb(0, 0, 175),
    Rgb(0, 0, 215),
    Rgb(0, 0, 255),
    Rgb(0, 95, 0),
    Rgb(0, 95, 95),
    Rgb(0, 95, 135),
    Rgb(0, 95, 175),
    Rgb(0, 95, 215),
    Rgb(0, 95, 255),
    Rgb(0, 135, 0),
    Rgb(0, 135, 95),
    Rgb(0, 135, 135),
    Rgb(0, 135, 175),
    Rgb(0, 135, 215),
    Rgb(0, 135, 255),
    Rgb(0, 175, 0),
    Rgb(0, 175, 95),
    Rgb(0, 175, 135),
    Rgb(0, 175, 175),
    Rgb(0, 175, 215),
    Rgb(0, 175, 255),
    Rgb(0, 215, 0),
    Rgb(0, 215, 95),
    Rgb(0, 215, 135),
    Rgb(0, 215, 175),
    Rgb(0, 215, 215),
    Rgb(0, 215, 255),
    Rgb(0, 255, 0),
    Rgb(0, 255, 95),
    Rgb(0, 255, 135),
    Rgb(0, 255, 175),
    Rgb(0, 255, 215),
    Rgb(0, 255, 255),
    Rgb(95, 0, 0),
    Rgb(95, 0, 95),
    Rgb(95, 0, 135),
    Rgb(95, 0, 175),
    Rgb(95, 0, 215),
    Rgb(95, 0, 255),
    Rgb(95, 95, 0),
    Rgb(95, 95, 95),
    Rgb(95, 95, 135),
    Rgb(95, 95, 175),
    Rgb(95, 95, 215),
    Rgb(95, 95, 255),
    Rgb(95, 135, 0),
    Rgb(95, 135, 95),
    Rgb(95, 135, 135),
    Rgb(95, 135, 175),
    Rgb(95, 135, 215),
    Rgb(95, 135, 255),
    Rgb(95, 175, 0),
    Rgb(95, 175, 95),
    Rgb(95, 175, 135),
    Rgb(95, 175, 175),
    Rgb(95, 175, 215),
    Rgb(95, 175, 255),
    Rgb(95, 215, 0),
    Rgb(95, 215, 95),
    Rgb(95, 215, 135),
    Rgb(95, 215, 175),
    Rgb(95, 215, 215),
    Rgb(95, 215, 255),
    Rgb(95, 255, 0),
    Rgb(95, 255, 95),
    Rgb(95, 255, 135),
    Rgb(95, 255, 175),
    Rgb(95, 255, 215),
    Rgb(95, 255, 255),
    Rgb(135, 0, 0),
    Rgb(135, 0, 95),
    Rgb(135, 0, 135),
    Rgb(135, 0, 175),
    Rgb(135, 0, 215),
    Rgb(135, 0, 255),
    Rgb(135, 95, 0),
    Rgb(135, 95, 95),
    Rgb(135, 95, 135),
    Rgb(135, 95, 175),
    Rgb(135, 95, 215),
    Rgb(135, 95, 255),
    Rgb(135, 135, 0),
    Rgb(135, 135, 95),
    Rgb(135, 135, 135),
    Rgb(135, 135, 175),
    Rgb(135, 135, 215),
    Rgb(135, 135, 255),
    Rgb(135, 175, 0),
    Rgb(135, 175, 95),
    Rgb(135, 175, 135),
    Rgb(135, 175, 175),
    Rgb(135, 175, 215),
    Rgb(135, 175, 255),
    Rgb(135, 215, 0),
    Rgb(135, 215, 95),
    Rgb(135, 215, 135),
    Rgb(135, 215, 175),
    Rgb(135, 215, 215),
    Rgb(135, 215, 255),
    Rgb(135, 255, 0),
    Rgb(135, 255, 95),
    Rgb(135, 255, 135),
    Rgb(135, 255, 175),
    Rgb(135, 255, 215),
    Rgb(135, 255, 255),
    Rgb(175, 0, 0),
    Rgb(175, 0, 95),
    Rgb(175, 0, 135),
    Rgb(175, 0, 175),
    Rgb(175, 0, 215),
    Rgb(175, 0, 255),
    Rgb(175, 95, 0),
    Rgb(175, 95, 95),
    Rgb(175, 95, 135),
    Rgb(175, 95, 175),
    Rgb(175, 95, 215),
    Rgb(175, 95, 255),
    Rgb(175, 135, 0),
    Rgb(175, 135, 95),
    Rgb(175, 135, 135),
    Rgb(175, 135, 175),
    Rgb(175, 135, 215),
    Rgb(175, 135, 255),
    Rgb(175, 175, 0),
    Rgb(175, 175, 95),
    Rgb(175, 175, 135),
    Rgb(175, 175, 175),
    Rgb(175, 175, 215),
    Rgb(175, 175, 255),
    Rgb(175, 215, 0),
    Rgb(175, 215, 95),
    Rgb(175, 215, 135),
    Rgb(175, 215, 175),
    Rgb(175, 215, 215),
    Rgb(175, 215, 255),
    Rgb(175, 255, 0),
    Rgb(175, 255, 95),
    Rgb(175, 255, 135),
    Rgb(175, 255, 175),
    Rgb(175, 255, 215),
    Rgb(175, 255, 255),
    Rgb(215, 0, 0),
    Rgb(215, 0, 95),
    Rgb(215, 0, 135),
    Rgb(215, 0, 175),
    Rgb(215, 0, 215),
    Rgb(215, 0, 255),
    Rgb(215, 95, 0),
    Rgb(215, 95, 95),
    Rgb(215, 95, 135),
    Rgb(215, 95, 175),
    Rgb(215, 95, 215),
    Rgb(215, 95, 255),
    Rgb(215, 135, 0),
    Rgb(215, 135, 95),
    Rgb(215, 135, 135),
    Rgb(215, 135, 175),
    Rgb(215, 135, 215),
    Rgb(215, 135, 255),
    Rgb(215, 175, 0),
    Rgb(215, 175, 95),
    Rgb(215, 175, 135),
    Rgb(215, 175, 175),
    Rgb(215, 175, 215),
    Rgb(215, 175, 255),
    Rgb(215, 215, 0),
    Rgb(215, 215, 95),
    Rgb(215, 215, 135),
    Rgb(215, 215, 175),
    Rgb(215, 215, 215),
    Rgb(215, 215, 255),
    Rgb(215, 255, 0),
    Rgb(215, 255, 95),
    Rgb(215, 255, 135),
    Rgb(215, 255, 175),
    Rgb(215, 255, 215),
    Rgb(215, 255, 255),
    Rgb(255, 0, 0),
    Rgb(255, 0, 95),
    Rgb(255, 0, 135),
    Rgb(255, 0, 175),
    Rgb(255, 0, 215),
    Rgb(255, 0, 255),
    Rgb(255, 95, 0),
    Rgb(255, 95, 95),
    Rgb(255, 95, 135),
    Rgb(255, 95, 175),
    Rgb(255, 95, 215),
    Rgb(255, 95, 255),
    Rgb(255, 135, 0),
    Rgb(255, 135, 95),
    Rgb(255, 135, 135),
    Rgb(255, 135, 175),
    Rgb(255, 135, 215),
    Rgb(255, 135, 255),
    Rgb(255, 175, 0),
    Rgb(255, 175, 95),
    Rgb(255, 175, 135),
    Rgb(255, 175, 175),
    Rgb(255, 175, 215),
    Rgb(255, 175, 255),
    Rgb(255, 215, 0),
    Rgb(255, 215, 95),
    Rgb(255, 215, 135),
    Rgb(255, 215, 175),
    Rgb(255, 215, 215),
    Rgb(255, 215, 255),
    Rgb(255, 255, 0),
    Rgb(255, 255, 95),
    Rgb(255, 255, 135),
    Rgb(255, 255, 175),
    Rgb(255, 255, 215),
    Rgb(255, 255, 255),
    // 6x6x6 cube.  One each axis, the six indices map to [0, 95, 135, 175,
    // 215, 255] RGB component values.
    Rgb(8, 8, 8),
    Rgb(18, 18, 18),
    Rgb(28, 28, 28),
    Rgb(38, 38, 38),
    Rgb(48, 48, 48),
    Rgb(58, 58, 58),
    Rgb(68, 68, 68),
    Rgb(78, 78, 78),
    Rgb(88, 88, 88),
    Rgb(98, 98, 98),
    Rgb(108, 108, 108),
    Rgb(118, 118, 118),
    Rgb(128, 128, 128),
    Rgb(138, 138, 138),
    Rgb(148, 148, 148),
    Rgb(158, 158, 158),
    Rgb(168, 168, 168),
    Rgb(178, 178, 178),
    Rgb(188, 188, 188),
    Rgb(198, 198, 198),
    Rgb(208, 208, 208),
    Rgb(218, 218, 218),
    Rgb(228, 228, 228),
    Rgb(238, 238, 238),
];
