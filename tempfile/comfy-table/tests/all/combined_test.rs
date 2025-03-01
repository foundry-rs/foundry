use comfy_table::presets::UTF8_FULL;
use comfy_table::*;
use pretty_assertions::assert_eq;

fn get_preset_table() -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(80)
        .set_header(vec![
            Cell::new("Header1").add_attribute(Attribute::Bold),
            Cell::new("Header2").fg(Color::Green),
            Cell::new("Header3"),
        ])
        .add_row(vec![
            Cell::new("This is a bold text").add_attribute(Attribute::Bold),
            Cell::new("This is a green text").fg(Color::Green),
            Cell::new("This one has black background").bg(Color::Black),
        ])
        .add_row(vec![
            Cell::new("Blinky boi").add_attribute(Attribute::SlowBlink),
            Cell::new("This table's content is dynamically arranged. The table is exactly 80 characters wide.\nHere comes a reallylongwordthatshoulddynamicallywrap"),
            Cell::new("COMBINE ALL THE THINGS")
            .fg(Color::Green)
            .bg(Color::Black)
            .add_attributes(vec![
                Attribute::Bold,
                Attribute::SlowBlink,
            ])
        ]);

    table
}

#[test]
fn combined_features() {
    let mut table = get_preset_table();
    table.force_no_tty().enforce_styling();
    println!("{table}");
    let expected = "
┌─────────────────────┬───────────────────────────────┬────────────────────────┐
│\u{1b}[1m Header1             \u{1b}[0m┆\u{1b}[38;5;10m Header2                       \u{1b}[39m┆ Header3                │
╞═════════════════════╪═══════════════════════════════╪════════════════════════╡
│\u{1b}[1m This is a bold text \u{1b}[0m┆\u{1b}[38;5;10m This is a green text          \u{1b}[39m┆\u{1b}[48;5;0m This one has black     \u{1b}[49m│
│                     ┆                               ┆\u{1b}[48;5;0m background             \u{1b}[49m│
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│\u{1b}[5m Blinky boi          \u{1b}[0m┆ This table\'s content is       ┆\u{1b}[48;5;0m\u{1b}[38;5;10m\u{1b}[1m\u{1b}[5m COMBINE ALL THE THINGS \u{1b}[0m│
│                     ┆ dynamically arranged. The     ┆                        │
│                     ┆ table is exactly 80           ┆                        │
│                     ┆ characters wide.              ┆                        │
│                     ┆ Here comes a reallylongwordth ┆                        │
│                     ┆ atshoulddynamicallywrap       ┆                        │
└─────────────────────┴───────────────────────────────┴────────────────────────┘";
    println!("{expected}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
