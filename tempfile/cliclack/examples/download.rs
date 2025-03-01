use std::{sync::mpsc::channel, time::Duration};

use cliclack::{clear_screen, intro, log::remark, outro, outro_cancel, progress_bar};
use console::{style, Term};
use rand::{thread_rng, Rng};

enum Message {
    Interrupt,
}

// Total number of bytes to simulate downloading.
const TOTAL_BYTES: u64 = 5_000_000;

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
    intro(style(" download ").on_cyan().black())?;
    remark("Press Ctrl-C")?;

    // Create a new progress bar and set the text to "Installation".
    let download = progress_bar(TOTAL_BYTES).with_download_template();
    download.start("Downloading, please wait...");

    // Loop until the progress bar reaches the total number of bytes
    while download.position() < TOTAL_BYTES {
        // Use a random timeout to simulate some work.
        let timeout = Duration::from_millis(thread_rng().gen_range(10..150));

        // Check if we received a message from the channel.
        if let Ok(Message::Interrupt) = rx.recv_timeout(timeout) {
            // Clear the garbage appearing because of Ctrl-C.
            let term = Term::stderr();
            term.clear_line()?;
            term.move_cursor_up(1)?;

            download.cancel("Downloading");
            outro_cancel("Interrupted")?;
            return Ok(());
        }

        // Increment the progress bar with a random number of bytes.
        download.inc(thread_rng().gen_range(1_000..200_000));
    }

    // Once we're done, we stop the progress bar and print the outro message.
    // This removes the progress bar and prints the message to the terminal.
    download.stop("Downloading");
    outro("Done!")?;

    Ok(())
}
