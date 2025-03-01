use ariadne::{Color, Config, Label, Report, ReportKind, Source};

fn main() {
    Report::build(ReportKind::Error, 34..34)
        .with_message("Incompatible types")
        .with_label(Label::new(32..33).with_message("This is of type Nat"))
        .with_label(Label::new(42..45).with_message("This is of type Str"))
        .finish()
        .print(Source::from(include_str!("sample.tao")))
        .unwrap();

    const SOURCE: &str = "a b c d e f";
    // also supports labels with no messages to only emphasis on some areas
    Report::build(ReportKind::Error, 2..3)
        .with_message("Incompatible types")
        .with_config(Config::default().with_compact(true))
        .with_label(Label::new(0..1).with_color(Color::Red))
        .with_label(
            Label::new(2..3)
                .with_color(Color::Blue)
                .with_message("`b` for banana")
                .with_order(1),
        )
        .with_label(Label::new(4..5).with_color(Color::Green))
        .with_label(
            Label::new(7..9)
                .with_color(Color::Cyan)
                .with_message("`e` for emerald"),
        )
        .finish()
        .print(Source::from(SOURCE))
        .unwrap();
}
