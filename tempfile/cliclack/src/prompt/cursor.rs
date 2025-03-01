use std::fmt::{Display, Formatter, Result};

use zeroize::ZeroizeOnDrop;

/// A cursor for editing multiline strings.
///
/// Supports moving the cursor (left, right, up, down), backspace, delete, etc.
#[doc(hidden)]
#[derive(Default, ZeroizeOnDrop, Clone)]
pub struct StringCursor {
    value: Vec<char>,
    cursor: usize,
}

/// Returns the indices of the first character of each word in the given string,
/// as well as the indices of the start and end of the string. The returned
/// indices are sorted in ascending order.
fn word_jump_indices(value: &[char]) -> Vec<usize> {
    let mut indices = vec![0];
    let mut in_word = false;

    for (i, ch) in value.iter().enumerate() {
        if ch.is_whitespace() {
            in_word = false;
        } else if !in_word {
            indices.push(i);
            in_word = true;
        }
    }

    indices.push(value.len());

    indices
}

/// Returns the indices of the start of each line in the given string.
fn line_jump_indices(value: &[char]) -> Vec<usize> {
    value.split(|c| *c == '\n').fold(vec![0], |mut acc, line| {
        acc.push(acc.last().unwrap() + line.len() + 1);
        acc
    })
}

impl StringCursor {
    /// Returns `true` if the cursor contains no characters.
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Returns a character at the current cursor position.
    pub fn current(&self) -> Option<char> {
        self.value.get(self.cursor).copied()
    }

    /// Inserts a character at the current cursor position.
    pub fn insert(&mut self, chr: char) {
        self.value.insert(self.cursor, chr);
        self.cursor += 1;
    }

    /// Moves the cursor one position left.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Moves the cursor one position right.
    pub fn move_right(&mut self) {
        if self.cursor < self.value.len() {
            self.cursor += 1;
        }
    }

    /// Moves the cursor one position up considering multiline representation.
    pub fn move_up(&mut self) {
        let jumps = line_jump_indices(&self.value);
        self.cursor = match jumps.binary_search(&self.cursor) {
            Ok(ix) if ix + 1 < jumps.len() => {
                // Happened to be at the start of a line.
                let target_line = ix.saturating_sub(1);
                jumps[target_line]
            }
            Ok(ix) | Err(ix) => {
                let ix = ix.saturating_sub(1); // current line
                let target_line = ix.saturating_sub(1);
                let offset = std::cmp::min(
                    self.cursor - jumps[ix],
                    (jumps[ix] - jumps[target_line]).saturating_sub(1),
                );
                jumps[target_line] + offset
            }
        }
    }

    /// Moves the cursor one position down considering multiline representation.
    pub fn move_down(&mut self) {
        let jumps = line_jump_indices(&self.value);
        self.cursor = match jumps.binary_search(&self.cursor) {
            Ok(ix) if ix + 1 < jumps.len() => {
                // Happened to be at the start of a line.
                let target_line = std::cmp::min(ix + 1, jumps.len().saturating_sub(2));
                jumps[target_line]
            }
            Ok(ix) => {
                // Happened to be at the end of string.
                jumps[ix].saturating_sub(1)
            }
            Err(ix) => {
                let ix = ix.saturating_sub(1); // current line
                let target_line = std::cmp::min(ix + 1, jumps.len().saturating_sub(2));
                let target_next = std::cmp::min(target_line + 1, jumps.len().saturating_sub(1));
                let offset = std::cmp::min(
                    self.cursor - jumps[ix],
                    (jumps[target_next] - jumps[target_line]).saturating_sub(1),
                );
                jumps[target_line] + offset
            }
        }
    }

    /// Moves the cursor left by a word.
    pub fn move_left_by_word(&mut self) {
        let jumps = word_jump_indices(&self.value);
        let ix = jumps.binary_search(&self.cursor).unwrap_or_else(|i| i);
        self.cursor = jumps[ix.saturating_sub(1)];
    }

    /// Moves the cursor right by a word.
    pub fn move_right_by_word(&mut self) {
        let jumps = word_jump_indices(&self.value);
        let ix = jumps
            .binary_search(&self.cursor)
            .map_or_else(|i| i, |i| i + 1);
        self.cursor = jumps[std::cmp::min(ix, jumps.len().saturating_sub(1))];
    }

