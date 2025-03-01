use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// returns printed length of string
/// if ansi feature enabled, takes into account escape codes
#[inline(always)]
pub fn measure_text_width(s: &str) -> usize {
    s.width()
}

/// Split a line into its individual parts along the given delimiter.
pub fn split_line_by_delimiter(line: &str, delimiter: char) -> Vec<String> {
    line.split(delimiter)
        .map(ToString::to_string)
        .collect::<Vec<String>>()
}

/// Splits a long word at a given character width.
/// This needs some special logic, as we have to take multi-character UTF-8 symbols into account.
/// When simply splitting at a certain char position, we might end up with a string that's has a
/// wider display width than allowed.
pub fn split_long_word(allowed_width: usize, word: &str) -> (String, String) {
    let mut current_width = 0;
    let mut parts = String::new();

    let mut graphmes = word.graphemes(true).peekable();

    // Check if the string might be too long, one Unicode grapheme at a time.
    // Peek into the next grapheme and check the exit condition.
    //
    // This code uses graphemes to handle both zero-width joiner[0] UTF-8 chars, which
    // combine multiple UTF-8 chars into a single grapheme, and variant selectors [1],
    // which pick a certain variant of the preceding char.
    //
    // [0]: https://en.wikipedia.org/wiki/Zero-width_joiner
    // [1]: https://en.wikipedia.org/wiki/Variation_Selectors_(Unicode_block)
    while let Some(c) = graphmes.peek() {
        if (current_width + c.width()) > allowed_width {
            break;
        }

        // We can unwrap, as we just checked that a suitable grapheme is next in line.
        let c = graphmes.next().unwrap();

        let character_width = c.width();
        current_width += character_width;
        parts.push_str(c);
    }

    // Collect the remaining characters.
    let remaining = graphmes.collect();
    (parts, remaining)
}
