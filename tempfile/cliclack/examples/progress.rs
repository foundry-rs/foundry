use std::{sync::mpsc::channel, time::Duration};

use cliclack::{clear_screen, intro, log::remark, outro, outro_cancel, progress_bar};
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
    let progress = progress_bar(100);
    progress.start("Copying files...");

    // Simulate doing some stuff....
    for _ in 0..100 {
        // Use a random timeout to simulate some work.
        let timeout = Duration::from_millis(thread_rng().gen_range(10..75));

        // Check if we received a message from the channel.
        if let Ok(Message::Interrupt) = rx.recv_timeout(timeout) {
            // Clear the garbage appearing because of Ctrl-C.
            let term = Term::stderr();
            term.clear_line()?;
            term.move_cursor_up(1)?;

            progress.cancel("Copying files");
            outro_cancel("Interrupted")?;
            return Ok(());
        }

        // Otherwise, we increase the progress bar by the delta 1.
        progress.inc(1);
    }

    // Once we're done, we stop the progress bar and print the outro message.
    // This removes the progress bar and prints the message to the terminal.
    progress.stop("Copying files");
    outro("Done!")?;

    Ok(())
}