    /// Moves the cursor to the start of the line.
    pub fn move_home(&mut self) {
        let jumps = line_jump_indices(&self.value);
        self.cursor = match jumps.binary_search(&self.cursor) {
            Ok(ix) if ix + 1 < jumps.len() => self.cursor, // happened to be at the start of a line
            Ok(ix) | Err(ix) => jumps[ix.saturating_sub(1)],
        }
    }

    /// Moves the cursor to the end of the line.
    pub fn move_end(&mut self) {
        let jumps = line_jump_indices(&self.value);
        self.cursor = match jumps.binary_search(&self.cursor) {
            Ok(ix) if ix + 1 < jumps.len() => jumps[ix + 1].saturating_sub(1), // happened to be at the start of a line
            Ok(ix) | Err(ix) => jumps[ix].saturating_sub(1),
        }
    }

    /// Deletes the character to the left of the cursor.
    pub fn delete_left(&mut self) {
        if self.value.is_empty() {
            return;
        }

        if self.cursor > 0 {
            self.value.remove(self.cursor - 1);
            self.cursor -= 1;
        }
    }

    /// Deletes the character to the right of the cursor.
    pub fn delete_right(&mut self) {
        if self.value.is_empty() {
            return;
        }

        if self.cursor < self.value.len() {
            self.value.remove(self.cursor);
        }
    }

    /// Deletes the word to the left of the cursor.
    pub fn delete_word_to_the_left(&mut self) {
        if self.cursor > 0 {
            let jumps = word_jump_indices(&self.value);
            let ix = jumps.binary_search(&self.cursor).unwrap_or_else(|x| x);
            let start = jumps[std::cmp::max(ix - 1, 0)];
            let end = self.cursor;
            self.value.drain(start..end);
            self.cursor = start;
        }
    }

    /// Clears the cursor, removing all characters.
    pub fn clear(&mut self) {
        self.cursor = 0;
        self.value.clear()
    }

    /// Extends the cursor with the contents of a given string.
    pub fn extend(&mut self, string: &str) {
        self.value.extend(string.chars());
    }

    /// Splits the cursor into three parts: left, cursor, and right.
    pub fn split(&self) -> (String, String, String) {
        let left = String::from_iter(&self.value[..self.cursor]);
        let mut cursor = String::from(' ');
        let mut right = String::new();

        match self.current() {
            Some('\n') => right.push('\n'),
            Some(chr) => cursor = chr.to_string(),
            None => {}
        };

        if !self.value.is_empty() && self.cursor < self.value.len() - 1 {
            right.push_str(&String::from_iter(&self.value[self.cursor + 1..]));
        }

        (left, cursor, right)
    }

    /// Returns a mutable iterator over the characters in the cursor.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut char> {
        self.value.iter_mut()
    }
}

impl Display for StringCursor {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", String::from_iter(&self.value))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    macro_rules! assert_cursor {
        ($cursor: expr, $char: expr) => {
            assert_eq!($cursor.current().unwrap_or(' '), $char);
        };
    }

    macro_rules! assert_content {
        ($cursor: expr, $content: expr) => {
            assert_eq!($cursor.value, $content.chars().collect::<Vec<_>>());
        };
    }

    #[test]
    fn test_string_cursor() {
        let mut cursor = StringCursor {
            value: "hello\nworld".chars().collect(),
            cursor: 0,
        };
        assert_cursor!(cursor, 'h');
        assert_content!(cursor, "hello\nworld");
        cursor.move_right();
        assert_cursor!(cursor, 'e');
        cursor.move_up();
        assert_cursor!(cursor, 'h');
        cursor.move_up();
        assert_cursor!(cursor, 'h');
        cursor.move_down();
        assert_cursor!(cursor, 'w');
        cursor.move_down();
        assert_cursor!(cursor, 'w');
        cursor.move_end();
        assert_cursor!(cursor, ' ');
        cursor.move_up();
        assert_cursor!(cursor, '\n');
        for c in "\nbeautiful".chars() {
            cursor.insert(c);
        }
        assert_content!(cursor, "hello\nbeautiful\nworld");
        cursor.move_up();
        assert_cursor!(cursor, '\n');
        cursor.move_down();
        assert_cursor!(cursor, 'i');
        cursor.move_end();
        assert_cursor!(cursor, '\n');
        cursor.move_end();
        assert_cursor!(cursor, '\n');
        cursor.move_down();
        cursor.move_left();
        assert_cursor!(cursor, 'd');
        cursor.move_home();
        assert_cursor!(cursor, 'w');
    }
}
