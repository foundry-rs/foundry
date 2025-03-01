use ansi_str::AnsiStr;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const ANSI_RESET: &str = "\u{1b}[0m";

/// Returns printed length of string, takes into account escape codes
#[inline(always)]
pub fn measure_text_width(s: &str) -> usize {
    s.ansi_strip().width()
}

/// Split the line by the given deliminator without breaking ansi codes that contain the delimiter
pub fn split_line_by_delimiter(line: &str, delimiter: char) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current_line = String::default();

    // Iterate over line, splitting text with delimiter
    let iter = console::AnsiCodeIterator::new(line);
    for (str_slice, is_esc) in iter {
        if is_esc {
            current_line.push_str(str_slice);
        } else {
            let mut split = str_slice.split(delimiter);

            // Text before first delimiter (if any) belongs to previous line
            let first = split
                .next()
                .expect("split always produces at least one value");
            current_line.push_str(first);

            // Text after each delimiter goes to new line.
            for text in split {
                lines.push(current_line);
                current_line = text.to_string();
            }
        }
    }
    lines.push(current_line);
    fix_style_in_split_str(lines.as_mut());
    lines
}

/// Splits a long word at a given character width. Inserting the needed ansi codes to preserve style.
pub fn split_long_word(allowed_width: usize, word: &str) -> (String, String) {
    // A buffer for the first half of the split str, which will take up at most `allowed_len` characters when printed to the terminal.
    let mut head = String::with_capacity(word.len());
    // A buffer for the second half of the split str
    let mut tail = String::with_capacity(word.len());
    // Tracks the len() of head
    let mut head_len = 0;
    // Tracks the len() of head, sans trailing ansi escape codes
    let mut head_len_last = 0;
    // Count of *non-trailing* escape codes in the buffer.
    let mut escape_count_last = 0;
    // A buffer for the escape codes that exist in the str before the split.
    let mut escapes = Vec::new();

    // Iterate over segments of the input string, each segment is either a singe escape code or block of text containing no escape codes.
    // Add text and escape codes to the head buffer, keeping track of printable length and what ansi codes are active, until there is no more room in allowed_width.
    // If the str was split at a point with active escape-codes, add the ansi reset code to the end of head, and the list of active escape codes to the beginning of tail.
    let mut iter = console::AnsiCodeIterator::new(word);
    for (str_slice, is_esc) in iter.by_ref() {
        if is_esc {
            escapes.push(str_slice);
            // If the code is reset, that means all current codes in the buffer can be ignored.
            if str_slice == ANSI_RESET {
                escapes.clear();
            }
        }

        let slice_len = match is_esc {
            true => 0,
            false => str_slice.width(),
        };

        if head_len + slice_len <= allowed_width {
            head.push_str(str_slice);
            head_len += slice_len;

            if !is_esc {
                // allows popping unneeded escape codes later
                head_len_last = head.len();
                escape_count_last = escapes.len();
            }
        } else {
            assert!(!is_esc);
            let mut graphmes = str_slice.graphemes(true).peekable();
            while let Some(c) = graphmes.peek() {
                let character_width = c.width();
                if allowed_width < head_len + character_width {
                    break;
                }

                head_len += character_width;
                let c = graphmes.next().unwrap();
                head.push_str(c);

                // c is not escape code
                head_len_last = head.len();
                escape_count_last = escapes.len();
            }

            // cut off dangling escape codes since they should have no effect
            head.truncate(head_len_last);
            if escape_count_last != 0 {
                head.push_str(ANSI_RESET);
            }

            for esc in escapes {
                tail.push_str(esc);
            }
            let remaining: String = graphmes.collect();
            tail.push_str(&remaining);
            break;
        }
    }

    iter.for_each(|s| tail.push_str(s.0));
    (head, tail)
}

/// Fixes ansi escape codes in a split string
/// 1. Adds reset code to the end of each substring if needed.
/// 2. Keeps track of previous substring's escape codes and inserts them in later substrings to continue style
pub fn fix_style_in_split_str(words: &mut [String]) {
    let mut escapes: Vec<String> = Vec::new();

    for word in words {
        // before we modify the escape list, make a copy
        let prepend = if !escapes.is_empty() {
            Some(escapes.join(""))
        } else {
            None
        };

        // add escapes in word to escape list
        let iter = console::AnsiCodeIterator::new(word)
            .filter(|(_, is_esc)| *is_esc)
            .map(|v| v.0);
        for esc in iter {
            if esc == ANSI_RESET {
                escapes.clear()
            } else {
                escapes.push(esc.to_string())
            }
        }

        // insert previous esc sequences at the beginning of the segment
        if let Some(prepend) = prepend {
            word.insert_str(0, &prepend);
        }

        // if there are active escape sequences, we need to append reset
        if !escapes.is_empty() {
            word.push_str(ANSI_RESET);
        }
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn ansi_aware_split_test() {
        use super::split_line_by_delimiter;

        let text = "\u{1b}[1m head [ middle [ tail \u{1b}[0m[ after";
        let split = split_line_by_delimiter(text, '[');

        assert_eq!(
            split,
            [
                "\u{1b}[1m head \u{1b}[0m",
                "\u{1b}[1m middle \u{1b}[0m",
                "\u{1b}[1m tail \u{1b}[0m",
                " after"
            ]
        )
    }

    // TODO: Figure out why this fails with the custom_styling feature enabled.
    #[test]
    #[cfg(not(feature = "custom_styling"))]
    fn measure_text_width_osc8_test() {
        use super::measure_text_width;
        use unicode_width::UnicodeWidthStr;

        let text = "\x1b]8;;https://github.com\x1b\\This is a link\x1b]8;;\x1b";
        let width = measure_text_width(text);

        assert_eq!(text.width(), 41);
        assert_eq!(width, 14);
    }
}
