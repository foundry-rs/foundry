use std::fmt::{Debug, Formatter, Write as _};
use std::io::Write as _;
use std::sync::{Arc, Mutex};

use vt100::Parser;

use crate::TermLike;

/// A thin wrapper around [`vt100::Parser`].
///
/// This is just an [`Arc`] around its internal state, so it can be freely cloned.
#[cfg_attr(docsrs, doc(cfg(feature = "in_memory")))]
#[derive(Debug, Clone)]
pub struct InMemoryTerm {
    state: Arc<Mutex<InMemoryTermState>>,
}

impl InMemoryTerm {
    pub fn new(rows: u16, cols: u16) -> InMemoryTerm {
        assert!(rows > 0, "rows must be > 0");
        assert!(cols > 0, "cols must be > 0");
        InMemoryTerm {
            state: Arc::new(Mutex::new(InMemoryTermState::new(rows, cols))),
        }
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        *state = InMemoryTermState::new(state.height, state.width);
    }

    pub fn contents(&self) -> String {
        let state = self.state.lock().unwrap();

        // For some reason, the `Screen::contents` method doesn't include newlines in what it
        // returns, making it useless for our purposes. So we need to manually reconstruct the
        // contents by iterating over the rows in the terminal buffer.
        let mut rows = state
            .parser
            .screen()
            .rows(0, state.width)
            .collect::<Vec<_>>();

        // Reverse the rows and trim empty lines from the end
        rows = rows
            .into_iter()
            .rev()
            .skip_while(|line| line.is_empty())
            .map(|line| line.trim_end().to_string())
            .collect();

        // Un-reverse the rows and join them up with newlines
        rows.reverse();
        rows.join("\n")
    }

    pub fn contents_formatted(&self) -> Vec<u8> {
        let state = self.state.lock().unwrap();

        // For some reason, the `Screen::contents` method doesn't include newlines in what it
        // returns, making it useless for our purposes. So we need to manually reconstruct the
        // contents by iterating over the rows in the terminal buffer.
        let mut rows = state
            .parser
            .screen()
            .rows_formatted(0, state.width)
            .collect::<Vec<_>>();

        // Reverse the rows and trim empty lines from the end
        rows = rows
            .into_iter()
            .rev()
            .skip_while(|line| line.is_empty())
            .collect();

        // Un-reverse the rows
        rows.reverse();

        // Calculate buffer size
        let reset = b"[m";
        let len = rows.iter().map(|line| line.len() + reset.len() + 1).sum();

        // Join rows up with reset codes and newlines
        let mut contents = rows.iter().fold(Vec::with_capacity(len), |mut acc, cur| {
            acc.extend_from_slice(cur);
            acc.extend_from_slice(reset);
            acc.push(b'\n');
            acc
        });

        // Remove last newline again, but leave the reset code
        contents.truncate(len.saturating_sub(1));
        contents
    }

    pub fn moves_since_last_check(&self) -> String {
        let mut s = String::new();
        for line in std::mem::take(&mut self.state.lock().unwrap().history) {
            writeln!(s, "{line:?}").unwrap();
        }
        s
    }
}

impl TermLike for InMemoryTerm {
    fn width(&self) -> u16 {
        self.state.lock().unwrap().width
    }

    fn height(&self) -> u16 {
        self.state.lock().unwrap().height
    }

    fn move_cursor_up(&self, n: usize) -> std::io::Result<()> {
        match n {
            0 => Ok(()),
            _ => {
                let mut state = self.state.lock().unwrap();
                state.history.push(Move::Up(n));
                state.write_str(&format!("\x1b[{n}A"))
            }
        }
    }

    fn move_cursor_down(&self, n: usize) -> std::io::Result<()> {
        match n {
            0 => Ok(()),
            _ => {
                let mut state = self.state.lock().unwrap();
                state.history.push(Move::Down(n));
                state.write_str(&format!("\x1b[{n}B"))
            }
        }
    }

    fn move_cursor_right(&self, n: usize) -> std::io::Result<()> {
        match n {
            0 => Ok(()),
            _ => {
                let mut state = self.state.lock().unwrap();
                state.history.push(Move::Right(n));
                state.write_str(&format!("\x1b[{n}C"))
            }
        }
    }

    fn move_cursor_left(&self, n: usize) -> std::io::Result<()> {
        match n {
            0 => Ok(()),
            _ => {
                let mut state = self.state.lock().unwrap();
                state.history.push(Move::Left(n));
                state.write_str(&format!("\x1b[{n}D"))
            }
        }
    }

    fn write_line(&self, s: &str) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        state.history.push(Move::Str(s.into()));
        state.history.push(Move::NewLine);

        // Don't try to handle writing lines with additional newlines embedded in them - it's not
        // worth the extra code for something that indicatif doesn't even do. May revisit in future.
        debug_assert!(
            s.lines().count() <= 1,
            "calling write_line with embedded newlines is not allowed"
        );

        // vte100 needs the full \r\n sequence to jump to the next line and reset the cursor to
        // the beginning of the line. Be flexible and take either \n or \r\n
        state.write_str(s)?;
        state.write_str("\r\n")
    }

    fn write_str(&self, s: &str) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        state.history.push(Move::Str(s.into()));
        state.write_str(s)
    }

    fn clear_line(&self) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        state.history.push(Move::Clear);
        state.write_str("\r\x1b[2K")
    }

    fn flush(&self) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        state.history.push(Move::Flush);
        state.parser.flush()
    }
}

struct InMemoryTermState {
    width: u16,
    height: u16,
    parser: vt100::Parser,
    history: Vec<Move>,
}

