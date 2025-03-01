use comfy_table::{Cell, ContentArrangement, Row, Table};

fn main() {
    let mut table = Table::new();
    //table.load_preset(comfy_table::presets::NOTHING);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_width(85);

    let mut row = Row::new();
    row.add_cell(Cell::new(format!(
        "List of devices:\n{}",
        console::style("Blockdevices\nCryptdevices").dim().blue()
    )));
    row.add_cell(Cell::new(""));

    table.add_row(row);

    let mut row = Row::new();
    row.add_cell(Cell::new(format!(
        "Block devices: \n/dev/{}\n/dev/{}",
        console::style("sda1").bold().red(),
        console::style("sda2").bold().red()
    )));
    row.add_cell(Cell::new("These are some block devices that were found."));
    table.add_row(row);

    let mut row = Row::new();
    row.add_cell(Cell::new(format!(
        "Crypt devices: \n/dev/mapper/{}",
        console::style("cryptroot").bold().yellow()
    )));
    row.add_cell(Cell::new("This one seems to be encrypted."));
    table.add_row(row);

    println!("{}", table);
}
