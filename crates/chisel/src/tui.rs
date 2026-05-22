//! Chisel TUI: an interactive terminal UI built on `foundry-tui` / `ratatui`.
//!
//! Layout (top-to-bottom):
//!
//! - 1-line title bar
//! - main area: scrollable output history (left) + live state pane (right)
//! - 3+ line input pane with syntax highlighting and inline error
//!
//! Falls back to the plain rustyline-based REPL when stdin/stdout is not a TTY (handled
//! by the caller in `args.rs`).

use crate::{
    args::chisel_history_file,
    dispatcher::ReplMsgKind,
    executor::snapshot_session_variables,
    prelude::{ChiselDispatcher, SolidityHelper},
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use foundry_common::block_on;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use rustyline::validate::ValidationResult;
use std::{io::BufRead, ops::ControlFlow, path::PathBuf};

/// One previously dispatched input together with its rendered output.
#[derive(Debug)]
struct HistoryEntry {
    /// Prompt prefix shown next to the input (e.g. `➜` or `(ID: foo) ➜`).
    prompt: String,
    /// The user-supplied input that was dispatched. May contain newlines for multi-line input.
    input: String,
    /// Output messages produced by the dispatch (kind + text), already ANSI-stripped.
    output: Vec<RenderedMessage>,
    /// Optional dispatch-error message (already ANSI-stripped).
    error: Option<String>,
}

/// A single rendered message line (kind + already-sanitized text).
#[derive(Debug)]
struct RenderedMessage {
    kind: ReplMsgKind,
    text: String,
}

/// A variable currently visible in the REPL session, with its type and last evaluated value.
#[derive(Debug)]
struct VariableEntry {
    name: String,
    type_str: String,
    value_str: String,
}

/// Chisel's interactive TUI app.
pub struct ChiselTuiApp {
    dispatcher: ChiselDispatcher,
    history: Vec<HistoryEntry>,
    output_scroll: u16,
    input: InputWidget,
    state: Vec<VariableEntry>,
    /// Inline error to show under the input (cleared on next submit or Ctrl+C).
    inline_error: Option<String>,
    /// Path to persist input history on quit.
    history_file: Option<PathBuf>,
}

impl ChiselTuiApp {
    /// Create a new TUI app wrapping `dispatcher`.
    pub fn new(dispatcher: ChiselDispatcher) -> Self {
        let history_file = chisel_history_file();
        let mut input = InputWidget::default();
        if let Some(path) = &history_file {
            input.load_history(path);
        }
        let mut app = Self {
            dispatcher,
            history: Vec::new(),
            output_scroll: 0,
            input,
            state: Vec::new(),
            inline_error: None,
            history_file,
        };
        app.refresh_state();
        app
    }

    /// Recomputes the variable state pane from the current session source via a single
    /// batched `abi.encode(v1, v2, ...)` execution.
    fn refresh_state(&mut self) {
        let snapshot = block_on(snapshot_session_variables(self.dispatcher.source()));
        // Discard any messages produced as a side-effect of the snapshot execution.
        let _ = self.dispatcher.drain_messages();
        self.state = snapshot
            .into_iter()
            .map(|(name, type_str, value_str)| VariableEntry {
                name,
                type_str,
                value_str: sanitize_output(&value_str),
            })
            .collect();
    }

    /// Submits the currently buffered input.
    fn submit(&mut self, line: String) {
        let prompt = self.dispatcher.get_prompt().to_string();
        self.inline_error = None;

        // Dispatch: `dispatcher.dispatch` is async; we drive it via block_on.
        let result = block_on(self.dispatcher.dispatch(&line));
        let messages: Vec<RenderedMessage> = self
            .dispatcher
            .drain_messages()
            .into_iter()
            .map(|m| RenderedMessage { kind: m.kind, text: sanitize_output(&m.text) })
            .collect();

        match result {
            Ok(ControlFlow::Continue(())) => {
                self.dispatcher.helper.set_errored(false);
                self.history.push(HistoryEntry {
                    prompt,
                    input: line,
                    output: messages,
                    error: None,
                });
                self.refresh_state();
            }
            Ok(ControlFlow::Break(())) => {
                self.history.push(HistoryEntry {
                    prompt,
                    input: line,
                    output: messages,
                    error: None,
                });
                self.input.quit_requested = true;
            }
            Err(e) => {
                self.dispatcher.helper.set_errored(true);
                let err_msg = sanitize_output(&foundry_common::errors::display_chain(&e));
                self.inline_error = Some(err_msg.clone());
                self.history.push(HistoryEntry {
                    prompt,
                    input: line,
                    output: messages,
                    error: Some(err_msg),
                });
            }
        }

        // Auto-scroll to bottom on new entry.
        self.output_scroll = u16::MAX;
    }

    fn draw_title(&self, frame: &mut Frame<'_>, area: Rect) {
        let id = self.dispatcher.id().unwrap_or("");
        let mut spans = vec![Span::styled(
            "⚒️  Chisel",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )];
        if !id.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(format!("[{id}]"), Style::default().fg(Color::Yellow)));
        }
        let line = Line::from(spans);
        let para = Paragraph::new(vec![line]).block(Block::default());
        frame.render_widget(para, area);

        // Render help on the right side of the title bar (within the same area).
        const HELP: &str = "  !help | Esc/Ctrl+D quit | Ctrl+L clear | PgUp/PgDn scroll";
        let help_w = HELP.chars().count() as u16;
        if area.width > help_w + 16 {
            let help_x = area.x + area.width.saturating_sub(help_w);
            let help_area = Rect { x: help_x, y: area.y, width: help_w, height: 1 };
            let help_para = Paragraph::new(Line::from(Span::styled(
                HELP,
                Style::default().fg(Color::DarkGray),
            )))
            .block(Block::default());
            frame.render_widget(help_para, help_area);
        }
    }

    fn draw_output(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for entry in &self.history {
            // Prompt + input (first line).
            let mut input_lines = entry.input.lines();
            let first = input_lines.next().unwrap_or("");
            lines.push(Line::from(vec![
                Span::styled(entry.prompt.clone(), Style::default().fg(Color::Green)),
                Span::raw(first.to_string()),
            ]));
            for cont in input_lines {
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::raw(cont.to_string()),
                ]));
            }
            for msg in &entry.output {
                let style = match msg.kind {
                    ReplMsgKind::Out => Style::default(),
                    ReplMsgKind::Err => Style::default().fg(Color::Red),
                };
                for line in msg.text.lines() {
                    lines.push(Line::from(Span::styled(line.to_string(), style)));
                }
            }
            if let Some(err) = &entry.error {
                for line in err.lines() {
                    lines.push(Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Red),
                    )));
                }
            }
            lines.push(Line::raw(""));
        }

        let total = lines.len() as u16;
        let inner_h = area.height.saturating_sub(2);
        let max_scroll = total.saturating_sub(inner_h);
        if self.output_scroll > max_scroll {
            self.output_scroll = max_scroll;
        }

        let block = Block::default().borders(Borders::ALL).title(" Output ");
        let para = Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((self.output_scroll, 0));
        frame.render_widget(para, area);
    }

    fn draw_state(&self, frame: &mut Frame<'_>, area: Rect) {
        let mut lines: Vec<Line<'static>> = Vec::new();
        if self.state.is_empty() {
            lines.push(Line::from(Span::styled(
                "(no variables)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for v in &self.state {
                lines.push(Line::from(vec![
                    Span::styled(
                        v.type_str.clone(),
                        Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(v.name.clone(), Style::default().fg(Color::White)),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("  = "),
                    Span::styled(v.value_str.clone(), Style::default().fg(Color::Yellow)),
                ]));
            }
        }
        let block = Block::default().borders(Borders::ALL).title(" Variables ");
        let para = Paragraph::new(Text::from(lines)).block(block).wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    }

    fn draw_input(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default().borders(Borders::ALL).title(" Input ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let prompt = self.dispatcher.get_prompt().to_string();
        let prompt_w = prompt.chars().count() as u16;
        let cont_prompt: String = " ".repeat(prompt.chars().count());

        // Input area is rendered without soft-wrap; instead we horizontally scroll the
        // current line so the cursor is always in view.
        let cur_chars: Vec<char> = self.input.buf.chars().collect();
        let cursor_char_idx = self.input.buf[..self.input.cursor].chars().count() as u16;
        let usable_w = inner.width.saturating_sub(prompt_w);
        // Recompute horizontal scroll to keep the cursor visible.
        if usable_w > 0 {
            if cursor_char_idx < self.input.h_scroll {
                self.input.h_scroll = cursor_char_idx;
            } else if cursor_char_idx >= self.input.h_scroll + usable_w {
                self.input.h_scroll = cursor_char_idx + 1 - usable_w;
            }
        } else {
            self.input.h_scroll = 0;
        }

        // Render pending lines (no syntax highlight, since they were already-balanced
        // continuation snippets). They still wrap visually if long; they don't host the cursor.
        let mut lines: Vec<Line<'static>> = Vec::new();
        for pending in &self.input.pending {
            let mut spans: Vec<Span<'static>> =
                vec![Span::styled(cont_prompt.clone(), Style::default().fg(Color::DarkGray))];
            for s in self.dispatcher.helper.highlight_ratatui(pending) {
                spans.push(Span::styled(s.content.into_owned(), s.style));
            }
            lines.push(Line::from(spans));
        }

        // Render the visible slice of the current line.
        let h_scroll = self.input.h_scroll as usize;
        let visible_chars: String = cur_chars.iter().skip(h_scroll).collect();
        let mut current_spans: Vec<Span<'static>> =
            vec![Span::styled(prompt, Style::default().fg(Color::Green))];
        for s in self.dispatcher.helper.highlight_ratatui(&visible_chars) {
            current_spans.push(Span::styled(s.content.into_owned(), s.style));
        }
        lines.push(Line::from(current_spans));

        if let Some(err) = &self.inline_error {
            for line in err.lines() {
                lines.push(Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::Red),
                )));
            }
        }

        let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
        frame.render_widget(para, inner);

        // Place the cursor on the current-line row, taking horizontal scroll into account.
        let pending_rows = self.input.pending.len() as u16;
        let cursor_y = inner.y + pending_rows;
        let cursor_x = inner.x + prompt_w + cursor_char_idx.saturating_sub(self.input.h_scroll);
        if cursor_y < inner.y + inner.height && cursor_x < inner.x + inner.width {
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

impl foundry_tui::TuiApp for ChiselTuiApp {
    type Exit = ();

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();

        let root = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(self.input_height()),
            ])
            .split(area);
        let title_area = root[0];
        let main_area = root[1];
        let input_area = root[2];

        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(main_area);

        self.draw_title(frame, title_area);
        self.draw_output(frame, main[0]);
        self.draw_state(frame, main[1]);
        self.draw_input(frame, input_area);
    }

    fn handle_event(&mut self, event: Event) -> ControlFlow<Self::Exit> {
        let key = match event {
            Event::Key(k) => k,
            // Bracketed paste: insert the pasted text into the input buffer, splitting
            // multi-line pastes into pending continuation lines.
            Event::Paste(s) => {
                self.input.insert_text(&s);
                return ControlFlow::Continue(());
            }
            _ => return ControlFlow::Continue(()),
        };
        // Only react to key-press events on platforms (Windows) that emit release events.
        if key.kind != KeyEventKind::Press {
            return ControlFlow::Continue(());
        }

        // Global keys.
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                if self.input.buf.is_empty() && self.input.pending.is_empty() {
                    return ControlFlow::Break(());
                }
                self.input.clear();
                self.inline_error = None;
                return ControlFlow::Continue(());
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) | (KeyCode::Esc, _)
                if self.input.buf.is_empty() && self.input.pending.is_empty() =>
            {
                return ControlFlow::Break(());
            }
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                self.history.clear();
                self.output_scroll = 0;
                return ControlFlow::Continue(());
            }
            (KeyCode::PageUp, _) => {
                self.output_scroll = self.output_scroll.saturating_sub(5);
                return ControlFlow::Continue(());
            }
            (KeyCode::PageDown, _) => {
                self.output_scroll = self.output_scroll.saturating_add(5);
                return ControlFlow::Continue(());
            }
            _ => {}
        }

        if let Some(submitted) = self.input.handle_key(key, &self.dispatcher.helper) {
            self.submit(submitted);
            if self.input.quit_requested {
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    }
}

