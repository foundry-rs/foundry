use std::collections::HashMap;
use std::fmt::{self, Write};
use std::mem;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use console::{measure_text_width, Style};
#[cfg(feature = "unicode-segmentation")]
use unicode_segmentation::UnicodeSegmentation;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::draw_target::LineType;
use crate::format::{
    BinaryBytes, DecimalBytes, FormattedDuration, HumanBytes, HumanCount, HumanDuration,
    HumanFloatCount,
};
use crate::state::{ProgressState, TabExpandedString, DEFAULT_TAB_WIDTH};

#[derive(Clone)]
pub struct ProgressStyle {
    tick_strings: Vec<Box<str>>,
    progress_chars: Vec<Box<str>>,
    template: Template,
    // how unicode-big each char in progress_chars is
    char_width: usize,
    tab_width: usize,
    pub(crate) format_map: HashMap<&'static str, Box<dyn ProgressTracker>>,
}

#[cfg(feature = "unicode-segmentation")]
fn segment(s: &str) -> Vec<Box<str>> {
    UnicodeSegmentation::graphemes(s, true)
        .map(|s| s.into())
        .collect()
}

#[cfg(not(feature = "unicode-segmentation"))]
fn segment(s: &str) -> Vec<Box<str>> {
    s.chars().map(|x| x.to_string().into()).collect()
}