impl InMemoryTermState {
    pub(crate) fn new(rows: u16, cols: u16) -> InMemoryTermState {
        InMemoryTermState {
            width: cols,
            height: rows,
            parser: Parser::new(rows, cols, 0),
            history: vec![],
        }
    }

    pub(crate) fn write_str(&mut self, s: &str) -> std::io::Result<()> {
        self.parser.write_all(s.as_bytes())
    }
}

impl Debug for InMemoryTermState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryTermState").finish_non_exhaustive()
    }
}

#[derive(Debug, PartialEq, Clone)]
enum Move {
    Up(usize),
    Down(usize),
    Left(usize),
    Right(usize),
    Str(String),
    NewLine,
    Clear,
    Flush,
}

#[cfg(test)]
mod test {
    use super::*;

    fn cursor_pos(in_mem: &InMemoryTerm) -> (u16, u16) {
        in_mem
            .state
            .lock()
            .unwrap()
            .parser
            .screen()
            .cursor_position()
    }

    #[test]
    fn line_wrapping() {
        let in_mem = InMemoryTerm::new(10, 5);
        assert_eq!(cursor_pos(&in_mem), (0, 0));

        in_mem.write_str("ABCDE").unwrap();
        assert_eq!(in_mem.contents(), "ABCDE");
        assert_eq!(cursor_pos(&in_mem), (0, 5));
        assert_eq!(
            in_mem.moves_since_last_check(),
            r#"Str("ABCDE")
"#
        );

        // Should wrap onto next line
        in_mem.write_str("FG").unwrap();
        assert_eq!(in_mem.contents(), "ABCDE\nFG");
        assert_eq!(cursor_pos(&in_mem), (1, 2));
        assert_eq!(
            in_mem.moves_since_last_check(),
            r#"Str("FG")
"#
        );

        in_mem.write_str("HIJ").unwrap();
        assert_eq!(in_mem.contents(), "ABCDE\nFGHIJ");
        assert_eq!(cursor_pos(&in_mem), (1, 5));
        assert_eq!(
            in_mem.moves_since_last_check(),
            r#"Str("HIJ")
"#
        );
    }

    #[test]
    fn write_line() {
        let in_mem = InMemoryTerm::new(10, 5);
        assert_eq!(cursor_pos(&in_mem), (0, 0));

        in_mem.write_line("A").unwrap();
        assert_eq!(in_mem.contents(), "A");
        assert_eq!(cursor_pos(&in_mem), (1, 0));
        assert_eq!(
            in_mem.moves_since_last_check(),
            r#"Str("A")
NewLine
"#
        );

        in_mem.write_line("B").unwrap();
        assert_eq!(in_mem.contents(), "A\nB");
        assert_eq!(cursor_pos(&in_mem), (2, 0));
        assert_eq!(
            in_mem.moves_since_last_check(),
            r#"Str("B")
NewLine
"#
        );

        in_mem.write_line("Longer than cols").unwrap();
        assert_eq!(in_mem.contents(), "A\nB\nLonge\nr tha\nn col\ns");
        assert_eq!(cursor_pos(&in_mem), (6, 0));
        assert_eq!(
            in_mem.moves_since_last_check(),
            r#"Str("Longer than cols")
NewLine
"#
        );
    }

    #[test]
    fn basic_functionality() {
        let in_mem = InMemoryTerm::new(10, 80);

        in_mem.write_line("This is a test line").unwrap();
        assert_eq!(in_mem.contents(), "This is a test line");
        assert_eq!(
            in_mem.moves_since_last_check(),
            r#"Str("This is a test line")
NewLine
"#
        );

        in_mem.write_line("And another line!").unwrap();
        assert_eq!(in_mem.contents(), "This is a test line\nAnd another line!");
        assert_eq!(
            in_mem.moves_since_last_check(),
            r#"Str("And another line!")
NewLine
"#
        );

        in_mem.move_cursor_up(1).unwrap();
        in_mem.write_str("TEST").unwrap();

        assert_eq!(in_mem.contents(), "This is a test line\nTESTanother line!");
        assert_eq!(
            in_mem.moves_since_last_check(),
            r#"Up(1)
Str("TEST")
"#
        );
    }

    #[test]
    fn newlines() {
        let in_mem = InMemoryTerm::new(10, 10);
        in_mem.write_line("LINE ONE").unwrap();
        in_mem.write_line("LINE TWO").unwrap();
        in_mem.write_line("").unwrap();
        in_mem.write_line("LINE FOUR").unwrap();

        assert_eq!(in_mem.contents(), "LINE ONE\nLINE TWO\n\nLINE FOUR");

        assert_eq!(
            in_mem.moves_since_last_check(),
            r#"Str("LINE ONE")
NewLine
Str("LINE TWO")
NewLine
Str("")
NewLine
Str("LINE FOUR")
NewLine
"#
        );
    }

    #[test]
    fn cursor_zero_movement() {
        let in_mem = InMemoryTerm::new(10, 80);
        in_mem.write_line("LINE ONE").unwrap();
        assert_eq!(cursor_pos(&in_mem), (1, 0));

        // Check that moving zero rows/cols does not actually move cursor
        in_mem.move_cursor_up(0).unwrap();
        assert_eq!(cursor_pos(&in_mem), (1, 0));

        in_mem.move_cursor_down(0).unwrap();
        assert_eq!(cursor_pos(&in_mem), (1, 0));

        in_mem.move_cursor_right(1).unwrap();
        assert_eq!(cursor_pos(&in_mem), (1, 1));

        in_mem.move_cursor_left(0).unwrap();
        assert_eq!(cursor_pos(&in_mem), (1, 1));

        in_mem.move_cursor_right(0).unwrap();
        assert_eq!(cursor_pos(&in_mem), (1, 1));
    }
}
