#![cfg(feature = "in_memory")]

use std::time::Duration;

use indicatif::{
    InMemoryTerm, MultiProgress, MultiProgressAlignment, ProgressBar, ProgressDrawTarget,
    ProgressFinish, ProgressStyle, TermLike,
};
use pretty_assertions::assert_eq;

#[test]
fn basic_progress_bar() {
    let in_mem = InMemoryTerm::new(10, 80);
    let pb = ProgressBar::with_draw_target(
        Some(10),
        ProgressDrawTarget::term_like(Box::new(in_mem.clone())),
    );

    assert_eq!(in_mem.contents(), String::new());

    pb.tick();
    assert_eq!(
        in_mem.contents(),
        "░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"
    );

    pb.inc(1);
    assert_eq!(
        in_mem.contents(),
        "███████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 1/10"
    );

    pb.finish();
    assert_eq!(
        in_mem.contents(),
        "██████████████████████████████████████████████████████████████████████████ 10/10"
    );
}

#[test]
fn progress_bar_builder_method_order() {
    let in_mem = InMemoryTerm::new(10, 80);
    // Test that `with_style` doesn't overwrite the message or prefix
    let pb = ProgressBar::with_draw_target(
        Some(10),
        ProgressDrawTarget::term_like(Box::new(in_mem.clone())),
    )
    .with_message("crate")
    .with_prefix("Downloading")
    .with_style(
        ProgressStyle::with_template("{prefix:>12.cyan.bold} {msg}: {wide_bar} {pos}/{len}")
            .unwrap(),
    );

    assert_eq!(in_mem.contents(), String::new());

    pb.tick();
    assert_eq!(
        in_mem.contents(),
        " Downloading crate: ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"
    );
}

#[test]
fn progress_bar_percent_with_no_length() {
    let in_mem = InMemoryTerm::new(10, 80);
    let pb = ProgressBar::with_draw_target(
        None,
        ProgressDrawTarget::term_like(Box::new(in_mem.clone())),
    )
    .with_style(ProgressStyle::with_template("{wide_bar} {percent}%").unwrap());

    assert_eq!(in_mem.contents(), String::new());

    pb.tick();

    assert_eq!(
        in_mem.contents(),
        "░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0%"
    );

    pb.set_length(10);

    pb.inc(1);
    assert_eq!(
        in_mem.contents(),
        "███████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 10%"
    );

    pb.finish();
    assert_eq!(
        in_mem.contents(),
        "███████████████████████████████████████████████████████████████████████████ 100%"
    );
}

#[test]
fn progress_bar_percent_precise_with_no_length() {
    let in_mem = InMemoryTerm::new(10, 80);
    let pb = ProgressBar::with_draw_target(
        None,
        ProgressDrawTarget::term_like(Box::new(in_mem.clone())),
    )
    .with_style(ProgressStyle::with_template("{wide_bar} {percent_precise}%").unwrap());

    assert_eq!(in_mem.contents(), String::new());

    pb.tick();

    assert_eq!(
        in_mem.contents(),
        "░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0.000%"
    );

    pb.set_length(10);

    pb.inc(1);
    assert_eq!(
        in_mem.contents(),
        "███████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 10.000%"
    );

    pb.finish();
    assert_eq!(
        in_mem.contents(),
        "███████████████████████████████████████████████████████████████████████ 100.000%"
    );
}

#[test]
fn multi_progress_single_bar_and_leave() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new(10).with_finish(ProgressFinish::AndLeave));

    assert_eq!(in_mem.contents(), String::new());

    pb1.tick();
    assert_eq!(
        in_mem.contents(),
        r#"░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"#
    );

    drop(pb1);
    assert_eq!(
        in_mem.contents(),
        r#"██████████████████████████████████████████████████████████████████████████ 10/10"#
    );
}

#[test]
fn multi_progress_single_bar_and_clear() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new(10));

    assert_eq!(in_mem.contents(), String::new());

    pb1.tick();
    assert_eq!(
        in_mem.contents(),
        r#"░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"#
    );

    drop(pb1);
    assert_eq!(in_mem.contents(), "");
}

