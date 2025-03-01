//! Convert ANSI escape codes to SVG
//!
//! See [`Term`]
//!
//! # Example
//!
//! ```
//! # use anstyle_svg::Term;
//! let vte = std::fs::read_to_string("tests/rainbow.vte").unwrap();
//! let svg = Term::new().render_svg(&vte);
//! ```
//!
//! ![demo of supported styles](https://raw.githubusercontent.com/rust-cli/anstyle/main/crates/anstyle-svg/tests/rainbow.svg "Example output")

#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![warn(missing_docs)]
#![warn(clippy::print_stderr)]
#![warn(clippy::print_stdout)]

pub use anstyle_lossy::palette::Palette;
pub use anstyle_lossy::palette::VGA;
pub use anstyle_lossy::palette::WIN10_CONSOLE;

/// Define the terminal-like settings for rendering outpu
#[derive(Copy, Clone, Debug)]
pub struct Term {
    palette: Palette,
    fg_color: anstyle::Color,
    bg_color: anstyle::Color,
    background: bool,
    font_family: &'static str,
    min_width_px: usize,
    padding_px: usize,
}

impl Term {
    /// Default terminal settings
    pub const fn new() -> Self {
        Self {
            palette: VGA,
            fg_color: FG_COLOR,
            bg_color: BG_COLOR,
            background: true,
            font_family: "SFMono-Regular, Consolas, Liberation Mono, Menlo, monospace",
            min_width_px: 720,
            padding_px: 10,
        }
    }