impl ChiselTuiApp {
    fn input_height(&self) -> u16 {
        // borders (2) + pending lines + current line + optional error lines
        let mut h: u16 = 2 + self.input.pending.len() as u16 + 1;
        if let Some(err) = &self.inline_error {
            h += err.lines().count() as u16;
        }
        h.max(3)
    }
}

impl Drop for ChiselTuiApp {
    fn drop(&mut self) {
        if let Some(path) = &self.history_file {
            self.input.save_history(path);
        }
    }
}

/// A single-line text input with multi-line accumulation, history, and basic editing keys.
#[derive(Debug, Default)]
struct InputWidget {
    buf: String,
    /// Cursor as a byte offset into `buf`.
    cursor: usize,
    /// History of *submitted* inputs (each may contain `\n` for multi-line entries).
    history: Vec<String>,
    /// `Some(idx)` while the user is browsing history with Up/Down.
    history_idx: Option<usize>,
    /// Buffered prior lines for multi-line input. Joined with `\n` when finally submitted.
    pending: Vec<String>,
    /// Snapshot of the (current line + pending) state when the user entered history-browsing
    /// mode, restored when they leave it via Down past the latest entry.
    saved: Option<(Vec<String>, String)>,
    /// Set by the dispatcher when `!quit` runs, so the run loop can break.
    quit_requested: bool,
    /// Horizontal-scroll offset (in chars) of the current line, recomputed each frame to
    /// keep the cursor in view.
    h_scroll: u16,
}