#[test]
fn multi_progress_two_bars() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new(10).with_finish(ProgressFinish::AndLeave));
    let pb2 = mp.add(ProgressBar::new(5));

    assert_eq!(in_mem.contents(), String::new());

    pb1.tick();
    assert_eq!(
        in_mem.contents(),
        r#"░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"#
    );

    pb2.tick();

    assert_eq!(
        in_mem.contents(),
        r#"
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/5"#
            .trim_start()
    );

    drop(pb1);
    assert_eq!(
        in_mem.contents(),
        r#"
██████████████████████████████████████████████████████████████████████████ 10/10
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/5"#
            .trim_start()
    );

    drop(pb2);

    assert_eq!(
        in_mem.contents(),
        r#"██████████████████████████████████████████████████████████████████████████ 10/10"#
    );
}

#[test]
fn multi_progress() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new(10).with_finish(ProgressFinish::AndLeave));
    let pb2 = mp.add(ProgressBar::new(5));
    let pb3 = mp.add(ProgressBar::new(100));

    assert_eq!(in_mem.contents(), String::new());

    pb1.tick();
    assert_eq!(
        in_mem.contents(),
        r#"░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"#
    );

    pb2.tick();

    assert_eq!(
        in_mem.contents(),
        r#"
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/5"#
            .trim_start()
    );

    pb3.tick();
    assert_eq!(
        in_mem.contents(),
        r#"
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/5
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/100"#
            .trim_start()
    );

    drop(pb1);
    assert_eq!(
        in_mem.contents(),
        r#"
██████████████████████████████████████████████████████████████████████████ 10/10
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/5
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/100"#
            .trim_start()
    );

    drop(pb2);
    assert_eq!(
        in_mem.contents(),
        r#"
██████████████████████████████████████████████████████████████████████████ 10/10
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/100"#
            .trim_start()
    );

    drop(pb3);

    assert_eq!(
        in_mem.contents(),
        r#"██████████████████████████████████████████████████████████████████████████ 10/10"#
    );
}

#[test]
fn multi_progress_println() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new(10));
    let pb2 = mp.add(ProgressBar::new(5));
    let pb3 = mp.add(ProgressBar::new(100));

    assert_eq!(in_mem.contents(), "");

    pb1.inc(2);
    mp.println("message printed :)").unwrap();

    assert_eq!(
        in_mem.contents(),
        r#"
message printed :)
███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/10
            "#
        .trim()
    );

    mp.println("another great message!").unwrap();
    assert_eq!(
        in_mem.contents(),
        r#"
message printed :)
another great message!
███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/10
            "#
        .trim()
    );

    pb2.inc(1);
    pb3.tick();
    mp.println("one last message").unwrap();

    assert_eq!(
        in_mem.contents(),
        r#"
message printed :)
another great message!
one last message
███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/10
███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 1/5
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/100
        "#
        .trim()
    );

    drop(pb1);
    drop(pb2);
    drop(pb3);

    assert_eq!(
        in_mem.contents(),
        r#"
message printed :)
another great message!
one last message"#
            .trim()
    );
}

#[test]
fn multi_progress_suspend() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new(10));
    let pb2 = mp.add(ProgressBar::new(10));

    assert_eq!(in_mem.contents(), "");

    pb1.inc(2);
    mp.println("message printed :)").unwrap();

    assert_eq!(
        in_mem.contents(),
        r#"
message printed :)
███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/10
            "#
        .trim()
    );

    mp.suspend(|| {
        in_mem.write_line("This is write_line output!").unwrap();
        in_mem.write_line("And so is this").unwrap();
        in_mem.move_cursor_down(1).unwrap();
    });

    assert_eq!(
        in_mem.contents(),
        r#"
message printed :)
This is write_line output!
And so is this

███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/10
            "#
        .trim()
    );

    pb2.inc(1);
    mp.println("Another line printed").unwrap();

    assert_eq!(
        in_mem.contents(),
        r#"
message printed :)
This is write_line output!
And so is this

Another line printed
███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/10
███████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 1/10
            "#
        .trim()
    );

    drop(pb1);
    drop(pb2);

    assert_eq!(
        in_mem.contents(),
        r#"
message printed :)
This is write_line output!
And so is this

Another line printed"#
            .trim()
    );
}

#[test]
fn multi_progress_move_cursor() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));
    mp.set_move_cursor(true);

    let pb1 = mp.add(ProgressBar::new(10));
    pb1.tick();
    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Str("\r")
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
Flush
"#
    );

    let pb2 = mp.add(ProgressBar::new(10));
    pb2.tick();
    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Str("\r")
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
Flush
"#
    );

    pb1.inc(1);
    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Up(1)
