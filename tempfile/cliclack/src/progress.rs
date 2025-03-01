use std::{
    fmt::Display,
    sync::{Arc, RwLock, RwLockWriteGuard},
    time::Duration,
};

use indicatif::ProgressStyle;

use crate::{theme::THEME, ThemeState};

#[derive(Default)]
pub(crate) struct ProgressBarState {
    pub template: String,
    pub grouped: bool,
    pub last: bool,
    pub stopped: bool,
}

/// A progress bar renders progress indication. Supports spinner and download templates.
/// Can be used as a single progress bar and as part of a multi-progress bar
/// (see [`MultiProgress`](crate::multiprogress::MultiProgress)).
///
/// Implemented via theming of [`indicatif::ProgressBar`](https://docs.rs/indicatif).
#[derive(Clone)]
pub struct ProgressBar {
    pub(crate) bar: indicatif::ProgressBar,
    pub(crate) options: Arc<RwLock<ProgressBarState>>,
}

impl ProgressBar {
    /// Creates a new progress bar with a given length.
    pub fn new(len: u64) -> Self {
        let this = Self {
            bar: indicatif::ProgressBar::new(len),
            options: Default::default(),
        };

        this.options_write().template = THEME.lock().unwrap().default_progress_template();

        this
    }

    /// Sets a default spinner visual template for the progress bar.
    pub fn with_spinner_template(self) -> Self {
        self.options_write().template = THEME.lock().unwrap().default_spinner_template();
        self
    }

    /// Sets a default visual template for downloading.
    pub fn with_download_template(self) -> Self {
        self.options_write().template = THEME.lock().unwrap().default_download_template();
        self
    }

    /// Sets a custom template string for the progress bar according to
    /// [`indicatif::ProgressStyle`](https://docs.rs/indicatif/latest/indicatif/#templates).
    pub fn with_template(self, template: &str) -> Self {
        self.options_write().template = template.to_string();
        self
    }

    /// Returns the current position.
    pub fn position(&self) -> u64 {
        self.bar.position()
    }

    /// Returns the current length.
    pub fn length(&self) -> Option<u64> {
        self.bar.length()
    }

    /// Advances the position of the progress bar by a delta.
    pub fn inc(&self, delta: u64) {
        self.bar.inc(delta)
    }

    /// Indicates that the progress bar is finished.
    pub fn is_finished(&self) -> bool {
        self.options().stopped
    }

    /// Sets the current message of the progress bar
    /// (`{msg}` placeholder must be present in the template if you're using
    /// a custom template via [`ProgressBar::with_template`]).
    pub fn set_message(&self, message: impl Display) {
        self.bar.set_message(message.to_string());
    }

    /// Sets the length of the progress bar.
    pub fn set_length(&self, len: u64) {
        self.bar.set_length(len);
    }

    /// Starts the progress bar.
    pub fn start(&self, message: impl Display) {
        let theme = THEME.lock().unwrap();
        let options = self.options();

        self.bar.set_style(
            ProgressStyle::with_template(&theme.format_progress_start(
                &options.template,
                options.grouped,
                options.last,
            ))
            .unwrap()
            .tick_chars(&theme.spinner_chars())
            .progress_chars(&theme.progress_chars()),
        );

        self.bar
            .set_message(theme.format_progress_message(&message.to_string()));
        self.bar.enable_steady_tick(Duration::from_millis(100));
    }

    /// Stops the progress bar.
    pub fn stop(&self, message: impl Display) {
        self.finish_with_state(message, &ThemeState::Submit);
    }

    /// Cancel the progress bar (stop with a cancelling style).
    pub fn cancel(&self, message: impl Display) {
        self.finish_with_state(message, &ThemeState::Cancel);
    }

    /// Makes the progress bar stop with an error.
    pub fn error(&self, message: impl Display) {
        self.finish_with_state(message, &ThemeState::Error("".into()));
    }

    /// Clears the progress bar.
    pub fn clear(&self) {
        self.finish_with_state("", &ThemeState::Submit);
    }

    /// Accesses the options for writing (a convenience function).
    #[inline]
    pub(crate) fn options_write(&self) -> RwLockWriteGuard<'_, ProgressBarState> {
        self.options.write().unwrap()
    }

    /// Accesses the options for reading (a convenience function).
    #[inline]
    pub(crate) fn options(&self) -> RwLockWriteGuard<'_, ProgressBarState> {
        self.options.write().unwrap()
    }

    /// Redraws the progress bar with a new message. Returns the number of lines printed.
    ///
    /// The method is semi-open for multi-progress bar purposes.
    pub(crate) fn redraw_finished(&self, message: impl Display, state: &ThemeState) -> usize {
        let theme = THEME.lock().unwrap();
        let options = self.options.read().unwrap();

        let render = theme.format_progress_with_state(
            &message.to_string(),
            options.grouped,
            options.last,
            state,
        );

        // Ignore a cleared progress bar.
        if !message.to_string().is_empty() {
            self.bar.println(render.clone());
        }

        render.lines().count()
    }

    /// Redraws the progress bar.
    pub(crate) fn redraw_active(&self) {
        if !self.options().stopped {
            self.redraw_active_as_started();
        } else {
            self.redraw_active_as_stopped();
        }
    }

    /// Redraws the progress bar without changing the message.
    fn redraw_active_as_started(&self) {
        let theme = THEME.lock().unwrap();
        let options = self.options();

        self.bar.set_style(
            ProgressStyle::with_template(&theme.format_progress_start(
                &options.template,
                options.grouped,
                options.last,
            ))
            .unwrap()
            .tick_chars(&theme.spinner_chars())
            .progress_chars(&theme.progress_chars()),
        );
    }

    /// Redraws the progress bar without changing the message.
    fn redraw_active_as_stopped(&self) {
        let theme = THEME.lock().unwrap();
        let options = self.options();

        self.bar.set_style(
            ProgressStyle::with_template(&theme.format_progress_with_state(
                &self.bar.message(),
                options.grouped,
                options.last,
                &ThemeState::Active,
            ))
            .unwrap(),
        );
    }

    fn finish_with_state(&self, message: impl Display, state: &ThemeState) {
        if self.options().stopped {
            return;
        }

        self.options_write().stopped = true;

        if !self.options().grouped {
            // Workaround: `bar.println` must be before `bar.finish_and_clear` to avoid "jumping"
            // of the printed line while resizing the terminal.
            self.redraw_finished(message, state);
            self.bar.finish_and_clear();
        } else {
            // Don't actually stop the indicatif progress bar.
            self.bar.set_message(message.to_string());
            self.redraw_active_as_stopped();
        }
    }
}