impl InputWidget {
    fn clear(&mut self) {
        self.buf.clear();
        self.cursor = 0;
        self.pending.clear();
        self.history_idx = None;
        self.saved = None;
        self.h_scroll = 0;
    }

    /// Insert literal text into the buffer at the cursor. If the text contains newlines,
    /// each leading line is pushed into `pending` and the final line becomes the new buffer
    /// content.
    fn insert_text(&mut self, text: &str) {
        let mut iter = text.split('\n');
        let first = iter.next().unwrap_or("");
        self.buf.insert_str(self.cursor, first);
        self.cursor += first.len();
        for line in iter {
            // Move the (possibly partially typed) current buffer into `pending` and start
            // a fresh line for the next chunk.
            self.pending.push(std::mem::take(&mut self.buf));
            self.cursor = 0;
            self.buf.push_str(line);
            self.cursor = line.len();
        }
    }

    fn load_history(&mut self, path: &std::path::Path) {
        let Ok(file) = std::fs::File::open(path) else { return };
        let reader = std::io::BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            // History entries are stored one per line, with literal `\n` escaped as `\\n`
            // so multi-line entries can round-trip.
            let unescaped = unescape_newlines(&line);
            if !unescaped.trim().is_empty() {
                self.history.push(unescaped);
            }
        }
    }

    fn save_history(&self, path: &std::path::Path) {
        use std::io::Write as _;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(mut f) = std::fs::File::create(path) else { return };
        // Cap the saved history to the most recent 1000 entries.
        let start = self.history.len().saturating_sub(1000);
        for entry in &self.history[start..] {
            let _ = writeln!(f, "{}", escape_newlines(entry));
        }
    }

    /// Handles a key press. Returns `Some(joined_input)` when input is ready for dispatch.
    fn handle_key(&mut self, key: KeyEvent, helper: &SolidityHelper) -> Option<String> {
        match (key.code, key.modifiers) {
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                self.buf.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                None
            }
            (KeyCode::Backspace, _) => {
                if self.cursor > 0 {
                    let new_cursor = prev_char_boundary(&self.buf, self.cursor);
                    self.buf.drain(new_cursor..self.cursor);
                    self.cursor = new_cursor;
                } else if !self.pending.is_empty() {
                    // Pop the last pending continuation back onto the current line so the
                    // user can edit it.
                    let last = self.pending.pop().unwrap();
                    self.cursor = last.len();
                    let mut combined = last;
                    combined.push_str(&self.buf);
                    self.buf = combined;
                }
                None
            }
            (KeyCode::Delete, _) => {
                if self.cursor < self.buf.len() {
                    let next = next_char_boundary(&self.buf, self.cursor);
                    self.buf.drain(self.cursor..next);
                }
                None
            }
            (KeyCode::Left, _) => {
                if self.cursor > 0 {
                    self.cursor = prev_char_boundary(&self.buf, self.cursor);
                }
                None
            }
            (KeyCode::Right, _) => {
                if self.cursor < self.buf.len() {
                    self.cursor = next_char_boundary(&self.buf, self.cursor);
                }
                None
            }
            (KeyCode::Home, _) => {
                self.cursor = 0;
                None
            }
            (KeyCode::End, _) => {
                self.cursor = self.buf.len();
                None
            }
            (KeyCode::Up, _) => {
                if self.history.is_empty() {
                    return None;
                }
                let new_idx = match self.history_idx {
                    None => {
                        self.saved = Some((self.pending.clone(), self.buf.clone()));
                        self.history.len() - 1
                    }
                    Some(0) => 0,
                    Some(i) => i - 1,
                };
                self.history_idx = Some(new_idx);
                self.load_history_entry(new_idx);
                None
            }
            (KeyCode::Down, _) => {
                if let Some(idx) = self.history_idx {
                    if idx + 1 < self.history.len() {
                        self.history_idx = Some(idx + 1);
                        self.load_history_entry(idx + 1);
                    } else {
                        self.history_idx = None;
                        let (pending, buf) = self.saved.take().unwrap_or_default();
                        self.pending = pending;
                        self.buf = buf;
                        self.cursor = self.buf.len();
                    }
                }
                None
            }
            (KeyCode::Enter, _) => {
                // Combine pending lines + current line and check whether the snippet is
                // syntactically closed. If not, accumulate as a continuation line.
                let combined = if self.pending.is_empty() {
                    self.buf.clone()
                } else {
                    let mut s = self.pending.join("\n");
                    s.push('\n');
                    s.push_str(&self.buf);
                    s
                };
                if !is_balanced(helper, &combined) {
                    self.pending.push(std::mem::take(&mut self.buf));
                    self.cursor = 0;
                    self.history_idx = None;
                    self.saved = None;
                    return None;
                }
                // Don't submit blank input or pollute history with empties.
                if combined.trim().is_empty() {
                    self.clear();
                    return None;
                }
                let submitted = combined;
                // Avoid duplicating the most recent history entry.
                if self.history.last().map(String::as_str) != Some(submitted.as_str()) {
                    self.history.push(submitted.clone());
                }
                self.buf.clear();
                self.cursor = 0;
                self.pending.clear();
                self.history_idx = None;
                self.saved = None;
                self.h_scroll = 0;
                Some(submitted)
            }
            _ => None,
        }
    }

    /// Loads history entry `idx` into the editor, splitting embedded newlines into pending
    /// lines + a current line so the editor model stays consistent.
    fn load_history_entry(&mut self, idx: usize) {
        let entry = &self.history[idx];
        let mut parts: Vec<&str> = entry.split('\n').collect();
        let last = parts.pop().unwrap_or("").to_string();
        self.pending = parts.into_iter().map(str::to_string).collect();
        self.buf = last;
        self.cursor = self.buf.len();
        self.h_scroll = 0;
    }
}