#[cfg(feature = "unicode-width")]
fn measure(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

#[cfg(not(feature = "unicode-width"))]
fn measure(s: &str) -> usize {
    s.chars().count()
}

/// finds the unicode-aware width of the passed grapheme cluters
/// panics on an empty parameter, or if the characters are not equal-width
fn width(c: &[Box<str>]) -> usize {
    c.iter()
        .map(|s| measure(s.as_ref()))
        .fold(None, |acc, new| {
            match acc {
                None => return Some(new),
                Some(old) => assert_eq!(old, new, "got passed un-equal width progress characters"),
            }
            acc
        })
        .unwrap()
}

impl ProgressStyle {
    /// Returns the default progress bar style for bars
    pub fn default_bar() -> Self {
        Self::new(Template::from_str("{wide_bar} {pos}/{len}").unwrap())
    }

    /// Returns the default progress bar style for spinners
    pub fn default_spinner() -> Self {
        Self::new(Template::from_str("{spinner} {msg}").unwrap())
    }

    /// Sets the template string for the progress bar
    ///
    /// Review the [list of template keys](../index.html#templates) for more information.
    pub fn with_template(template: &str) -> Result<Self, TemplateError> {
        Ok(Self::new(Template::from_str(template)?))
    }

    pub(crate) fn set_tab_width(&mut self, new_tab_width: usize) {
        self.tab_width = new_tab_width;
        self.template.set_tab_width(new_tab_width);
    }

    fn new(template: Template) -> Self {
        let progress_chars = segment("█░");
        let char_width = width(&progress_chars);
        Self {
            tick_strings: "⠁⠁⠉⠙⠚⠒⠂⠂⠒⠲⠴⠤⠄⠄⠤⠠⠠⠤⠦⠖⠒⠐⠐⠒⠓⠋⠉⠈⠈ "
                .chars()
                .map(|c| c.to_string().into())
                .collect(),
            progress_chars,
            char_width,
            template,
            format_map: HashMap::default(),
            tab_width: DEFAULT_TAB_WIDTH,
        }
    }

    /// Sets the tick character sequence for spinners
    ///
    /// Note that the last character is used as the [final tick string][Self::get_final_tick_str()].
    /// At least two characters are required to provide a non-final and final state.
    pub fn tick_chars(mut self, s: &str) -> Self {
        self.tick_strings = s.chars().map(|c| c.to_string().into()).collect();
        // Format bar will panic with some potentially confusing message, better to panic here
        // with a message explicitly informing of the problem
        assert!(
            self.tick_strings.len() >= 2,
            "at least 2 tick chars required"
        );
        self
    }

    /// Sets the tick string sequence for spinners
    ///
    /// Note that the last string is used as the [final tick string][Self::get_final_tick_str()].
    /// At least two strings are required to provide a non-final and final state.
    pub fn tick_strings(mut self, s: &[&str]) -> Self {
        self.tick_strings = s.iter().map(|s| s.to_string().into()).collect();
        // Format bar will panic with some potentially confusing message, better to panic here
        // with a message explicitly informing of the problem
        assert!(
            self.progress_chars.len() >= 2,
            "at least 2 tick strings required"
        );
        self
    }

    /// Sets the progress characters `(filled, current, to do)`
    ///
    /// You can pass more than three for a more detailed display.
    /// All passed grapheme clusters need to be of equal width.
    pub fn progress_chars(mut self, s: &str) -> Self {
        self.progress_chars = segment(s);
        // Format bar will panic with some potentially confusing message, better to panic here
        // with a message explicitly informing of the problem
        assert!(
            self.progress_chars.len() >= 2,
            "at least 2 progress chars required"
        );
        self.char_width = width(&self.progress_chars);
        self
    }

    /// Adds a custom key that owns a [`ProgressTracker`] to the template
    pub fn with_key<S: ProgressTracker + 'static>(mut self, key: &'static str, f: S) -> Self {
        self.format_map.insert(key, Box::new(f));
        self
    }

    /// Sets the template string for the progress bar
    ///
    /// Review the [list of template keys](../index.html#templates) for more information.
    pub fn template(mut self, s: &str) -> Result<Self, TemplateError> {
        self.template = Template::from_str(s)?;
        Ok(self)
    }

    fn current_tick_str(&self, state: &ProgressState) -> &str {
        match state.is_finished() {
            true => self.get_final_tick_str(),
            false => self.get_tick_str(state.tick),
        }
    }

    /// Returns the tick string for a given number
    pub fn get_tick_str(&self, idx: u64) -> &str {
        &self.tick_strings[(idx as usize) % (self.tick_strings.len() - 1)]
    }

    /// Returns the tick string for the finished state
    pub fn get_final_tick_str(&self) -> &str {
        &self.tick_strings[self.tick_strings.len() - 1]
    }

    fn format_bar(&self, fract: f32, width: usize, alt_style: Option<&Style>) -> BarDisplay<'_> {
        // The number of clusters from progress_chars to write (rounding down).
        let width = width / self.char_width;
        // The number of full clusters (including a fractional component for a partially-full one).
        let fill = fract * width as f32;
        // The number of entirely full clusters (by truncating `fill`).
        let entirely_filled = fill as usize;
        // 1 if the bar is not entirely empty or full (meaning we need to draw the "current"
        // character between the filled and "to do" segment), 0 otherwise.
        let head = usize::from(fill > 0.0 && entirely_filled < width);

        let cur = if head == 1 {
            // Number of fine-grained progress entries in progress_chars.
            let n = self.progress_chars.len().saturating_sub(2);
            let cur_char = if n <= 1 {
                // No fine-grained entries. 1 is the single "current" entry if we have one, the "to
                // do" entry if not.
                1
            } else {
                // Pick a fine-grained entry, ranging from the last one (n) if the fractional part
                // of fill is 0 to the first one (1) if the fractional part of fill is almost 1.
                n.saturating_sub((fill.fract() * n as f32) as usize)
            };
            Some(cur_char)
        } else {
            None
        };

        // Number of entirely empty clusters needed to fill the bar up to `width`.
        let bg = width.saturating_sub(entirely_filled).saturating_sub(head);
        let rest = RepeatedStringDisplay {
            str: &self.progress_chars[self.progress_chars.len() - 1],
            num: bg,
        };

        BarDisplay {
            chars: &self.progress_chars,
            filled: entirely_filled,
            cur,
            rest: alt_style.unwrap_or(&Style::new()).apply_to(rest),
        }
    }

    pub(crate) fn format_state(
        &self,
        state: &ProgressState,
        lines: &mut Vec<LineType>,
        target_width: u16,
    ) {
        let mut cur = String::new();
        let mut buf = String::new();
        let mut wide = None;

        let pos = state.pos();
        let len = state.len().unwrap_or(pos);
        for part in &self.template.parts {
            match part {
                TemplatePart::Placeholder {
                    key,
                    align,
                    width,
                    truncate,
                    style,
                    alt_style,
                } => {
                    buf.clear();
                    if let Some(tracker) = self.format_map.get(key.as_str()) {
                        tracker.write(state, &mut TabRewriter(&mut buf, self.tab_width));
                    } else {
                        match key.as_str() {
                            "wide_bar" => {
                                wide = Some(WideElement::Bar { alt_style });
                                buf.push('\x00');
                            }
                            "bar" => buf
                                .write_fmt(format_args!(
                                    "{}",
                                    self.format_bar(
                                        state.fraction(),
                                        width.unwrap_or(20) as usize,
                                        alt_style.as_ref(),
                                    )
                                ))
                                .unwrap(),
                            "spinner" => buf.push_str(self.current_tick_str(state)),
                            "wide_msg" => {
                                wide = Some(WideElement::Message { align });
                                buf.push('\x00');
                            }
                            "msg" => buf.push_str(state.message.expanded()),
                            "prefix" => buf.push_str(state.prefix.expanded()),
                            "pos" => buf.write_fmt(format_args!("{pos}")).unwrap(),
                            "human_pos" => {
                                buf.write_fmt(format_args!("{}", HumanCount(pos))).unwrap();
                            }
                            "len" => buf.write_fmt(format_args!("{len}")).unwrap(),
                            "human_len" => {
                                buf.write_fmt(format_args!("{}", HumanCount(len))).unwrap();
                            }
                            "percent" => buf
                                .write_fmt(format_args!("{:.*}", 0, state.fraction() * 100f32))
                                .unwrap(),
                            "percent_precise" => buf
                                .write_fmt(format_args!("{:.*}", 3, state.fraction() * 100f32))
                                .unwrap(),
                            "bytes" => buf.write_fmt(format_args!("{}", HumanBytes(pos))).unwrap(),
                            "total_bytes" => {
                                buf.write_fmt(format_args!("{}", HumanBytes(len))).unwrap();
                            }
                            "decimal_bytes" => buf
                                .write_fmt(format_args!("{}", DecimalBytes(pos)))
                                .unwrap(),
                            "decimal_total_bytes" => buf
                                .write_fmt(format_args!("{}", DecimalBytes(len)))
                                .unwrap(),
                            "binary_bytes" => {
                                buf.write_fmt(format_args!("{}", BinaryBytes(pos))).unwrap();
                            }
                            "binary_total_bytes" => {
                                buf.write_fmt(format_args!("{}", BinaryBytes(len))).unwrap();
                            }
                            "elapsed_precise" => buf
                                .write_fmt(format_args!("{}", FormattedDuration(state.elapsed())))
                                .unwrap(),
                            "elapsed" => buf
                                .write_fmt(format_args!("{:#}", HumanDuration(state.elapsed())))
                                .unwrap(),
                            "per_sec" => buf
                                .write_fmt(format_args!("{}/s", HumanFloatCount(state.per_sec())))
                                .unwrap(),
                            "bytes_per_sec" => buf
                                .write_fmt(format_args!("{}/s", HumanBytes(state.per_sec() as u64)))
                                .unwrap(),
                            "decimal_bytes_per_sec" => buf
                                .write_fmt(format_args!(
                                    "{}/s",
                                    DecimalBytes(state.per_sec() as u64)
                                ))
                                .unwrap(),
                            "binary_bytes_per_sec" => buf
                                .write_fmt(format_args!(
                                    "{}/s",
                                    BinaryBytes(state.per_sec() as u64)
                                ))
                                .unwrap(),
                            "eta_precise" => buf
                                .write_fmt(format_args!("{}", FormattedDuration(state.eta())))
                                .unwrap(),
                            "eta" => buf
                                .write_fmt(format_args!("{:#}", HumanDuration(state.eta())))
                                .unwrap(),
                            "duration_precise" => buf
                                .write_fmt(format_args!("{}", FormattedDuration(state.duration())))
                                .unwrap(),
                            "duration" => buf
                                .write_fmt(format_args!("{:#}", HumanDuration(state.duration())))
                                .unwrap(),
                            _ => (),
                        }
                    };

                    match width {
                        Some(width) => {
                            let padded = PaddedStringDisplay {
                                str: &buf,
                                width: *width as usize,
                                align: *align,
                                truncate: *truncate,
                            };
                            match style {
                                Some(s) => cur
                                    .write_fmt(format_args!("{}", s.apply_to(padded)))
                                    .unwrap(),
                                None => cur.write_fmt(format_args!("{padded}")).unwrap(),
                            }
                        }
                        None => match style {
                            Some(s) => cur.write_fmt(format_args!("{}", s.apply_to(&buf))).unwrap(),
                            None => cur.push_str(&buf),
                        },
                    }
                }
                TemplatePart::Literal(s) => cur.push_str(s.expanded()),
                TemplatePart::NewLine => {
                    self.push_line(lines, &mut cur, state, &mut buf, target_width, &wide);
                }
            }
        }

        if !cur.is_empty() {
            self.push_line(lines, &mut cur, state, &mut buf, target_width, &wide);
        }
    }

    /// This is used exclusively to add the bars built above to the lines to print
    fn push_line(
        &self,
        lines: &mut Vec<LineType>,
        cur: &mut String,
        state: &ProgressState,
        buf: &mut String,
        target_width: u16,
        wide: &Option<WideElement>,
    ) {
        let expanded = match wide {
            Some(inner) => inner.expand(mem::take(cur), self, state, buf, target_width),
            None => mem::take(cur),
        };

        // If there are newlines, we need to split them up
        // and add the lines separately so that they're counted
        // correctly on re-render.
        for (i, line) in expanded.split('\n').enumerate() {
            // No newlines found in this case
            if i == 0 && line.len() == expanded.len() {
                lines.push(LineType::Bar(expanded));
                break;
            }

            lines.push(LineType::Bar(line.to_string()));
        }
    }
}