Str("\r")
Str("███████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 1/10")
Str("")
NewLine
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
Flush
"#
    );
}

#[test]
fn multi_progress_println_bar_with_target() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb = mp.add(ProgressBar::with_draw_target(
        Some(10),
        ProgressDrawTarget::term_like(Box::new(in_mem.clone())),
    ));

    assert_eq!(in_mem.contents(), "");

    pb.println("message printed :)");
    pb.inc(2);
    assert_eq!(
        in_mem.contents(),
        r#"
message printed :)
███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/10
            "#
        .trim()
    );
}

#[test]
fn ticker_drop() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let mut spinner: Option<ProgressBar> = None;

    for i in 0..5 {
        let new_spinner = mp.add(
            ProgressBar::new_spinner()
                .with_finish(ProgressFinish::AndLeave)
                .with_message(format!("doing stuff {i}")),
        );
        new_spinner.enable_steady_tick(Duration::from_millis(100));
        spinner.replace(new_spinner);
    }

    drop(spinner);
    assert_eq!(
        in_mem.contents(),
        "  doing stuff 0\n  doing stuff 1\n  doing stuff 2\n  doing stuff 3\n  doing stuff 4"
    );
}

#[test]
fn manually_inc_ticker() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let spinner = mp.add(ProgressBar::new_spinner().with_message("msg"));

    assert_eq!(in_mem.contents(), "");

    spinner.inc(1);
    assert_eq!(in_mem.contents(), "⠁ msg");

    spinner.inc(1);
    assert_eq!(in_mem.contents(), "⠉ msg");

    // set_message / set_prefix shouldn't increase tick
    spinner.set_message("new message");
    spinner.set_prefix("prefix");
    assert_eq!(in_mem.contents(), "⠉ new message");
}

#[test]
fn multi_progress_prune_zombies() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb0 = mp
        .add(ProgressBar::new(10))
        .with_finish(ProgressFinish::AndLeave);
    let pb1 = mp.add(ProgressBar::new(15));
    pb0.tick();
    assert_eq!(
        in_mem.contents(),
        "░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"
    );

    pb0.inc(1);
    assert_eq!(
        in_mem.contents(),
        "███████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 1/10"
    );

    drop(pb0);

    // Clear the screen
    mp.clear().unwrap();

    // Write a line that we expect to remain. This helps ensure the adjustment to last_line_count is
    // working as expected, and `MultiState` isn't erasing lines when it shouldn't.
    in_mem.write_line("don't erase me plz").unwrap();

    // pb0 is dead, so only pb1 should be drawn from now on
    pb1.tick();
    assert_eq!(
        in_mem.contents(),
        "don't erase me plz\n░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/15"
    );
}

