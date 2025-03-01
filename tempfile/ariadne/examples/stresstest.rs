use ariadne::{Color, ColorGenerator, Config, Label, Report, ReportKind, Source};

fn main() {
    let mut colors = ColorGenerator::new();

    Report::build(ReportKind::Error, ("stresstest.tao", 13..13))
        .with_code(3)
        .with_message("Incompatible types".to_string())
        .with_label(
            Label::new(("stresstest.tao", 0..1))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 1..2))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 2..3))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 3..4))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 4..5))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 5..6))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 6..7))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 7..8))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 8..9))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 9..10))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 10..11))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 11..12))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 12..13))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 13..14))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 14..15))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 15..16))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 16..17))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 17..18))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 18..19))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 19..20))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 20..21))
                .with_message("Color")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 18..19))
                .with_message("This is of type Nat")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 13..16))
                .with_message("This is of type Str")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 40..41))
                .with_message("This is of type Nat")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 43..47))
                .with_message("This is of type Bool")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 49..51))
                .with_message("This is of type ()")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 53..55))
                .with_message("This is of type [_]")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 25..78))
                .with_message("This is of type Str")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 81..124))
                .with_message("This is of type Nat")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 100..126))
                .with_message("This is an inner multi-line")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 106..120))
                .with_message("This is another inner multi-line")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 108..122))
                .with_message("This is *really* nested multi-line")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 110..111))
                .with_message("This is an inline within the nesting!")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 111..112))
                .with_message("And another!")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 103..123))
                .with_message("This is *really* nested multi-line")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 105..125))
                .with_message("This is *really* nested multi-line")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 112..116))
                .with_message("This is *really* nested multi-line")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 26..100))
                .with_message("Hahaha!")
                .with_color(Color::Fixed(75)),
        )
        .with_label(
            Label::new(("stresstest.tao", 85..110))
                .with_message("Oh god, no more 1")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 84..114))
                .with_message("Oh god, no more 2")
                .with_color(colors.next()),
        )
        .with_label(
            Label::new(("stresstest.tao", 89..113))
                .with_message("Oh god, no more 3")
                .with_color(colors.next()),
        )
        .with_config(
            Config::default()
                .with_cross_gap(false)
                .with_compact(true)
                .with_underlines(true)
                .with_tab_width(4),
        )
        .finish()
        .print((
            "stresstest.tao",
            Source::from(include_str!("stresstest.tao")),
        ))
        .unwrap();
}