struct TabRewriter<'a>(&'a mut dyn fmt::Write, usize);

impl Write for TabRewriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0
            .write_str(s.replace('\t', &" ".repeat(self.1)).as_str())
    }
}

#[derive(Clone, Copy)]
enum WideElement<'a> {
    Bar { alt_style: &'a Option<Style> },
    Message { align: &'a Alignment },
}

impl WideElement<'_> {
    fn expand(
        self,
        cur: String,
        style: &ProgressStyle,
        state: &ProgressState,
        buf: &mut String,
        width: u16,
    ) -> String {
        let left = (width as usize).saturating_sub(measure_text_width(&cur.replace('\x00', "")));
        match self {
            Self::Bar { alt_style } => cur.replace(
                '\x00',
                &format!(
                    "{}",
                    style.format_bar(state.fraction(), left, alt_style.as_ref())
                ),
            ),
            WideElement::Message { align } => {
                buf.clear();
                buf.write_fmt(format_args!(
                    "{}",
                    PaddedStringDisplay {
                        str: state.message.expanded(),
                        width: left,
                        align: *align,
                        truncate: true,
                    }
                ))
                .unwrap();

                let trimmed = match cur.as_bytes().last() == Some(&b'\x00') {
                    true => buf.trim_end(),
                    false => buf,
                };

                cur.replace('\x00', trimmed)
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Template {
    parts: Vec<TemplatePart>,
}

impl Template {
    fn from_str_with_tab_width(s: &str, tab_width: usize) -> Result<Self, TemplateError> {
        use State::*;
        let (mut state, mut parts, mut buf) = (Literal, vec![], String::new());
        for c in s.chars() {
            let new = match (state, c) {
                (Literal, '{') => (MaybeOpen, None),
                (Literal, '\n') => {
                    if !buf.is_empty() {
                        parts.push(TemplatePart::Literal(TabExpandedString::new(
                            mem::take(&mut buf).into(),
                            tab_width,
                        )));
                    }
                    parts.push(TemplatePart::NewLine);
                    (Literal, None)
                }
                (Literal, '}') => (DoubleClose, Some('}')),
                (Literal, c) => (Literal, Some(c)),
                (DoubleClose, '}') => (Literal, None),
                (MaybeOpen, '{') => (Literal, Some('{')),
                (MaybeOpen | Key, c) if c.is_ascii_whitespace() => {
                    // If we find whitespace where the variable key is supposed to go,
                    // backtrack and act as if this was a literal.
                    buf.push(c);
                    let mut new = String::from("{");
                    new.push_str(&buf);
                    buf.clear();
                    parts.push(TemplatePart::Literal(TabExpandedString::new(
                        new.into(),
                        tab_width,
                    )));
                    (Literal, None)
                }
                (MaybeOpen, c) if c != '}' && c != ':' => (Key, Some(c)),
                (Key, c) if c != '}' && c != ':' => (Key, Some(c)),
                (Key, ':') => (Align, None),
                (Key, '}') => (Literal, None),
                (Key, '!') if !buf.is_empty() => {
                    parts.push(TemplatePart::Placeholder {
                        key: mem::take(&mut buf),
                        align: Alignment::Left,
                        width: None,
                        truncate: true,
                        style: None,
                        alt_style: None,
                    });
                    (Width, None)
                }
                (Align, c) if c == '<' || c == '^' || c == '>' => {
                    if let Some(TemplatePart::Placeholder { align, .. }) = parts.last_mut() {
                        match c {
                            '<' => *align = Alignment::Left,
                            '^' => *align = Alignment::Center,
                            '>' => *align = Alignment::Right,
                            _ => (),
                        }
                    }

                    (Width, None)
                }
                (Align, c @ '0'..='9') => (Width, Some(c)),
                (Align | Width, '!') => {
                    if let Some(TemplatePart::Placeholder { truncate, .. }) = parts.last_mut() {
                        *truncate = true;
                    }
                    (Width, None)
                }
                (Align, '.') => (FirstStyle, None),
                (Align, '}') => (Literal, None),
                (Width, c @ '0'..='9') => (Width, Some(c)),
                (Width, '.') => (FirstStyle, None),
                (Width, '}') => (Literal, None),
                (FirstStyle, '/') => (AltStyle, None),
                (FirstStyle, '}') => (Literal, None),
                (FirstStyle, c) => (FirstStyle, Some(c)),
                (AltStyle, '}') => (Literal, None),
                (AltStyle, c) => (AltStyle, Some(c)),
                (st, c) => return Err(TemplateError { next: c, state: st }),
            };

            match (state, new.0) {
                (MaybeOpen, Key) if !buf.is_empty() => parts.push(TemplatePart::Literal(
                    TabExpandedString::new(mem::take(&mut buf).into(), tab_width),
                )),
                (Key, Align | Literal) if !buf.is_empty() => {
                    parts.push(TemplatePart::Placeholder {
                        key: mem::take(&mut buf),
                        align: Alignment::Left,
                        width: None,
                        truncate: false,
                        style: None,
                        alt_style: None,
                    });
                }
                (Width, FirstStyle | Literal) if !buf.is_empty() => {
                    if let Some(TemplatePart::Placeholder { width, .. }) = parts.last_mut() {
                        *width = Some(buf.parse().unwrap());
                        buf.clear();
                    }
                }
                (FirstStyle, AltStyle | Literal) if !buf.is_empty() => {
                    if let Some(TemplatePart::Placeholder { style, .. }) = parts.last_mut() {
                        *style = Some(Style::from_dotted_str(&buf));
                        buf.clear();
                    }
                }
                (AltStyle, Literal) if !buf.is_empty() => {
                    if let Some(TemplatePart::Placeholder { alt_style, .. }) = parts.last_mut() {
                        *alt_style = Some(Style::from_dotted_str(&buf));
                        buf.clear();
                    }
                }
                (_, _) => (),
            }

            state = new.0;
            if let Some(c) = new.1 {
                buf.push(c);
            }
        }

        if matches!(state, Literal | DoubleClose) && !buf.is_empty() {
            parts.push(TemplatePart::Literal(TabExpandedString::new(
                buf.into(),
                tab_width,
            )));
        }

        Ok(Self { parts })
    }

    fn from_str(s: &str) -> Result<Self, TemplateError> {
        Self::from_str_with_tab_width(s, DEFAULT_TAB_WIDTH)
    }

    fn set_tab_width(&mut self, new_tab_width: usize) {
        for part in &mut self.parts {
            if let TemplatePart::Literal(s) = part {
                s.set_tab_width(new_tab_width);
            }
        }
    }
}

#[derive(Debug)]
pub struct TemplateError {
    state: State,
    next: char,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TemplateError: unexpected character {:?} in state {:?}",
            self.next, self.state
        )
    }
}

impl std::error::Error for TemplateError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TemplatePart {
    Literal(TabExpandedString),
    Placeholder {
        key: String,
        align: Alignment,
        width: Option<u16>,
        truncate: bool,
        style: Option<Style>,
        alt_style: Option<Style>,
    },
    NewLine,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum State {
    Literal,
    MaybeOpen,
    DoubleClose,
    Key,
    Align,
    Width,
    FirstStyle,
    AltStyle,
}

struct BarDisplay<'a> {
    chars: &'a [Box<str>],
    filled: usize,
    cur: Option<usize>,
    rest: console::StyledObject<RepeatedStringDisplay<'a>>,
}

impl fmt::Display for BarDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for _ in 0..self.filled {
            f.write_str(&self.chars[0])?;
        }
        if let Some(cur) = self.cur {
            f.write_str(&self.chars[cur])?;
        }
        self.rest.fmt(f)
    }
}