#[test]
fn multi_progress_prune_zombies_2() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new(10).with_finish(ProgressFinish::AndLeave));
    let pb2 = mp.add(ProgressBar::new(5));
    let pb3 = mp
        .add(ProgressBar::new(100))
        .with_finish(ProgressFinish::Abandon);
    let pb4 = mp
        .add(ProgressBar::new(500))
        .with_finish(ProgressFinish::AndLeave);
    let pb5 = mp.add(ProgressBar::new(7));

    assert_eq!(in_mem.contents(), String::new());

    pb1.tick();
    assert_eq!(
        in_mem.contents(),
        r#"░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"#
    );

    pb2.tick();

    assert_eq!(
        in_mem.contents(),
        r#"
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/5"#
            .trim_start()
    );

    pb3.tick();
    assert_eq!(
        in_mem.contents(),
        r#"
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/5
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/100"#
            .trim_start()
    );

    drop(pb1);
    drop(pb2);
    drop(pb3);

    assert_eq!(
        in_mem.contents(),
        r#"
██████████████████████████████████████████████████████████████████████████ 10/10
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/100"#
            .trim_start()
    );

    mp.clear().unwrap();

    assert_eq!(in_mem.contents(), "");

    // A sacrificial line we expect shouldn't be touched
    in_mem.write_line("don't erase plz").unwrap();

    mp.println("Test friend :)").unwrap();
    assert_eq!(
        in_mem.contents(),
        r#"
don't erase plz
Test friend :)"#
            .trim_start()
    );

    pb4.tick();
    assert_eq!(
        in_mem.contents(),
        r#"
don't erase plz
Test friend :)
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/500"#
            .trim_start()
    );

    drop(pb4);
    assert_eq!(
        in_mem.contents(),
        r#"
don't erase plz
Test friend :)
████████████████████████████████████████████████████████████████████████ 500/500"#
            .trim_start()
    );

    mp.clear().unwrap();
    assert_eq!(in_mem.contents(), "don't erase plz\nTest friend :)");

    pb5.tick();
    assert_eq!(
        in_mem.contents(),
        r#"
don't erase plz
Test friend :)
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/7"#
            .trim_start()
    );

    mp.println("not your friend, buddy").unwrap();
    assert_eq!(
        in_mem.contents(),
        r#"
don't erase plz
Test friend :)
not your friend, buddy
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/7"#
            .trim_start()
    );

    pb5.inc(1);
    assert_eq!(
        in_mem.contents(),
        r#"
don't erase plz
Test friend :)
not your friend, buddy
██████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 1/7"#
            .trim_start()
    );

    mp.clear().unwrap();
    in_mem.write_line("don't erase me either").unwrap();

    pb5.inc(1);
    assert_eq!(
        in_mem.contents(),
        r#"
don't erase plz
Test friend :)
not your friend, buddy
don't erase me either
█████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/7"#
            .trim_start()
    );

    drop(pb5);

    assert_eq!(
        in_mem.contents(),
        r#"
don't erase plz
Test friend :)
not your friend, buddy
don't erase me either"#
            .trim_start()
    );
}

#[test]
fn basic_tab_expansion() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let spinner = mp.add(ProgressBar::new_spinner().with_message("Test\t:)"));
    spinner.tick();

    // 8 is the default number of spaces
    assert_eq!(in_mem.contents(), "⠁ Test        :)");

    spinner.set_tab_width(4);
    assert_eq!(in_mem.contents(), "⠁ Test    :)");
}

#[test]
fn tab_expansion_in_template() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let spinner = mp.add(
        ProgressBar::new_spinner()
            .with_message("Test\t:)")
            .with_prefix("Pre\tfix!")
            .with_style(ProgressStyle::with_template("{spinner}{prefix}\t{msg}").unwrap()),
    );

    spinner.tick();
    assert_eq!(in_mem.contents(), "⠁Pre        fix!        Test        :)");

    spinner.set_tab_width(4);
    assert_eq!(in_mem.contents(), "⠁Pre    fix!    Test    :)");

    spinner.set_tab_width(2);
    assert_eq!(in_mem.contents(), "⠁Pre  fix!  Test  :)");
}

#[test]
fn progress_style_tab_width_unification() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    // Style will have default of 8 spaces for tabs
    let style = ProgressStyle::with_template("{msg}\t{msg}").unwrap();

    let spinner = mp.add(
        ProgressBar::new_spinner()
            .with_message("OK")
            .with_tab_width(4),
    );

    // Setting the spinner's style to |style| should override the style's tab width with that of bar
    spinner.set_style(style);
    spinner.tick();
    assert_eq!(in_mem.contents(), "OK    OK");
}

#[test]
fn multi_progress_clear_println() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    mp.println("Test of println").unwrap();
    // Should have no effect
    mp.clear().unwrap();
    assert_eq!(in_mem.contents(), "Test of println");
}

#[test]
fn multi_progress_clear_zombies_no_ticks() {
    _multi_progress_clear_zombies(0);
}

#[test]
fn multi_progress_clear_zombies_one_tick() {
    _multi_progress_clear_zombies(1);
}

#[test]
fn multi_progress_clear_zombies_two_ticks() {
    _multi_progress_clear_zombies(2);
}

// In the old (broken) implementation, zombie handling sometimes worked differently depending on
// how many draws were between certain operations. Let's make sure that doesn't happen again.
fn _multi_progress_clear_zombies(ticks: usize) {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));
    let style = ProgressStyle::with_template("{msg}").unwrap();

    let pb1 = mp.add(
        ProgressBar::new_spinner()
            .with_style(style.clone())
            .with_message("pb1"),
    );
    pb1.tick();

    let pb2 = mp.add(
        ProgressBar::new_spinner()
            .with_style(style)
            .with_message("pb2"),
    );

    pb2.tick();
    assert_eq!(in_mem.contents(), "pb1\npb2");

    pb1.finish_with_message("pb1 done");
    drop(pb1);
    assert_eq!(in_mem.contents(), "pb1 done\npb2");

    for _ in 0..ticks {
        pb2.tick();
    }

    mp.clear().unwrap();
    assert_eq!(in_mem.contents(), "");
}

