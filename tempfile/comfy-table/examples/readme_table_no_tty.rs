use comfy_table::presets::UTF8_FULL;
use comfy_table::*;

// This example works even with the `tty` feature disabled
// You can try it out with `cargo run --example no_tty --no-default-features`

fn main() {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(80)
        .set_header(vec![
            Cell::new("Header1"),
            Cell::new("Header2"),
            Cell::new("Header3"),
        ])
        .add_row(vec![
            Cell::new("No bold text without tty"),
            Cell::new("No colored text without tty"),
            Cell::new("No custom background without tty"),
        ])
        .add_row(vec![
            Cell::new("Blinky boi"),
            Cell::new("This table's content is dynamically arranged. The table is exactly 80 characters wide.\nHere comes a reallylongwordthatshoulddynamicallywrap"),
            Cell::new("Done"),
        ]);

    println!("{table}");
}
