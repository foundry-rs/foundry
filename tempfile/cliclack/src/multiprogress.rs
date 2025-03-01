use std::{
    fmt::Display,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
};

use console::Term;

use crate::{progress::ProgressBar, theme::THEME, ThemeState};

const HEADER_HEIGHT: usize = 1;

/// Renders other progress bars and spinners under a common header in a single visual block.
#[derive(Clone)]
pub struct MultiProgress {
    multi: indicatif::MultiProgress,
    bars: Arc<RwLock<Vec<ProgressBar>>>,
    prompt: String,
    logs: Arc<AtomicUsize>,
}

impl MultiProgress {
    /// Creates a new multi-progress bar with a given prompt.
    pub fn new(prompt: impl Display) -> Self {
        let theme = THEME.lock().unwrap();
        let multi = indicatif::MultiProgress::new();

        let header =
            theme.format_header(&ThemeState::Active, (prompt.to_string() + "\n ").trim_end());

        multi.println(header).ok();

        Self {
            multi,
            bars: Default::default(),
            prompt: prompt.to_string(),
            logs: Default::default(),
        }
    }

    /// Adds a progress bar and returns an internalized reference to it.
    ///
    /// The progress bar will be positioned below all other bars in the [`MultiProgress`].
    pub fn add(&self, pb: ProgressBar) -> ProgressBar {
        let bars_count = self.bars.read().unwrap().len();
        self.insert(bars_count, pb)
    }

    /// Inserts a progress bar at a given index and returns an internalized reference to it.
    ///
    /// If the index is greater than or equal to the number of progress bars, the bar is added to the end.
    pub fn insert(&self, index: usize, pb: ProgressBar) -> ProgressBar {
        let bars_count = self.bars.read().unwrap().len();
        let index = index.min(bars_count);
        if index == bars_count {
            // Unset the last flag for all other progress bars: it affects rendering.
            for bar in self.bars.write().unwrap().iter_mut() {
                bar.options_write().last = false;
                bar.redraw_active();
            }
        }
        // Attention: deconstructing `pb` to avoid borrowing `pb.bar` twice.
        let ProgressBar { bar, options } = pb;
        let bar = self.multi.insert(index, bar);
        {
            let mut options = options.write().unwrap();
            options.grouped = true;
            if index == bars_count {
                options.last = true;
            }
        }

        let pb = ProgressBar { bar, options };
        self.bars.write().unwrap().insert(index, pb.clone());
        pb
    }

    /// Prints a log line above the multi-progress bar.
    ///
    /// By default, there is no empty line between each log added with
    /// this function. To add an empty line, use a line
    /// return character (`\n`) at the end of the message.
    pub fn println(&self, message: impl Display) {
        let theme = THEME.lock().unwrap();
        let symbol = theme.remark_symbol();
        let log = theme.format_log_with_spacing(&message.to_string(), &symbol, false);
        self.logs.fetch_add(log.lines().count(), Ordering::SeqCst);
        self.multi.println(log).ok();
    }

    /// Stops the multi-progress bar with a submitted (successful) state.
    pub fn stop(&self) {
        self.stop_with(&ThemeState::Submit)
    }

    /// Stops the multi-progress bar with a default cancel message.
    pub fn cancel(&self) {
        self.stop_with(&ThemeState::Cancel)
    }

    /// Stops the multi-progress bar with an error message.
    pub fn error(&self, error: impl Display) {
        self.stop_with(&ThemeState::Error(error.to_string()))
    }

    fn stop_with(&self, state: &ThemeState) {
        let mut inner_height = self.logs.load(Ordering::SeqCst);

        // Redraw all progress bars.
        for pb in self.bars.read().unwrap().iter() {
            // Workaround: `bar.println` must be called before `bar.finish_and_clear`
            // to avoid lines "jumping" while terminal resizing.
            inner_height += pb.redraw_finished(pb.bar.message(), state);
            pb.bar.finish_and_clear();
        }

        let term = Term::stderr();

        // Move up to the header, clear and print the new header, then move down.
        term.move_cursor_up(inner_height).ok();
        term.clear_last_lines(HEADER_HEIGHT).ok();
        term.write_str(
            &THEME
                .lock()
                .unwrap()
                .format_header(state, (self.prompt.clone() + "\n ").trim_end()),
        )
        .ok();
        term.move_cursor_down(inner_height).ok();
    }
}