// This test reproduces examples/multi.rs in a simpler form
#[test]
fn multi_zombie_handling() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));
    let style = ProgressStyle::with_template("{msg}").unwrap();

    let pb1 = mp.add(
        ProgressBar::new_spinner()
            .with_style(style.clone())
            .with_message("pb1"),
    );
    pb1.tick();
    let pb2 = mp.add(
        ProgressBar::new_spinner()
            .with_style(style.clone())
            .with_message("pb2"),
    );
    pb2.tick();
    let pb3 = mp.add(
        ProgressBar::new_spinner()
            .with_style(style)
            .with_message("pb3"),
    );
    pb3.tick();

    mp.println("pb1 done!").unwrap();
    pb1.finish_with_message("done");
    assert_eq!(in_mem.contents(), "pb1 done!\ndone\npb2\npb3");
    drop(pb1);

    assert_eq!(in_mem.contents(), "pb1 done!\ndone\npb2\npb3");

    pb2.tick();
    assert_eq!(in_mem.contents(), "pb1 done!\ndone\npb2\npb3");
    pb3.tick();
    assert_eq!(in_mem.contents(), "pb1 done!\ndone\npb2\npb3");

    mp.println("pb3 done!").unwrap();
    assert_eq!(in_mem.contents(), "pb1 done!\npb3 done!\npb2\npb3");

    pb3.finish_with_message("done");
    drop(pb3);

    pb2.tick();

    mp.println("pb2 done!").unwrap();
    pb2.finish_with_message("done");
    drop(pb2);

    assert_eq!(
        in_mem.contents(),
        "pb1 done!\npb3 done!\npb2 done!\ndone\ndone"
    );

    mp.clear().unwrap();

    assert_eq!(in_mem.contents(), "pb1 done!\npb3 done!\npb2 done!");
}

#[test]
fn multi_progress_multiline_msg() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new_spinner().with_message("test1"));
    let pb2 = mp.add(ProgressBar::new_spinner().with_message("test2"));

    assert_eq!(in_mem.contents(), "");

    pb1.inc(1);
    pb2.inc(1);

    assert_eq!(
        in_mem.contents(),
        r#"
⠁ test1
⠁ test2
            "#
        .trim()
    );

    pb1.set_message("test1\n  test1 line2\n  test1 line3");

    assert_eq!(
        in_mem.contents(),
        r#"
⠁ test1
  test1 line2
  test1 line3
⠁ test2
            "#
        .trim()
    );

    pb1.inc(1);
    pb2.inc(1);

    assert_eq!(
        in_mem.contents(),
        r#"
⠉ test1
  test1 line2
  test1 line3
⠉ test2
            "#
        .trim()
    );

    pb2.set_message("test2\n  test2 line2");

    assert_eq!(
        in_mem.contents(),
        r#"
⠉ test1
  test1 line2
  test1 line3
⠉ test2
  test2 line2
            "#
        .trim()
    );

    pb1.set_message("single line again");

    assert_eq!(
        in_mem.contents(),
        r#"
⠉ single line again
⠉ test2
  test2 line2
            "#
        .trim()
    );

    pb1.finish_with_message("test1 done!");
    pb2.finish_with_message("test2 done!");

    assert_eq!(
        in_mem.contents(),
        r#"  test1 done!
  test2 done!"#
    );
}

#[test]
fn multi_progress_bottom_alignment() {
    let in_mem = InMemoryTerm::new(10, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));
    mp.set_alignment(MultiProgressAlignment::Bottom);

    let pb1 = mp.add(ProgressBar::new_spinner().with_message("test1"));
    let pb2 = mp.add(ProgressBar::new_spinner().with_message("test2"));

    pb1.tick();
    pb2.tick();
    pb1.finish_and_clear();

    assert_eq!(in_mem.contents(), "\n⠁ test2");

    pb2.finish_and_clear();
    // `InMemoryTerm::contents` normally gets rid of trailing newlines, so write some text to ensure
    // the newlines are seen.
    in_mem.write_line("anchor").unwrap();
    assert_eq!(in_mem.contents(), "\n\nanchor");
}

