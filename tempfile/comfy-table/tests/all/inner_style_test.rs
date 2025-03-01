use comfy_table::{presets::UTF8_FULL, *};
use pretty_assertions::assert_eq;

fn get_preset_table() -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_width(85);

    let mut row = Row::new();
    row.add_cell(Cell::new(format!(
        "hello{}cell1",
        console::style("123\n456").dim().blue()
    )));
    row.add_cell(Cell::new("cell2"));

    table.add_row(row);

    let mut row = Row::new();
    row.add_cell(Cell::new(
        format!(r"cell sys-devices-pci00:00-0000:000:07:00.1-usb2-2\x2d1-2\x2d1.3-2\x2d1.3:1.0-host2-target2:0:0-2:0:0:1-block-sdb{}", console::style(".device").bold().red())
    ));
    row.add_cell(Cell::new(
        "cell4 asdfasfsad asdfasdf sad fas df asdf as df asdf    asdfasdfasdfasdfasdfasdfa dsfa sdf asdf asd f asdf as df sadf asd fas df "
    ));
    table.add_row(row);

    let mut row = Row::new();
    row.add_cell(Cell::new("cell5"));
    row.add_cell(Cell::new("cell6"));
    table.add_row(row);

    table
}

#[test]
fn styled_table() {
    console::set_colors_enabled(true);
    let mut table = get_preset_table();
    table.force_no_tty().enforce_styling();
    println!("{table}");
    let expected = "
┌─────────────────────────────────────────┬─────────────────────────────────────────┐
│ hello\u{1b}[34m\u{1b}[2m123\u{1b}[0m                                ┆ cell2                                   │
│ \u{1b}[34m\u{1b}[2m456\u{1b}[0mcell1                                ┆                                         │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ cell sys-devices-pci00:00-0000:000:07:0 ┆ cell4 asdfasfsad asdfasdf sad fas df    │
│ 0.1-usb2-2\\x2d1-2\\x2d1.3-2\\x2d1.3:1.0-h ┆ asdf as df asdf                         │
│ ost2-target2:0:0-2:0:0:1-block-sdb\u{1b}[31m\u{1b}[1m.devi\u{1b}[0m ┆ asdfasdfasdfasdfasdfasdfa dsfa sdf asdf │
│ \u{1b}[31m\u{1b}[1mce\u{1b}[0m                                      ┆ asd f asdf as df sadf asd fas df        │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ cell5                                   ┆ cell6                                   │
└─────────────────────────────────────────┴─────────────────────────────────────────┘";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn no_style_styled_table() {
    console::set_colors_enabled(true);
    let mut table = get_preset_table();
    table.force_no_tty();

    println!("{table}");
    let expected = "
┌─────────────────────────────────────────┬─────────────────────────────────────────┐
│ hello\u{1b}[34m\u{1b}[2m123\u{1b}[0m                                ┆ cell2                                   │
│ \u{1b}[34m\u{1b}[2m456\u{1b}[0mcell1                                ┆                                         │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ cell sys-devices-pci00:00-0000:000:07:0 ┆ cell4 asdfasfsad asdfasdf sad fas df    │
│ 0.1-usb2-2\\x2d1-2\\x2d1.3-2\\x2d1.3:1.0-h ┆ asdf as df asdf                         │
│ ost2-target2:0:0-2:0:0:1-block-sdb\u{1b}[31m\u{1b}[1m.devi\u{1b}[0m ┆ asdfasdfasdfasdfasdfasdfa dsfa sdf asdf │
│ \u{1b}[31m\u{1b}[1mce\u{1b}[0m                                      ┆ asd f asdf as df sadf asd fas df        │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ cell5                                   ┆ cell6                                   │
└─────────────────────────────────────────┴─────────────────────────────────────────┘";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}

#[test]
fn styled_text_only_table() {
    console::set_colors_enabled(true);
    let mut table = get_preset_table();
    table.force_no_tty().enforce_styling().style_text_only();
    println!("{table}");
    let expected = "
┌─────────────────────────────────────────┬─────────────────────────────────────────┐
│ hello\u{1b}[34m\u{1b}[2m123\u{1b}[0m                                ┆ cell2                                   │
│ \u{1b}[34m\u{1b}[2m456\u{1b}[0mcell1                                ┆                                         │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ cell sys-devices-pci00:00-0000:000:07:0 ┆ cell4 asdfasfsad asdfasdf sad fas df    │
│ 0.1-usb2-2\\x2d1-2\\x2d1.3-2\\x2d1.3:1.0-h ┆ asdf as df asdf                         │
│ ost2-target2:0:0-2:0:0:1-block-sdb\u{1b}[31m\u{1b}[1m.devi\u{1b}[0m ┆ asdfasdfasdfasdfasdfasdfa dsfa sdf asdf │
│ \u{1b}[31m\u{1b}[1mce\u{1b}[0m                                      ┆ asd f asdf as df sadf asd fas df        │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ cell5                                   ┆ cell6                                   │
└─────────────────────────────────────────┴─────────────────────────────────────────┘";
    assert_eq!(expected, "\n".to_string() + &table.to_string());
}