struct RepeatedStringDisplay<'a> {
    str: &'a str,
    num: usize,
}

impl fmt::Display for RepeatedStringDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for _ in 0..self.num {
            f.write_str(self.str)?;
        }
        Ok(())
    }
}

struct PaddedStringDisplay<'a> {
    str: &'a str,
    width: usize,
    align: Alignment,
    truncate: bool,
}

impl fmt::Display for PaddedStringDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cols = measure_text_width(self.str);
        let excess = cols.saturating_sub(self.width);
        if excess > 0 && !self.truncate {
            return f.write_str(self.str);
        } else if excess > 0 {
            let (start, end) = match self.align {
                Alignment::Left => (0, self.str.len() - excess),
                Alignment::Right => (excess, self.str.len()),
                Alignment::Center => (
                    excess / 2,
                    self.str.len() - excess.saturating_sub(excess / 2),
                ),
            };

            return f.write_str(self.str.get(start..end).unwrap_or(self.str));
        }

        let diff = self.width.saturating_sub(cols);
        let (left_pad, right_pad) = match self.align {
            Alignment::Left => (0, diff),
            Alignment::Right => (diff, 0),
            Alignment::Center => (diff / 2, diff.saturating_sub(diff / 2)),
        };

        for _ in 0..left_pad {
            f.write_char(' ')?;
        }
        f.write_str(self.str)?;
        for _ in 0..right_pad {
            f.write_char(' ')?;
        }
        Ok(())
    }
}

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
enum Alignment {
    Left,
    Center,
    Right,
}