#[test]
fn progress_bar_terminal_wrap() {
    use std::cmp::min;
    let in_mem = InMemoryTerm::new(10, 20);

    let mut downloaded = 0;
    let total_size = 231231231;

    let pb = ProgressBar::with_draw_target(
        None,
        ProgressDrawTarget::term_like(Box::new(in_mem.clone())),
    );
    pb.set_style(ProgressStyle::default_bar()
        .template("{msg:>12.cyan.bold} {spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes}").unwrap()
        .progress_chars("#>-"));

    pb.set_message("Downloading");
    assert_eq!(
        in_mem.contents(),
        r#" Downloading ⠁ [00:0
0:00] [-------------
--------------------
-------] 0 B/0 B"#
    );

    let new = min(downloaded + 223211, total_size);
    downloaded = new;
    pb.set_position(new);
    assert_eq!(
        in_mem.contents(),
        r#" Downloading ⠁ [00:0
0:00] [-------------
--------------------
-------] 217.98 KiB/
217.98 KiB"#
    );

    let new = min(downloaded + 223211, total_size);
    pb.set_position(new);
    assert_eq!(
        in_mem.contents(),
        r#" Downloading ⠉ [00:0
0:00] [-------------
--------------------
-------] 435.96 KiB/
435.96 KiB"#
    );

    pb.set_style(
        ProgressStyle::default_bar()
            .template("{msg:>12.green.bold} downloading {total_bytes:.green} in {elapsed:.green}")
            .unwrap(),
    );
    pb.finish_with_message("Finished");
    assert_eq!(
        in_mem.contents(),
        r#"    Finished downloa
ding 435.96 KiB in 0
s"#
    );

    println!("{:?}", in_mem.contents())
}

#[test]
fn spinner_terminal_cleared_log_line_with_ansi_codes() {
    let in_mem = InMemoryTerm::new(10, 100);

    let pb = ProgressBar::with_draw_target(
        Some(10),
        ProgressDrawTarget::term_like(Box::new(in_mem.clone())),
    );
    pb.set_style(ProgressStyle::default_spinner());
    assert_eq!(in_mem.contents(), String::new());

    pb.finish_and_clear();
    // Visually empty, but consists of an ANSII code
    pb.println("\u{1b}[1m");

    pb.println("text\u{1b}[0m");
    assert_eq!(in_mem.contents(), "\ntext");
}

#[test]
fn multi_progress_println_terminal_wrap() {
    let in_mem = InMemoryTerm::new(10, 48);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new(10));
    let pb2 = mp.add(ProgressBar::new(5));
    let pb3 = mp.add(ProgressBar::new(100));

    assert_eq!(in_mem.contents(), "");

    pb1.inc(2);
    mp.println("message printed that is longer than terminal width :)")
        .unwrap();
    assert_eq!(
        in_mem.contents(),
        r#"message printed that is longer than terminal wid
th :)
████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/10"#
    );

    mp.println("another great message!").unwrap();
    assert_eq!(
        in_mem.contents(),
        r#"message printed that is longer than terminal wid
th :)
another great message!
████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/10"#
    );

    pb2.inc(1);
    pb3.tick();
    mp.println("one last message but this one is also longer than terminal width")
        .unwrap();

    assert_eq!(
        in_mem.contents(),
        r#"message printed that is longer than terminal wid
th :)
another great message!
one last message but this one is also longer tha
n terminal width
████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 2/10
████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 1/5
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/100"#
            .trim()
    );

    drop(pb1);
    drop(pb2);
    drop(pb3);

    assert_eq!(
        in_mem.contents(),
        r#"message printed that is longer than terminal wid
th :)
another great message!
one last message but this one is also longer tha
n terminal width"#
            .trim()
    );
}

#[test]
fn basic_progress_bar_newline() {
    let in_mem = InMemoryTerm::new(10, 80);
    let pb = ProgressBar::with_draw_target(
        Some(10),
        ProgressDrawTarget::term_like(Box::new(in_mem.clone())),
    );

    assert_eq!(in_mem.contents(), String::new());

    pb.println("\nhello");
    pb.tick();
    assert_eq!(
        in_mem.contents(),
        r#"
hello
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"#
    );

    pb.inc(1);
    pb.println("");
    assert_eq!(
        in_mem.contents(),
        r#"
hello

███████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 1/10"#
    );

    pb.finish();
    assert_eq!(
        in_mem.contents(),
        "
hello

██████████████████████████████████████████████████████████████████████████ 10/10"
    );
}

