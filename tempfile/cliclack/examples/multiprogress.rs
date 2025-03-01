use std::{sync::mpsc::channel, time::Duration};

use cliclack::{clear_screen, intro, log::remark, multi_progress, outro, progress_bar, spinner};
use console::{style, Term};
use rand::{thread_rng, Rng};

enum Message {
    Interrupt,
}

fn main() -> std::io::Result<()> {
    let (tx, rx) = channel();

    // Set a no-op Ctrl-C handler which allows to catch
    // `ErrorKind::Interrupted` error on `term.read_key()`.
    ctrlc::set_handler(move || {
        tx.send(Message::Interrupt).ok();
    })
    .expect("setting Ctrl-C handler");

    // Clear the screen and print the header.
    clear_screen()?;
    intro(style(" progress bar ").on_cyan().black())?;
    remark("Press Ctrl-C")?;

    // Create a new progress bar and set the text to "Installation".
    let multi = multi_progress("Doing stuff...");

    let spinner = multi.add(spinner());
    spinner.start("Establish connection...");
    std::thread::sleep(Duration::from_secs(1));
    spinner.clear();

    let pb1 = multi.add(progress_bar(500));
    let pb2 = multi.add(progress_bar(500));

    pb1.start("Downloading files...");
    pb2.start("Copying files...");

    // Simulate doing some stuff....
    while !pb1.is_finished() || !pb2.is_finished() {
        // Use a random timeout to simulate some work.
        let timeout = Duration::from_millis(thread_rng().gen_range(10..75));

        // Check if we received a message from the channel.
        if let Ok(Message::Interrupt) = rx.recv_timeout(timeout) {
            // Clear the garbage appearing because of Ctrl-C.
            let term = Term::stderr();
            term.clear_line()?;
            term.move_cursor_up(1)?;

            pb1.cancel(format!("{} Downloading files", style("✘").red()));
            pb2.cancel(format!("{} Copying files", style("✘").red()));
            multi.cancel();
            return Ok(());
        }

        if pb1.position() < pb1.length().unwrap() {
            pb1.inc(thread_rng().gen_range(1..20));
        } else if !pb1.is_finished() {
            pb1.stop(format!("{} Downloading files", style("✔").green()));
        }

        if pb2.position() < pb2.length().unwrap() {
            pb2.inc(thread_rng().gen_range(1..13));
        } else if !pb2.is_finished() {
            pb2.stop(format!("{} Copying files", style("✔").green()));
        }
    }

    multi.stop();
    outro("Done!")?;

    Ok(())
}