/// Trait for defining stateful or stateless formatters
pub trait ProgressTracker: Send + Sync {
    /// Creates a new instance of the progress tracker
    fn clone_box(&self) -> Box<dyn ProgressTracker>;
    /// Notifies the progress tracker of a tick event
    fn tick(&mut self, state: &ProgressState, now: Instant);
    /// Notifies the progress tracker of a reset event
    fn reset(&mut self, state: &ProgressState, now: Instant);
    /// Provides access to the progress bar display buffer for custom messages
    fn write(&self, state: &ProgressState, w: &mut dyn fmt::Write);
}

impl Clone for Box<dyn ProgressTracker> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl<F> ProgressTracker for F
where
    F: Fn(&ProgressState, &mut dyn fmt::Write) + Send + Sync + Clone + 'static,
{
    fn clone_box(&self) -> Box<dyn ProgressTracker> {
        Box::new(self.clone())
    }

    fn tick(&mut self, _: &ProgressState, _: Instant) {}

    fn reset(&mut self, _: &ProgressState, _: Instant) {}

    fn write(&self, state: &ProgressState, w: &mut dyn fmt::Write) {
        (self)(state, w);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::state::{AtomicPosition, ProgressState};

    use console::set_colors_enabled;
    use std::sync::Mutex;

    #[test]
    fn test_stateful_tracker() {
        #[derive(Debug, Clone)]
        struct TestTracker(Arc<Mutex<String>>);

        impl ProgressTracker for TestTracker {
            fn clone_box(&self) -> Box<dyn ProgressTracker> {
                Box::new(self.clone())
            }

            fn tick(&mut self, state: &ProgressState, _: Instant) {
                let mut m = self.0.lock().unwrap();
                m.clear();
                m.push_str(format!("{} {}", state.len().unwrap(), state.pos()).as_str());
            }

            fn reset(&mut self, _state: &ProgressState, _: Instant) {
                let mut m = self.0.lock().unwrap();
                m.clear();
            }

            fn write(&self, _state: &ProgressState, w: &mut dyn fmt::Write) {
                w.write_str(self.0.lock().unwrap().as_str()).unwrap();
            }
        }

        use crate::ProgressBar;

        let pb = ProgressBar::new(1);
        pb.set_style(
            ProgressStyle::with_template("{{ {foo} }}")
                .unwrap()
                .with_key("foo", TestTracker(Arc::new(Mutex::new(String::default()))))
                .progress_chars("#>-"),
        );

        let mut buf = Vec::new();
        let style = pb.clone().style();

        style.format_state(&pb.state().state, &mut buf, 16);
        assert_eq!(&buf[0], "{  }");
        buf.clear();
        pb.inc(1);
        style.format_state(&pb.state().state, &mut buf, 16);
        assert_eq!(&buf[0], "{ 1 1 }");
        pb.reset();
        buf.clear();
        style.format_state(&pb.state().state, &mut buf, 16);
        assert_eq!(&buf[0], "{  }");
        pb.finish_and_clear();
    }

    use crate::state::TabExpandedString;

    #[test]
    fn test_expand_template() {
        const WIDTH: u16 = 80;
        let pos = Arc::new(AtomicPosition::new());
        let state = ProgressState::new(Some(10), pos);
        let mut buf = Vec::new();

        let mut style = ProgressStyle::default_bar();
        style.format_map.insert(
            "foo",
            Box::new(|_: &ProgressState, w: &mut dyn Write| write!(w, "FOO").unwrap()),
        );
        style.format_map.insert(
            "bar",
            Box::new(|_: &ProgressState, w: &mut dyn Write| write!(w, "BAR").unwrap()),
        );

        style.template = Template::from_str("{{ {foo} {bar} }}").unwrap();
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], "{ FOO BAR }");

        buf.clear();
        style.template = Template::from_str(r#"{ "foo": "{foo}", "bar": {bar} }"#).unwrap();
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], r#"{ "foo": "FOO", "bar": BAR }"#);
    }

    #[test]
    fn test_expand_template_flags() {
        set_colors_enabled(true);

        const WIDTH: u16 = 80;
        let pos = Arc::new(AtomicPosition::new());
        let state = ProgressState::new(Some(10), pos);
        let mut buf = Vec::new();

        let mut style = ProgressStyle::default_bar();
        style.format_map.insert(
            "foo",
            Box::new(|_: &ProgressState, w: &mut dyn Write| write!(w, "XXX").unwrap()),
        );

        style.template = Template::from_str("{foo:5}").unwrap();
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], "XXX  ");

        buf.clear();
        style.template = Template::from_str("{foo:.red.on_blue}").unwrap();
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], "\u{1b}[31m\u{1b}[44mXXX\u{1b}[0m");

        buf.clear();
        style.template = Template::from_str("{foo:^5.red.on_blue}").unwrap();
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], "\u{1b}[31m\u{1b}[44m XXX \u{1b}[0m");

        buf.clear();
        style.template = Template::from_str("{foo:^5.red.on_blue/green.on_cyan}").unwrap();
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], "\u{1b}[31m\u{1b}[44m XXX \u{1b}[0m");
    }

    #[test]
    fn align_truncation() {
        const WIDTH: u16 = 10;
        let pos = Arc::new(AtomicPosition::new());
        let mut state = ProgressState::new(Some(10), pos);
        let mut buf = Vec::new();

        let style = ProgressStyle::with_template("{wide_msg}").unwrap();
        state.message = TabExpandedString::NoTabs("abcdefghijklmnopqrst".into());
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], "abcdefghij");

        buf.clear();
        let style = ProgressStyle::with_template("{wide_msg:>}").unwrap();
        state.message = TabExpandedString::NoTabs("abcdefghijklmnopqrst".into());
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], "klmnopqrst");

        buf.clear();
        let style = ProgressStyle::with_template("{wide_msg:^}").unwrap();
        state.message = TabExpandedString::NoTabs("abcdefghijklmnopqrst".into());
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], "fghijklmno");
    }

    #[test]
    fn wide_element_style() {
        set_colors_enabled(true);

        const CHARS: &str = "=>-";
        const WIDTH: u16 = 8;
        let pos = Arc::new(AtomicPosition::new());
        // half finished
        pos.set(2);
        let mut state = ProgressState::new(Some(4), pos);
        let mut buf = Vec::new();

        let style = ProgressStyle::with_template("{wide_bar}")
            .unwrap()
            .progress_chars(CHARS);
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], "====>---");

        buf.clear();
        let style = ProgressStyle::with_template("{wide_bar:.red.on_blue/green.on_cyan}")
            .unwrap()
            .progress_chars(CHARS);
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(
            &buf[0],
            "\u{1b}[31m\u{1b}[44m====>\u{1b}[32m\u{1b}[46m---\u{1b}[0m\u{1b}[0m"
        );

        buf.clear();
        let style = ProgressStyle::with_template("{wide_msg:^.red.on_blue}").unwrap();
        state.message = TabExpandedString::NoTabs("foobar".into());
        style.format_state(&state, &mut buf, WIDTH);
        assert_eq!(&buf[0], "\u{1b}[31m\u{1b}[44m foobar \u{1b}[0m");
    }

    #[test]
    fn multiline_handling() {
        const WIDTH: u16 = 80;
        let pos = Arc::new(AtomicPosition::new());
        let mut state = ProgressState::new(Some(10), pos);
        let mut buf = Vec::new();

        let mut style = ProgressStyle::default_bar();
        state.message = TabExpandedString::new("foo\nbar\nbaz".into(), 2);
        style.template = Template::from_str("{msg}").unwrap();
        style.format_state(&state, &mut buf, WIDTH);

        assert_eq!(buf.len(), 3);
        assert_eq!(&buf[0], "foo");
        assert_eq!(&buf[1], "bar");
        assert_eq!(&buf[2], "baz");

        buf.clear();
        style.template = Template::from_str("{wide_msg}").unwrap();
        style.format_state(&state, &mut buf, WIDTH);

        assert_eq!(buf.len(), 3);
        assert_eq!(&buf[0], "foo");
        assert_eq!(&buf[1], "bar");
        assert_eq!(&buf[2], "baz");

        buf.clear();
        state.prefix = TabExpandedString::new("prefix\nprefix".into(), 2);
        style.template = Template::from_str("{prefix} {wide_msg}").unwrap();
        style.format_state(&state, &mut buf, WIDTH);

        assert_eq!(buf.len(), 4);
        assert_eq!(&buf[0], "prefix");
        assert_eq!(&buf[1], "prefix foo");
        assert_eq!(&buf[2], "bar");
        assert_eq!(&buf[3], "baz");
    }
}