const fn prev_char_boundary(s: &str, mut i: usize) -> usize {
    if i == 0 {
        return 0;
    }
    i -= 1;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

const fn next_char_boundary(s: &str, mut i: usize) -> usize {
    let len = s.len();
    if i >= len {
        return len;
    }
    i += 1;
    while i < len && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Returns `true` if `input` is syntactically closed (or invalid in a way that the dispatcher
/// can surface), meaning Enter should submit rather than continue on a new line.
fn is_balanced(helper: &SolidityHelper, input: &str) -> bool {
    !matches!(helper.validate_closed(input), ValidationResult::Incomplete)
}

/// Strips ANSI escape sequences from a string so that buffered terminal output (which includes
/// color codes generated by `yansi::Paint`) renders correctly inside ratatui.
fn sanitize_output(s: &str) -> String {
    // Simple ANSI CSI / SGR stripper: removes ESC [ ... <final byte> sequences.
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Consume `[`, then skip until a final byte in 0x40..=0x7E.
            if matches!(chars.peek(), Some(&'[')) {
                chars.next();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if matches!(c, '\x40'..='\x7e') {
                        break;
                    }
                }
                continue;
            }
            // Other escape sequences: drop the next char if any.
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// Escape `\n` as `\\n` and `\\` as `\\\\` so multi-line history entries can be saved one per
/// line.
fn escape_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Inverse of [`escape_newlines`].
fn unescape_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_roundtrip() {
        let inputs = ["", "abc", "a\nb\nc", "back\\slash", "mix\\n and \nreal newline"];
        for s in inputs {
            assert_eq!(unescape_newlines(&escape_newlines(s)), s, "input: {s:?}");
        }
    }

    #[test]
    fn insert_text_splits_pasted_newlines() {
        let mut w = InputWidget::default();
        w.insert_text("a\nb\nc");
        assert_eq!(w.pending, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(w.buf, "c");
        assert_eq!(w.cursor, 1);
    }
}