#[test]
fn multi_progress_many_bars() {
    let in_mem = InMemoryTerm::new(4, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new(10).with_finish(ProgressFinish::AndLeave));
    let mut spinners = vec![];
    for i in 0..7 {
        let spinner = ProgressBar::new_spinner().with_message(i.to_string());
        mp.add(spinner.clone());
        spinners.push(spinner);
    }

    assert_eq!(in_mem.contents(), String::new());

    pb1.tick();
    assert_eq!(
        in_mem.contents(),
        r#"░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"#
    );
    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
Flush
"#
    );

    for spinner in &spinners {
        spinner.tick()
    }

    assert_eq!(
        in_mem.contents(),
        r#"
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10
⠁ 0
⠁ 1
⠁ 2"#
            .trim_start()
    );
    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Clear
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("                                                                             ")
Flush
Up(1)
Clear
Down(1)
Clear
Up(1)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("                                                                             ")
Flush
Up(2)
Clear
Down(1)
Clear
Down(1)
Clear
Up(2)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Str("                                                                             ")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
"#
    );

    drop(pb1);
    assert_eq!(
        in_mem.contents(),
        r#"
██████████████████████████████████████████████████████████████████████████ 10/10
⠁ 0
⠁ 1
⠁ 2"#
            .trim_start()
    );
    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("██████████████████████████████████████████████████████████████████████████ 10/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
"#
    );

    drop(spinners);

    assert_eq!(in_mem.contents(), r#""#);
    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Up(2)
Clear
Down(1)
Clear
Down(1)
Clear
Up(2)
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Str("")
NewLine
Str("⠁ 3")
Str("")
NewLine
Str("⠁ 4")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("⠁ 2")
Str("")
NewLine
Str("⠁ 3")
Str("")
NewLine
Str("⠁ 4")
Str("")
NewLine
Str("⠁ 5")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("⠁ 3")
Str("")
NewLine
Str("⠁ 4")
Str("")
NewLine
Str("⠁ 5")
Str("")
NewLine
Str("⠁ 6")
Str("                                                                             ")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("⠁ 4")
Str("")
NewLine
Str("⠁ 5")
Str("")
NewLine
Str("⠁ 6")
Str("                                                                             ")
Flush
Up(2)
Clear
Down(1)
Clear
Down(1)
Clear
Up(2)
Str("⠁ 5")
Str("")
NewLine
Str("⠁ 6")
Str("                                                                             ")
Flush
Up(1)
Clear
Down(1)
Clear
Up(1)
Str("⠁ 6")
Str("                                                                             ")
Flush
Clear
Flush
"#
    );
}

#[test]
fn multi_progress_many_spinners() {
    let in_mem = InMemoryTerm::new(4, 80);
    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb1 = mp.add(ProgressBar::new(10).with_finish(ProgressFinish::AndLeave));
    let mut spinners = vec![];
    for i in 0..7 {
        let spinner = ProgressBar::new_spinner().with_message(i.to_string());
        mp.add(spinner.clone());
        spinners.push(spinner);
    }

    assert_eq!(in_mem.contents(), String::new());

    pb1.tick();
    assert_eq!(
        in_mem.contents(),
        r#"░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"#
    );
    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
Flush
"#
    );

    for spinner in &spinners {
        spinner.tick()
    }

    assert_eq!(
        in_mem.contents(),
        r#"
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10
⠁ 0
⠁ 1
⠁ 2"#
            .trim_start()
    );

    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Clear
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("                                                                             ")
Flush
Up(1)
Clear
Down(1)
Clear
Up(1)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("                                                                             ")
Flush
Up(2)
Clear
Down(1)
Clear
Down(1)
Clear
Up(2)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Str("                                                                             ")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
"#
    );

    spinners.remove(3);

    assert_eq!(
        in_mem.contents(),
        r#"
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10
⠁ 0
⠁ 1
⠁ 2"#
            .trim_start()
    );

    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
"#
    );

    spinners.remove(4);

    assert_eq!(
        in_mem.contents(),
        r#"
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10
⠁ 0
⠁ 1
⠁ 2"#
            .trim_start()
    );
    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 0")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Flush