    /// Select the color palette for [`anstyle::AnsiColor`]
    pub const fn palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }

    /// Select the default foreground color
    pub const fn fg_color(mut self, color: anstyle::Color) -> Self {
        self.fg_color = color;
        self
    }

    /// Select the default background color
    pub const fn bg_color(mut self, color: anstyle::Color) -> Self {
        self.bg_color = color;
        self
    }

    /// Toggle default background off with `false`
    pub const fn background(mut self, yes: bool) -> Self {
        self.background = yes;
        self
    }

    /// Minimum width for the text
    pub const fn min_width_px(mut self, px: usize) -> Self {
        self.min_width_px = px;
        self
    }

    /// Render the SVG with the terminal defined
    ///
    /// **Note:** Lines are not wrapped.  This is intentional as this attempts to convey the exact
    /// output with escape codes translated to SVG elements.
    pub fn render_svg(&self, ansi: &str) -> String {
        use std::fmt::Write as _;
        use unicode_width::UnicodeWidthStr as _;

        const FG: &str = "fg";
        const BG: &str = "bg";

        let mut styled = anstream::adapter::WinconBytes::new();
        let mut styled = styled.extract_next(ansi.as_bytes()).collect::<Vec<_>>();
        let mut effects_in_use = anstyle::Effects::new();
        for (style, _) in &mut styled {
            // Pre-process INVERT to make fg/bg calculations easier
            if style.get_effects().contains(anstyle::Effects::INVERT) {
                *style = style
                    .fg_color(Some(style.get_bg_color().unwrap_or(self.bg_color)))
                    .bg_color(Some(style.get_fg_color().unwrap_or(self.fg_color)))
                    .effects(style.get_effects().remove(anstyle::Effects::INVERT));
            }
            effects_in_use |= style.get_effects();
        }
        let styled_lines = split_lines(&styled);

        let fg_color = rgb_value(self.fg_color, self.palette);
        let bg_color = rgb_value(self.bg_color, self.palette);
        let font_family = self.font_family;

        let line_height = 18;
        let height = styled_lines.len() * line_height + self.padding_px * 2;
        let max_width = styled_lines
            .iter()
            .map(|l| l.iter().map(|(_, t)| t.width()).sum())
            .max()
            .unwrap_or(0);
        let width_px = (max_width as f64 * 8.4).ceil() as usize;
        let width_px = std::cmp::max(width_px, self.min_width_px) + self.padding_px * 2;

        let mut buffer = String::new();
        writeln!(
            &mut buffer,
            r#"<svg width="{width_px}px" height="{height}px" xmlns="http://www.w3.org/2000/svg">"#
        )
        .unwrap();
        writeln!(&mut buffer, r#"  <style>"#).unwrap();
        writeln!(&mut buffer, r#"    .{FG} {{ fill: {fg_color} }}"#).unwrap();
        writeln!(&mut buffer, r#"    .{BG} {{ background: {bg_color} }}"#).unwrap();
        for (name, rgb) in color_styles(&styled, self.palette) {
            if name.starts_with(FG_PREFIX) {
                writeln!(&mut buffer, r#"    .{name} {{ fill: {rgb} }}"#).unwrap();
            }
            if name.starts_with(BG_PREFIX) {
                writeln!(
                    &mut buffer,
                    r#"    .{name} {{ stroke: {rgb}; fill: {rgb}; user-select: none;  }}"#
                )
                .unwrap();
            }
            if name.starts_with(UNDERLINE_PREFIX) {
                writeln!(
                    &mut buffer,
                    r#"    .{name} {{ text-decoration-line: underline; text-decoration-color: {rgb} }}"#
                )
                .unwrap();
            }
        }
        writeln!(&mut buffer, r#"    .container {{"#).unwrap();
        writeln!(&mut buffer, r#"      padding: 0 10px;"#).unwrap();
        writeln!(&mut buffer, r#"      line-height: {line_height}px;"#).unwrap();
        writeln!(&mut buffer, r#"    }}"#).unwrap();
        if effects_in_use.contains(anstyle::Effects::BOLD) {
            writeln!(&mut buffer, r#"    .bold {{ font-weight: bold; }}"#).unwrap();
        }
        if effects_in_use.contains(anstyle::Effects::ITALIC) {
            writeln!(&mut buffer, r#"    .italic {{ font-style: italic; }}"#).unwrap();
        }
        if effects_in_use.contains(anstyle::Effects::UNDERLINE) {
            writeln!(
                &mut buffer,
                r#"    .underline {{ text-decoration-line: underline; }}"#
            )
            .unwrap();
        }
        if effects_in_use.contains(anstyle::Effects::DOUBLE_UNDERLINE) {
            writeln!(
                &mut buffer,
                r#"    .double-underline {{ text-decoration-line: underline; text-decoration-style: double; }}"#
            )
            .unwrap();
        }
        if effects_in_use.contains(anstyle::Effects::CURLY_UNDERLINE) {
            writeln!(
                &mut buffer,
                r#"    .curly-underline {{ text-decoration-line: underline; text-decoration-style: wavy; }}"#
            )
            .unwrap();
        }
        if effects_in_use.contains(anstyle::Effects::DOTTED_UNDERLINE) {
            writeln!(
                &mut buffer,
                r#"    .dotted-underline {{ text-decoration-line: underline; text-decoration-style: dotted; }}"#
            )
            .unwrap();
        }
        if effects_in_use.contains(anstyle::Effects::DASHED_UNDERLINE) {
            writeln!(
                &mut buffer,
                r#"    .dashed-underline {{ text-decoration-line: underline; text-decoration-style: dashed; }}"#
            )
            .unwrap();
        }
        if effects_in_use.contains(anstyle::Effects::STRIKETHROUGH) {
            writeln!(
                &mut buffer,
                r#"    .strikethrough {{ text-decoration-line: line-through; }}"#
            )
            .unwrap();
        }
        if effects_in_use.contains(anstyle::Effects::DIMMED) {
            writeln!(&mut buffer, r#"    .dimmed {{ opacity: 0.7; }}"#).unwrap();
        }
        if effects_in_use.contains(anstyle::Effects::HIDDEN) {
            writeln!(&mut buffer, r#"    .hidden {{ opacity: 0; }}"#).unwrap();
        }
        writeln!(&mut buffer, r#"    tspan {{"#).unwrap();
        writeln!(&mut buffer, r#"      font: 14px {font_family};"#).unwrap();
        writeln!(&mut buffer, r#"      white-space: pre;"#).unwrap();
        writeln!(&mut buffer, r#"      line-height: {line_height}px;"#).unwrap();
        writeln!(&mut buffer, r#"    }}"#).unwrap();
        writeln!(&mut buffer, r#"  </style>"#).unwrap();
        writeln!(&mut buffer).unwrap();

        if self.background {
            writeln!(
                &mut buffer,
                r#"  <rect width="100%" height="100%" y="0" rx="4.5" class="{BG}" />"#
            )
            .unwrap();
            writeln!(&mut buffer).unwrap();
        }

        let text_x = self.padding_px;
        let mut text_y = self.padding_px + line_height;
        writeln!(
            &mut buffer,
            r#"  <text xml:space="preserve" class="container {FG}">"#
        )
        .unwrap();
        for line in &styled_lines {
            if line.iter().any(|(s, _)| s.get_bg_color().is_some()) {
                write!(&mut buffer, r#"    <tspan x="{text_x}px" y="{text_y}px">"#).unwrap();
                for (style, fragment) in line {
                    if fragment.is_empty() {
                        continue;
                    }
                    write_bg_span(&mut buffer, style, fragment);
                }
                // HACK: must close tspan on newline to include them in copy/paste
                writeln!(&mut buffer).unwrap();
                writeln!(&mut buffer, r#"</tspan>"#).unwrap();
            }

            write!(&mut buffer, r#"    <tspan x="{text_x}px" y="{text_y}px">"#).unwrap();
            for (style, fragment) in line {
                if fragment.is_empty() {
                    continue;
                }
                write_fg_span(&mut buffer, style, fragment);
            }
            // HACK: must close tspan on newline to include them in copy/paste
            writeln!(&mut buffer).unwrap();
            writeln!(&mut buffer, r#"</tspan>"#).unwrap();

            text_y += line_height;
        }
        writeln!(&mut buffer, r#"  </text>"#).unwrap();
        writeln!(&mut buffer).unwrap();

        writeln!(&mut buffer, r#"</svg>"#).unwrap();
        buffer
    }
}

const FG_COLOR: anstyle::Color = anstyle::Color::Ansi(anstyle::AnsiColor::White);
const BG_COLOR: anstyle::Color = anstyle::Color::Ansi(anstyle::AnsiColor::Black);

fn write_fg_span(buffer: &mut String, style: &anstyle::Style, fragment: &str) {
    use std::fmt::Write as _;
    let fg_color = style.get_fg_color().map(|c| color_name(FG_PREFIX, c));
    let underline_color = style
        .get_underline_color()
        .map(|c| color_name(UNDERLINE_PREFIX, c));
    let effects = style.get_effects();
    let underline = effects.contains(anstyle::Effects::UNDERLINE);
    let double_underline = effects.contains(anstyle::Effects::DOUBLE_UNDERLINE);
    let curly_underline = effects.contains(anstyle::Effects::CURLY_UNDERLINE);
    let dotted_underline = effects.contains(anstyle::Effects::DOTTED_UNDERLINE);
    let dashed_underline = effects.contains(anstyle::Effects::DASHED_UNDERLINE);
    let strikethrough = effects.contains(anstyle::Effects::STRIKETHROUGH);
    // skipping INVERT as that was handled earlier
    let bold = effects.contains(anstyle::Effects::BOLD);
    let italic = effects.contains(anstyle::Effects::ITALIC);
    let dimmed = effects.contains(anstyle::Effects::DIMMED);
    let hidden = effects.contains(anstyle::Effects::HIDDEN);

    let fragment = html_escape::encode_text(fragment);
    let mut classes = Vec::new();
    if let Some(class) = fg_color.as_deref() {
        classes.push(class);
    }
    if let Some(class) = underline_color.as_deref() {
        classes.push(class);
    }
    if underline {
        classes.push("underline");
    }
    if double_underline {
        classes.push("double-underline");
    }
    if curly_underline {
        classes.push("curly-underline");
    }
    if dotted_underline {
        classes.push("dotted-underline");
    }
    if dashed_underline {
        classes.push("dashed-underline");
    }
    if strikethrough {
        classes.push("strikethrough");
    }
    if bold {
        classes.push("bold");
    }
    if italic {
        classes.push("italic");
    }
    if dimmed {
        classes.push("dimmed");
    }
    if hidden {
        classes.push("hidden");
    }

    write!(buffer, r#"<tspan"#).unwrap();
    if !classes.is_empty() {
        let classes = classes.join(" ");
        write!(buffer, r#" class="{classes}""#).unwrap();
    }
    write!(buffer, r#">"#).unwrap();
    write!(buffer, "{fragment}").unwrap();
    write!(buffer, r#"</tspan>"#).unwrap();
}

fn write_bg_span(buffer: &mut String, style: &anstyle::Style, fragment: &str) {
    use std::fmt::Write as _;
    use unicode_width::UnicodeWidthStr;

    let bg_color = style.get_bg_color().map(|c| color_name(BG_PREFIX, c));

    let fill = if bg_color.is_some() { "█" } else { " " };

    let fragment = html_escape::encode_text(fragment);
    let width = fragment.width();
    let fragment = fill.repeat(width);
    let mut classes = Vec::new();
    if let Some(class) = bg_color.as_deref() {
        classes.push(class);
    }
    write!(buffer, r#"<tspan"#).unwrap();
    if !classes.is_empty() {
        let classes = classes.join(" ");
        write!(buffer, r#" class="{classes}""#).unwrap();
    }
    write!(buffer, r#">"#).unwrap();
    write!(buffer, "{fragment}").unwrap();
    write!(buffer, r#"</tspan>"#).unwrap();
}

impl Default for Term {
    fn default() -> Self {
        Self::new()
    }
}

const ANSI_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright-black",
    "bright-red",
    "bright-green",
    "bright-yellow",
    "bright-blue",
    "bright-magenta",
    "bright-cyan",
    "bright-white",
];

fn rgb_value(color: anstyle::Color, palette: Palette) -> String {
    let color = anstyle_lossy::color_to_rgb(color, palette);
    let anstyle::RgbColor(r, g, b) = color;
    format!("#{r:02X}{g:02X}{b:02X}")
}

const FG_PREFIX: &str = "fg";
const BG_PREFIX: &str = "bg";
const UNDERLINE_PREFIX: &str = "underline";

fn color_name(prefix: &str, color: anstyle::Color) -> String {
    match color {
        anstyle::Color::Ansi(color) => {
            let color = anstyle::Ansi256Color::from_ansi(color);
            let index = color.index() as usize;
            let name = ANSI_NAMES[index];
            format!("{prefix}-{name}")
        }
        anstyle::Color::Ansi256(color) => {
            let index = color.index();
            format!("{prefix}-ansi256-{index:03}")
        }
        anstyle::Color::Rgb(color) => {
            let anstyle::RgbColor(r, g, b) = color;
            format!("{prefix}-rgb-{r:02X}{g:02X}{b:02X}")
        }
    }
}

fn color_styles(
    styled: &[(anstyle::Style, String)],
    palette: Palette,
) -> impl Iterator<Item = (String, String)> {
    let mut colors = std::collections::BTreeMap::new();
    for (style, _) in styled {
        if let Some(color) = style.get_fg_color() {
            colors.insert(color_name(FG_PREFIX, color), rgb_value(color, palette));
        }
        if let Some(color) = style.get_bg_color() {
            colors.insert(color_name(BG_PREFIX, color), rgb_value(color, palette));
        }
        if let Some(color) = style.get_underline_color() {
            colors.insert(
                color_name(UNDERLINE_PREFIX, color),
                rgb_value(color, palette),
            );
        }
    }

    colors.into_iter()
}

fn split_lines(styled: &[(anstyle::Style, String)]) -> Vec<Vec<(anstyle::Style, &str)>> {
    let mut lines = Vec::new();
    let mut current_line = Vec::new();
    for (style, mut next) in styled.iter().map(|(s, t)| (*s, t.as_str())) {
        while let Some((current, remaining)) = next.split_once('\n') {
            let current = current.strip_suffix('\r').unwrap_or(current);
            current_line.push((style, current));
            lines.push(current_line);
            current_line = Vec::new();
            next = remaining;
        }
        current_line.push((style, next));
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }
    lines
}
