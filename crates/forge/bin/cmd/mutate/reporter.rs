// use foundry_common::term::Spinner;
// use indicatif::{ProgressBar, ProgressStyle};
// use std::{
//     sync::mpsc::{self, TryRecvError},
//     thread,
//     time::Duration,
// };

// /// This reporter will prefix messages with a spinning cursor
// #[derive(Debug)]
// pub struct MutateSpinnerReporter {
//     /// The sender to the spinner thread.
//     sender: mpsc::Sender<SpinnerMsg>,
// }

// impl MutateSpinnerReporter {
//     pub fn new(message: &str) -> Self {
//         let progress_bar: ProgressBar = ProgressBar::new_spinner();
//         progress_bar.enable_steady_tick(Duration::from_millis(120));
//         progress_bar.set_style(
//             ProgressStyle::with_template("{spinner:.bold} {msg}\n")
//                 .unwrap()
//                 .tick_strings(&[" ", "▖", "▘", "▀", "▜", "▟", "▄", "█"]),
//         );

//         progress_bar.set_message(message.to_string());

//         Self { progress_bar }
//     }

//     pub fn message(&self, msg: String) {
//         self.progress_bar.set_message(msg);
//     }

//     pub fn finish(self) {
//         self.progress_bar.finish_and_clear();
//     }
// }
