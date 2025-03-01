use pretty_assertions::assert_eq;

use comfy_table::presets::UTF8_FULL;
use comfy_table::*;

fn get_preset_table() -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_header(vec![
            Cell::new("Header1").add_attribute(Attribute::Bold),
            Cell::new("Header2").fg(Color::Green),
            Cell::new("Header3").bg(Color::Black),
        ])
        .add_row(vec![
            Cell::new("This is a bold text").add_attribute(Attribute::Bold),
            Cell::new("This is a green text").fg(Color::Green),
            Cell::new("This one has black background").bg(Color::Black),
        ])
        .add_row(vec![
            Cell::new("Blinking boiii").add_attribute(Attribute::SlowBlink),
            Cell::new("Now\nadd some\nmulti line stuff")
                .fg(Color::Cyan)
                .add_attribute(Attribute::Underlined),
            Cell::new("COMBINE ALL THE THINGS")
                .fg(Color::Green)
                .bg(Color::Black)
                .add_attribute(Attribute::Bold)
                .add_attribute(Attribute::SlowBlink),
        ]);

    table
}

#[test]
fn styled_table() {
    let mut table = get_preset_table();
    table.force_no_tty().enforce_styling();
    println!("{table}");
    let expected = "
┌─────────────────────┬──────────────────────┬───────────────────────────────┐
│\u{1b}[1m Header1             \u{1b}[0m┆\u{1b}[38;5;10m Header2              \u{1b}[39m┆\u{1b}[48;5;0m Header3                       \u{1b}[49m│
╞═════════════════════╪══════════════════════╪═══════════════════════════════╡
│\u{1b}[1m This is a bold text \u{1b}[0m┆\u{1b}[38;5;10m This is a green text \u{1b}[39m┆\u{1b}[48;5;0m This one has black background \u{1b}[49m│
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│\u{1b}[5m Blinking boiii      \u{1b}[0m┆\u{1b}[38;5;14m\u{1b}[4m Now                  \u{1b}[0m┆\u{1b}[48;5;0m\u{1b}[38;5;10m\u{1b}[1m\u{1b}[5m COMBINE ALL THE THINGS        \u{1b}[0m│
│                     ┆\u{1b}[38;5;14m\u{1b}[4m add some             \u{1b}[0m┆                               │
│                     ┆\u{1b}[38;5;14m\u{1b}[4m multi line stuff     \u{1b}[0m┆                               │
└─────────────────────┴──────────────────────┴───────────────────────────────┘";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn no_style_styled_table() {
    let mut table = get_preset_table();
    table.force_no_tty();

    println!("{table}");
    let expected = "
┌─────────────────────┬──────────────────────┬───────────────────────────────┐
│ Header1             ┆ Header2              ┆ Header3                       │
╞═════════════════════╪══════════════════════╪═══════════════════════════════╡
│ This is a bold text ┆ This is a green text ┆ This one has black background │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Blinking boiii      ┆ Now                  ┆ COMBINE ALL THE THINGS        │
│                     ┆ add some             ┆                               │
│                     ┆ multi line stuff     ┆                               │
└─────────────────────┴──────────────────────┴───────────────────────────────┘";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn styled_text_only_table() {
    let mut table = get_preset_table();
    table.force_no_tty().enforce_styling().style_text_only();
    println!("{table}");
    let expected = "
┌─────────────────────┬──────────────────────┬───────────────────────────────┐
│ \u{1b}[1mHeader1\u{1b}[0m             ┆ \u{1b}[38;5;10mHeader2\u{1b}[39m              ┆ \u{1b}[48;5;0mHeader3\u{1b}[49m                       │
╞═════════════════════╪══════════════════════╪═══════════════════════════════╡
│ \u{1b}[1mThis is a bold text\u{1b}[0m ┆ \u{1b}[38;5;10mThis is a green text\u{1b}[39m ┆ \u{1b}[48;5;0mThis one has black background\u{1b}[49m │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ \u{1b}[5mBlinking boiii\u{1b}[0m      ┆ \u{1b}[38;5;14m\u{1b}[4mNow\u{1b}[0m                  ┆ \u{1b}[48;5;0m\u{1b}[38;5;10m\u{1b}[1m\u{1b}[5mCOMBINE ALL THE THINGS\u{1b}[0m        │
│                     ┆ \u{1b}[38;5;14m\u{1b}[4madd some\u{1b}[0m             ┆                               │
│                     ┆ \u{1b}[38;5;14m\u{1b}[4mmulti line stuff\u{1b}[0m     ┆                               │
└─────────────────────┴──────────────────────┴───────────────────────────────┘";

    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
