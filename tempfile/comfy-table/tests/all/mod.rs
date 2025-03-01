use comfy_table::Table;
use unicode_width::UnicodeWidthStr;

mod add_predicate;
mod alignment_test;
#[cfg(feature = "tty")]
mod combined_test;
mod constraints_test;
mod content_arrangement_test;
mod counts;
mod custom_delimiter_test;
mod edge_cases;
mod hidden_test;
#[cfg(feature = "custom_styling")]
mod inner_style_test;
mod modifiers_test;
mod padding_test;
mod presets_test;
mod property_test;
mod simple_test;
#[cfg(feature = "tty")]
mod styling_test;
mod truncation;
mod utf_8_characters;

pub fn assert_table_line_width(table: &Table, count: usize) {
    for line in table.lines() {
        assert_eq!(line.width(), count);
    }
}
