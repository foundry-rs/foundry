use pretty_assertions::assert_eq;

use comfy_table::modifiers::*;
use comfy_table::presets::*;
use comfy_table::*;

fn get_preset_table() -> Table {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec!["One One", "One Two", "One Three"])
        .add_row(vec!["One One", "One Two", "One Three"]);

    table
}

#[test]
fn utf8_round_corners() {
    let mut table = get_preset_table();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS);
    let expected = "
╭─────────┬─────────┬───────────╮
│ Header1 ┆ Header2 ┆ Header3   │
╞═════════╪═════════╪═══════════╡
│ One One ┆ One Two ┆ One Three │
├╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┤
│ One One ┆ One Two ┆ One Three │
╰─────────┴─────────┴───────────╯";

    println!("{table}");
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