"#
    );

    drop(spinners);

    assert_eq!(
        in_mem.contents(),
        r#"░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10"#
    );
    assert_eq!(
        in_mem.moves_since_last_check(),
        r#"Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 1")
Str("")
NewLine
Str("⠁ 2")
Str("")
NewLine
Str("⠁ 4")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 2")
Str("")
NewLine
Str("⠁ 4")
Str("")
NewLine
Str("⠁ 6")
Str("                                                                             ")
Flush
Up(3)
Clear
Down(1)
Clear
Down(1)
Clear
Down(1)
Clear
Up(3)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 4")
Str("")
NewLine
Str("⠁ 6")
Str("                                                                             ")
Flush
Up(2)
Clear
Down(1)
Clear
Down(1)
Clear
Up(2)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
NewLine
Str("⠁ 6")
Str("                                                                             ")
Flush
Up(1)
Clear
Down(1)
Clear
Up(1)
Str("░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 0/10")
Str("")
Flush
"#
    );
}

#[test]
fn orphan_lines() {
    let in_mem = InMemoryTerm::new(10, 80);

    let pb = ProgressBar::with_draw_target(
        Some(10),
        ProgressDrawTarget::term_like(Box::new(in_mem.clone())),
    );
    assert_eq!(in_mem.contents(), String::new());

    for i in 0..=10 {
        if i != 0 {
            pb.inc(1);
        }

        let n = 5 + i;

        pb.println("\n".repeat(n));
    }

    pb.finish();
}

#[test]
fn orphan_lines_message_above_progress_bar() {
    let in_mem = InMemoryTerm::new(10, 80);

    let pb = ProgressBar::with_draw_target(
        Some(10),
        ProgressDrawTarget::term_like(Box::new(in_mem.clone())),
    );

    orphan_lines_message_above_progress_bar_test(&pb, &in_mem);
}

#[test]
fn orphan_lines_message_above_multi_progress_bar() {
    let in_mem = InMemoryTerm::new(10, 80);

    let mp =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(in_mem.clone())));

    let pb = mp.add(ProgressBar::new(10));

    orphan_lines_message_above_progress_bar_test(&pb, &in_mem);
}

fn orphan_lines_message_above_progress_bar_test(pb: &ProgressBar, in_mem: &InMemoryTerm) {
    assert_eq!(in_mem.contents(), String::new());

    for i in 0..=10 {
        if i != 0 {
            pb.inc(1);
        }

        let n = 5 + i;

        // Test with messages of differing numbers of lines. The messages have the form:
        // n - 1 newlines followed by n * 11 dashes (`-`). The value of n ranges from 5
        // (less than the terminal height) to 15 (greater than the terminal height). The
        // number 11 is intentionally not a factor of the terminal width (80), but large
        // enough that the strings of dashes eventually wrap.
        pb.println(format!("{}{}", "\n".repeat(n - 1), "-".repeat(n * 11)));

        // Check that the line above the progress bar is a string of dashes of length
        // n * 11 mod the terminal width.
        assert_eq!(
            format!("{}", "-".repeat(n * 11 % 80)),
            in_mem.contents().lines().rev().nth(1).unwrap(),
        );
    }

    pb.finish();
}

/// Test proper wrapping of the text lines before a bar is added. #447 on github.
#[test]
fn barless_text_wrapping() {
    let lorem: &str= "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Donec nec viverra massa. Nunc nisl lectus, auctor in lorem eu, maximus elementum est.";

    let in_mem = InMemoryTerm::new(40, 80);
    let mp = indicatif::MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(
        in_mem.clone(),
    )));
    assert_eq!(in_mem.contents(), String::new());

    for _ in 0..=1 {
        mp.println(lorem).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100)); // This is primordial. The bug
                                                                   // came from writing multiple text lines in a row on different ticks.
    }

    assert_eq!(
        in_mem.contents(),
        r#"Lorem ipsum dolor sit amet, consectetur adipiscing elit. Donec nec viverra massa
. Nunc nisl lectus, auctor in lorem eu, maximus elementum est.
Lorem ipsum dolor sit amet, consectetur adipiscing elit. Donec nec viverra massa
. Nunc nisl lectus, auctor in lorem eu, maximus elementum est."#
    );
}
