#[cfg(test)]
use portable_atomic::{AtomicBool, Ordering};
use std::borrow::Cow;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
use std::{fmt, io, thread};

#[cfg(test)]
use once_cell::sync::Lazy;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::draw_target::ProgressDrawTarget;
use crate::state::{AtomicPosition, BarState, ProgressFinish, Reset, TabExpandedString};
use crate::style::ProgressStyle;
use crate::{ProgressBarIter, ProgressIterator, ProgressState};

/// A progress bar or spinner
///
/// The progress bar is an [`Arc`] around its internal state. When the progress bar is cloned it
/// just increments the refcount (so the original and its clone share the same state).
#[derive(Clone)]
pub struct ProgressBar {
    state: Arc<Mutex<BarState>>,
    pos: Arc<AtomicPosition>,
    ticker: Arc<Mutex<Option<Ticker>>>,
}

impl fmt::Debug for ProgressBar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProgressBar").finish()
    }
}

impl ProgressBar {
    /// Creates a new progress bar with a given length
    ///
    /// This progress bar by default draws directly to stderr, and refreshes a maximum of 20 times
    /// a second. To change the refresh rate, [set] the [draw target] to one with a different refresh
    /// rate.
    ///
    /// [set]: ProgressBar::set_draw_target
    /// [draw target]: ProgressDrawTarget
    pub fn new(len: u64) -> Self {
        Self::with_draw_target(Some(len), ProgressDrawTarget::stderr())
    }

    /// Creates a new progress bar without a specified length
    ///
    /// This progress bar by default draws directly to stderr, and refreshes a maximum of 20 times
    /// a second. To change the refresh rate, [set] the [draw target] to one with a different refresh
    /// rate.
    ///
    /// [set]: ProgressBar::set_draw_target
    /// [draw target]: ProgressDrawTarget
    pub fn no_length() -> Self {
        Self::with_draw_target(None, ProgressDrawTarget::stderr())
    }

    /// Creates a completely hidden progress bar
    ///
    /// This progress bar still responds to API changes but it does not have a length or render in
    /// any way.
    pub fn hidden() -> Self {
        Self::with_draw_target(None, ProgressDrawTarget::hidden())
    }

    /// Creates a new progress bar with a given length and draw target
    pub fn with_draw_target(len: Option<u64>, draw_target: ProgressDrawTarget) -> Self {
        let pos = Arc::new(AtomicPosition::new());
        Self {
            state: Arc::new(Mutex::new(BarState::new(len, draw_target, pos.clone()))),
            pos,
            ticker: Arc::new(Mutex::new(None)),
        }
    }

    /// Get a clone of the current progress bar style.
    pub fn style(&self) -> ProgressStyle {
        self.state().style.clone()
    }

    /// A convenience builder-like function for a progress bar with a given style
    pub fn with_style(self, style: ProgressStyle) -> Self {
        self.set_style(style);
        self
    }

    /// A convenience builder-like function for a progress bar with a given tab width
    pub fn with_tab_width(self, tab_width: usize) -> Self {
        self.state().set_tab_width(tab_width);
        self
    }

    /// A convenience builder-like function for a progress bar with a given prefix
    ///
    /// For the prefix to be visible, the `{prefix}` placeholder must be present in the template
    /// (see [`ProgressStyle`]).
    pub fn with_prefix(self, prefix: impl Into<Cow<'static, str>>) -> Self {
        let mut state = self.state();
        state.state.prefix = TabExpandedString::new(prefix.into(), state.tab_width);
        drop(state);
        self
    }

    /// A convenience builder-like function for a progress bar with a given message
    ///
    /// For the message to be visible, the `{msg}` placeholder must be present in the template (see
    /// [`ProgressStyle`]).
    pub fn with_message(self, message: impl Into<Cow<'static, str>>) -> Self {
        let mut state = self.state();
        state.state.message = TabExpandedString::new(message.into(), state.tab_width);
        drop(state);
        self
    }

    /// A convenience builder-like function for a progress bar with a given position
    pub fn with_position(self, pos: u64) -> Self {
        self.state().state.set_pos(pos);
        self
    }

    /// A convenience builder-like function for a progress bar with a given elapsed time
    pub fn with_elapsed(self, elapsed: Duration) -> Self {
        self.state().state.started = Instant::now().checked_sub(elapsed).unwrap();
        self
    }

    /// Sets the finish behavior for the progress bar
    ///
    /// This behavior is invoked when [`ProgressBar`] or
    /// [`ProgressBarIter`] completes and
    /// [`ProgressBar::is_finished()`] is false.
    /// If you don't want the progress bar to be automatically finished then
    /// call `with_finish(Abandon)`.
    ///
    /// [`ProgressBar`]: crate::ProgressBar
    /// [`ProgressBarIter`]: crate::ProgressBarIter
    /// [`ProgressBar::is_finished()`]: crate::ProgressBar::is_finished
    pub fn with_finish(self, finish: ProgressFinish) -> Self {
        self.state().on_finish = finish;
        self
    }

    /// Creates a new spinner
    ///
    /// This spinner by default draws directly to stderr. This adds the default spinner style to it.
    pub fn new_spinner() -> Self {
        let rv = Self::with_draw_target(None, ProgressDrawTarget::stderr());
        rv.set_style(ProgressStyle::default_spinner());
        rv
    }

    /// Overrides the stored style
    ///
    /// This does not redraw the bar. Call [`ProgressBar::tick()`] to force it.
    pub fn set_style(&self, style: ProgressStyle) {
        self.state().set_style(style);
    }

    /// Sets the tab width (default: 8). All tabs will be expanded to this many spaces.
    pub fn set_tab_width(&self, tab_width: usize) {
        let mut state = self.state();
        state.set_tab_width(tab_width);
        state.draw(true, Instant::now()).unwrap();
    }

    /// Spawns a background thread to tick the progress bar
    ///
    /// When this is enabled a background thread will regularly tick the progress bar in the given
    /// interval. This is useful to advance progress bars that are very slow by themselves.
    ///
    /// When steady ticks are enabled, calling [`ProgressBar::tick()`] on a progress bar does not
    /// have any effect.
    pub fn enable_steady_tick(&self, interval: Duration) {
        // The way we test for ticker termination is with a single static `AtomicBool`. Since cargo
        // runs tests concurrently, we have a `TICKER_TEST` lock to make sure tests using ticker
        // don't step on each other. This check catches attempts to use tickers in tests without
        // acquiring the lock.
        #[cfg(test)]
        {
            let guard = TICKER_TEST.try_lock();
            let lock_acquired = guard.is_ok();
            // Drop the guard before panicking to avoid poisoning the lock (which would cause other
            // ticker tests to fail)
            drop(guard);
            if lock_acquired {
                panic!("you must acquire the TICKER_TEST lock in your test to use this method");
            }
        }

        if interval.is_zero() {
            return;
        }

        self.stop_and_replace_ticker(Some(interval));
    }

    /// Undoes [`ProgressBar::enable_steady_tick()`]
    pub fn disable_steady_tick(&self) {
        self.stop_and_replace_ticker(None);
    }

    fn stop_and_replace_ticker(&self, interval: Option<Duration>) {
        let mut ticker_state = self.ticker.lock().unwrap();
        if let Some(ticker) = ticker_state.take() {
            ticker.stop();
        }

        *ticker_state = interval.map(|interval| Ticker::new(interval, &self.state));
    }

    /// Manually ticks the spinner or progress bar
    ///
    /// This automatically happens on any other change to a progress bar.
    pub fn tick(&self) {
        self.tick_inner(Instant::now());
    }

    fn tick_inner(&self, now: Instant) {
        // Only tick if a `Ticker` isn't installed
        if self.ticker.lock().unwrap().is_none() {
            self.state().tick(now);
        }
    }

    /// Advances the position of the progress bar by `delta`
    pub fn inc(&self, delta: u64) {
        self.pos.inc(delta);
        let now = Instant::now();
        if self.pos.allow(now) {
            self.tick_inner(now);
        }
    }

    /// Decrease the position of the progress bar by `delta`
    pub fn dec(&self, delta: u64) {
        self.pos.dec(delta);
        let now = Instant::now();
        if self.pos.allow(now) {
            self.tick_inner(now);
        }
    }

    /// A quick convenience check if the progress bar is hidden
    pub fn is_hidden(&self) -> bool {
        self.state().draw_target.is_hidden()
    }

    /// Indicates that the progress bar finished
    pub fn is_finished(&self) -> bool {
        self.state().state.is_finished()
    }

    /// Print a log line above the progress bar
    ///
    /// If the progress bar is hidden (e.g. when standard output is not a terminal), `println()`
    /// will not do anything. If you want to write to the standard output in such cases as well, use
    /// [`ProgressBar::suspend()`] instead.
    ///
    /// If the progress bar was added to a [`MultiProgress`], the log line will be
    /// printed above all other progress bars.
    ///
    /// [`ProgressBar::suspend()`]: ProgressBar::suspend
    /// [`MultiProgress`]: crate::MultiProgress
    pub fn println<I: AsRef<str>>(&self, msg: I) {
        self.state().println(Instant::now(), msg.as_ref());
    }

    /// Update the `ProgressBar`'s inner [`ProgressState`]
    pub fn update(&self, f: impl FnOnce(&mut ProgressState)) {
        self.state()
            .update(Instant::now(), f, self.ticker.lock().unwrap().is_none());
    }

    /// Sets the position of the progress bar
    pub fn set_position(&self, pos: u64) {
        self.pos.set(pos);
        let now = Instant::now();
        if self.pos.allow(now) {
            self.tick_inner(now);
        }
    }

    /// Sets the length of the progress bar to `None`
    pub fn unset_length(&self) {
        self.state().unset_length(Instant::now());
    }

    /// Sets the length of the progress bar
    pub fn set_length(&self, len: u64) {
        self.state().set_length(Instant::now(), len);
    }

    /// Increase the length of the progress bar
    pub fn inc_length(&self, delta: u64) {
        self.state().inc_length(Instant::now(), delta);
    }

    /// Decrease the length of the progress bar
    pub fn dec_length(&self, delta: u64) {
        self.state().dec_length(Instant::now(), delta);
    }

    /// Sets the current prefix of the progress bar
    ///
    /// For the prefix to be visible, the `{prefix}` placeholder must be present in the template
    /// (see [`ProgressStyle`]).
    pub fn set_prefix(&self, prefix: impl Into<Cow<'static, str>>) {
        let mut state = self.state();
        state.state.prefix = TabExpandedString::new(prefix.into(), state.tab_width);
        state.update_estimate_and_draw(Instant::now());
    }

    /// Sets the current message of the progress bar
    ///
    /// For the message to be visible, the `{msg}` placeholder must be present in the template (see
    /// [`ProgressStyle`]).
    pub fn set_message(&self, msg: impl Into<Cow<'static, str>>) {
        let mut state = self.state();
        state.state.message = TabExpandedString::new(msg.into(), state.tab_width);
        state.update_estimate_and_draw(Instant::now());
    }

    /// Creates a new weak reference to this [`ProgressBar`]
    pub fn downgrade(&self) -> WeakProgressBar {
        WeakProgressBar {
            state: Arc::downgrade(&self.state),
            pos: Arc::downgrade(&self.pos),
            ticker: Arc::downgrade(&self.ticker),
        }
    }

    /// Resets the ETA calculation
    ///
    /// This can be useful if the progress bars made a large jump or was paused for a prolonged
    /// time.
    pub fn reset_eta(&self) {
        self.state().reset(Instant::now(), Reset::Eta);
    }

    /// Resets elapsed time and the ETA calculation
    pub fn reset_elapsed(&self) {
        self.state().reset(Instant::now(), Reset::Elapsed);
    }

    /// Resets all of the progress bar state
    pub fn reset(&self) {
        self.state().reset(Instant::now(), Reset::All);
    }

    /// Finishes the progress bar and leaves the current message
    pub fn finish(&self) {
        self.state()
            .finish_using_style(Instant::now(), ProgressFinish::AndLeave);
    }

    /// Finishes the progress bar and sets a message
    ///
    /// For the message to be visible, the `{msg}` placeholder must be present in the template (see
    /// [`ProgressStyle`]).
    pub fn finish_with_message(&self, msg: impl Into<Cow<'static, str>>) {
        self.state()
            .finish_using_style(Instant::now(), ProgressFinish::WithMessage(msg.into()));
    }

    /// Finishes the progress bar and completely clears it
    pub fn finish_and_clear(&self) {
        self.state()
            .finish_using_style(Instant::now(), ProgressFinish::AndClear);
    }

    /// Finishes the progress bar and leaves the current message and progress
    pub fn abandon(&self) {
        self.state()
            .finish_using_style(Instant::now(), ProgressFinish::Abandon);
    }

    /// Finishes the progress bar and sets a message, and leaves the current progress
    ///
    /// For the message to be visible, the `{msg}` placeholder must be present in the template (see
    /// [`ProgressStyle`]).
    pub fn abandon_with_message(&self, msg: impl Into<Cow<'static, str>>) {
        self.state().finish_using_style(
            Instant::now(),
            ProgressFinish::AbandonWithMessage(msg.into()),
        );
    }

    /// Finishes the progress bar using the behavior stored in the [`ProgressStyle`]
    ///
    /// See [`ProgressBar::with_finish()`].
    pub fn finish_using_style(&self) {
        let mut state = self.state();
        let finish = state.on_finish.clone();
        state.finish_using_style(Instant::now(), finish);
    }

    /// Sets a different draw target for the progress bar
    ///
    /// This can be used to draw the progress bar to stderr (this is the default):
    ///
    /// ```rust,no_run
    /// # use indicatif::{ProgressBar, ProgressDrawTarget};
    /// let pb = ProgressBar::new(100);
    /// pb.set_draw_target(ProgressDrawTarget::stderr());
    /// ```
    ///
    /// **Note:** Calling this method on a [`ProgressBar`] linked with a [`MultiProgress`] (after
    /// running [`MultiProgress::add()`]) will unlink this progress bar. If you don't want this
    /// behavior, call [`MultiProgress::set_draw_target()`] instead.
    ///
    /// Use [`ProgressBar::with_draw_target()`] to set the draw target during creation.
    ///
    /// [`MultiProgress`]: crate::MultiProgress
    /// [`MultiProgress::add()`]: crate::MultiProgress::add
    /// [`MultiProgress::set_draw_target()`]: crate::MultiProgress::set_draw_target
    pub fn set_draw_target(&self, target: ProgressDrawTarget) {
        let mut state = self.state();
        state.draw_target.disconnect(Instant::now());
        state.draw_target = target;
    }

    /// Hide the progress bar temporarily, execute `f`, then redraw the progress bar
    ///
    /// Useful for external code that writes to the standard output.
    ///
    /// If the progress bar was added to a [`MultiProgress`], it will suspend the entire [`MultiProgress`].
    ///
    /// **Note:** The internal lock is held while `f` is executed. Other threads trying to print
    /// anything on the progress bar will be blocked until `f` finishes.
    /// Therefore, it is recommended to avoid long-running operations in `f`.
    ///
    /// ```rust,no_run
    /// # use indicatif::ProgressBar;
    /// let mut pb = ProgressBar::new(3);
    /// pb.suspend(|| {
    ///     println!("Log message");
    /// })
    /// ```
    ///
    /// [`MultiProgress`]: crate::MultiProgress
    pub fn suspend<F: FnOnce() -> R, R>(&self, f: F) -> R {
        self.state().suspend(Instant::now(), f)
    }

    /// Wraps an [`Iterator`] with the progress bar
    ///
    /// ```rust,no_run
    /// # use indicatif::ProgressBar;
    /// let v = vec![1, 2, 3];
    /// let pb = ProgressBar::new(3);
    /// for item in pb.wrap_iter(v.iter()) {
    ///     // ...
    /// }
    /// ```
    pub fn wrap_iter<It: Iterator>(&self, it: It) -> ProgressBarIter<It> {
        it.progress_with(self.clone())
    }

    /// Wraps an [`io::Read`] with the progress bar
    ///
    /// ```rust,no_run
    /// # use std::fs::File;
    /// # use std::io;
    /// # use indicatif::ProgressBar;
    /// # fn test () -> io::Result<()> {
    /// let source = File::open("work.txt")?;
    /// let mut target = File::create("done.txt")?;
    /// let pb = ProgressBar::new(source.metadata()?.len());
    /// io::copy(&mut pb.wrap_read(source), &mut target);
    /// # Ok(())
    /// # }
    /// ```
    pub fn wrap_read<R: io::Read>(&self, read: R) -> ProgressBarIter<R> {
        ProgressBarIter {
            progress: self.clone(),
            it: read,
        }
    }

    /// Wraps an [`io::Write`] with the progress bar
    ///
    /// ```rust,no_run
    /// # use std::fs::File;
    /// # use std::io;
    /// # use indicatif::ProgressBar;
    /// # fn test () -> io::Result<()> {
    /// let mut source = File::open("work.txt")?;
    /// let target = File::create("done.txt")?;
    /// let pb = ProgressBar::new(source.metadata()?.len());
    /// io::copy(&mut source, &mut pb.wrap_write(target));
    /// # Ok(())
    /// # }
    /// ```
    pub fn wrap_write<W: io::Write>(&self, write: W) -> ProgressBarIter<W> {
        ProgressBarIter {
            progress: self.clone(),
            it: write,
        }
    }

    #[cfg(feature = "tokio")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
    /// Wraps an [`tokio::io::AsyncWrite`] with the progress bar
    ///
    /// ```rust,no_run
    /// # use tokio::fs::File;
    /// # use tokio::io;
    /// # use indicatif::ProgressBar;
    /// # async fn test() -> io::Result<()> {
    /// let mut source = File::open("work.txt").await?;
    /// let mut target = File::open("done.txt").await?;
    /// let pb = ProgressBar::new(source.metadata().await?.len());
    /// io::copy(&mut source, &mut pb.wrap_async_write(target)).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn wrap_async_write<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        write: W,
    ) -> ProgressBarIter<W> {
        ProgressBarIter {
            progress: self.clone(),
            it: write,
        }
    }

    #[cfg(feature = "tokio")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
    /// Wraps an [`tokio::io::AsyncRead`] with the progress bar
    ///
    /// ```rust,no_run
    /// # use tokio::fs::File;
    /// # use tokio::io;
    /// # use indicatif::ProgressBar;
    /// # async fn test() -> io::Result<()> {
    /// let mut source = File::open("work.txt").await?;
    /// let mut target = File::open("done.txt").await?;
    /// let pb = ProgressBar::new(source.metadata().await?.len());
    /// io::copy(&mut pb.wrap_async_read(source), &mut target).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn wrap_async_read<R: tokio::io::AsyncRead + Unpin>(&self, read: R) -> ProgressBarIter<R> {
        ProgressBarIter {
            progress: self.clone(),
            it: read,
        }
    }

    /// Wraps a [`futures::Stream`](https://docs.rs/futures/0.3/futures/stream/trait.StreamExt.html) with the progress bar
    ///
    /// ```
    /// # use indicatif::ProgressBar;
    /// # futures::executor::block_on(async {
    /// use futures::stream::{self, StreamExt};
    /// let pb = ProgressBar::new(10);
    /// let mut stream = pb.wrap_stream(stream::iter('a'..='z'));
    ///
    /// assert_eq!(stream.next().await, Some('a'));
    /// assert_eq!(stream.count().await, 25);
    /// # }); // block_on
    /// ```
    #[cfg(feature = "futures")]
    #[cfg_attr(docsrs, doc(cfg(feature = "futures")))]
    pub fn wrap_stream<S: futures_core::Stream>(&self, stream: S) -> ProgressBarIter<S> {
        ProgressBarIter {
            progress: self.clone(),
            it: stream,
        }
    }

    /// Returns the current position
    pub fn position(&self) -> u64 {
        self.state().state.pos()
    }

    /// Returns the current length
    pub fn length(&self) -> Option<u64> {
        self.state().state.len()
    }

    /// Returns the current ETA
    pub fn eta(&self) -> Duration {
        self.state().state.eta()
    }

    /// Returns the current rate of progress
    pub fn per_sec(&self) -> f64 {
        self.state().state.per_sec()
    }

    /// Returns the current expected duration
    pub fn duration(&self) -> Duration {
        self.state().state.duration()
    }

    /// Returns the current elapsed time
    pub fn elapsed(&self) -> Duration {
        self.state().state.elapsed()
    }

    /// Index in the `MultiState`
    pub(crate) fn index(&self) -> Option<usize> {
        self.state().draw_target.remote().map(|(_, idx)| idx)
    }

    /// Current message
    pub fn message(&self) -> String {
        self.state().state.message.expanded().to_string()
    }

    /// Current prefix
    pub fn prefix(&self) -> String {
        self.state().state.prefix.expanded().to_string()
    }

    #[inline]
    pub(crate) fn state(&self) -> MutexGuard<'_, BarState> {
        self.state.lock().unwrap()
    }
}

/// A weak reference to a [`ProgressBar`].
///
/// Useful for creating custom steady tick implementations
#[derive(Clone, Default)]
pub struct WeakProgressBar {
    state: Weak<Mutex<BarState>>,
    pos: Weak<AtomicPosition>,
    ticker: Weak<Mutex<Option<Ticker>>>,
}

impl WeakProgressBar {
    /// Create a new [`WeakProgressBar`] that returns `None` when [`upgrade()`] is called.
    ///
    /// [`upgrade()`]: WeakProgressBar::upgrade
    pub fn new() -> Self {
        Self::default()
    }

    /// Attempts to upgrade the Weak pointer to a [`ProgressBar`], delaying dropping of the inner
    /// value if successful. Returns [`None`] if the inner value has since been dropped.
    ///
    /// [`ProgressBar`]: struct.ProgressBar.html
    pub fn upgrade(&self) -> Option<ProgressBar> {
        let state = self.state.upgrade()?;
        let pos = self.pos.upgrade()?;
        let ticker = self.ticker.upgrade()?;
        Some(ProgressBar { state, pos, ticker })
    }
}

pub(crate) struct Ticker {
    stopping: Arc<(Mutex<bool>, Condvar)>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop();
        self.join_handle.take().map(|handle| handle.join());
    }
}

#[cfg(test)]
static TICKER_RUNNING: AtomicBool = AtomicBool::new(false);

impl Ticker {
    pub(crate) fn new(interval: Duration, bar_state: &Arc<Mutex<BarState>>) -> Self {
        debug_assert!(!interval.is_zero());

        // A `Mutex<bool>` is used as a flag to indicate whether the ticker was requested to stop.
        // The `Condvar` is used a notification mechanism: when the ticker is dropped, we notify
        // the thread and interrupt the ticker wait.
        #[allow(clippy::mutex_atomic)]
        let stopping = Arc::new((Mutex::new(false), Condvar::new()));
        let control = TickerControl {
            stopping: stopping.clone(),
            state: Arc::downgrade(bar_state),
        };

        let join_handle = thread::spawn(move || control.run(interval));
        Self {
            stopping,
            join_handle: Some(join_handle),
        }
    }

    pub(crate) fn stop(&self) {
        *self.stopping.0.lock().unwrap() = true;
        self.stopping.1.notify_one();
    }
}

struct TickerControl {
    stopping: Arc<(Mutex<bool>, Condvar)>,
    state: Weak<Mutex<BarState>>,
}

impl TickerControl {
    fn run(&self, interval: Duration) {
        #[cfg(test)]
        TICKER_RUNNING.store(true, Ordering::SeqCst);

        while let Some(arc) = self.state.upgrade() {
            let mut state = arc.lock().unwrap();
            if state.state.is_finished() {
                break;
            }

            state.tick(Instant::now());

            drop(state); // Don't forget to drop the lock before sleeping
            drop(arc); // Also need to drop Arc otherwise BarState won't be dropped

            // Wait for `interval` but return early if we are notified to stop
            let result = self
                .stopping
                .1
                .wait_timeout_while(self.stopping.0.lock().unwrap(), interval, |stopped| {
                    !*stopped
                })
                .unwrap();

            // If the wait didn't time out, it means we were notified to stop
            if !result.1.timed_out() {
                break;
            }
        }

        #[cfg(test)]
        TICKER_RUNNING.store(false, Ordering::SeqCst);
    }
}

// Tests using the global TICKER_RUNNING flag need to be serialized
#[cfg(test)]
pub(crate) static TICKER_TEST: Lazy<Mutex<()>> = Lazy::new(Mutex::default);

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::float_cmp)]
    #[test]
    fn test_pbar_zero() {
        let pb = ProgressBar::new(0);
        assert_eq!(pb.state().state.fraction(), 1.0);
    }

    #[allow(clippy::float_cmp)]
    #[test]
    fn test_pbar_maxu64() {
        let pb = ProgressBar::new(!0);
        assert_eq!(pb.state().state.fraction(), 0.0);
    }

    #[test]
    fn test_pbar_overflow() {
        let pb = ProgressBar::new(1);
        pb.set_draw_target(ProgressDrawTarget::hidden());
        pb.inc(2);
        pb.finish();
    }

    #[test]
    fn test_get_position() {
        let pb = ProgressBar::new(1);
        pb.set_draw_target(ProgressDrawTarget::hidden());
        pb.inc(2);
        let pos = pb.position();
        assert_eq!(pos, 2);
    }

    #[test]
    fn test_weak_pb() {
        let pb = ProgressBar::new(0);
        let weak = pb.downgrade();
        assert!(weak.upgrade().is_some());
        ::std::mem::drop(pb);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn it_can_wrap_a_reader() {
        let bytes = &b"I am an implementation of io::Read"[..];
        let pb = ProgressBar::new(bytes.len() as u64);
        let mut reader = pb.wrap_read(bytes);
        let mut writer = Vec::new();
        io::copy(&mut reader, &mut writer).unwrap();
        assert_eq!(writer, bytes);
    }

    #[test]
    fn it_can_wrap_a_writer() {
        let bytes = b"implementation of io::Read";
        let mut reader = &bytes[..];
        let pb = ProgressBar::new(bytes.len() as u64);
        let writer = Vec::new();
        let mut writer = pb.wrap_write(writer);
        io::copy(&mut reader, &mut writer).unwrap();
        assert_eq!(writer.it, bytes);
    }

    #[test]
    fn ticker_thread_terminates_on_drop() {
        let _guard = TICKER_TEST.lock().unwrap();
        assert!(!TICKER_RUNNING.load(Ordering::SeqCst));

        let pb = ProgressBar::new_spinner();
        pb.enable_steady_tick(Duration::from_millis(50));

        // Give the thread time to start up
        thread::sleep(Duration::from_millis(250));

        assert!(TICKER_RUNNING.load(Ordering::SeqCst));

        drop(pb);
        assert!(!TICKER_RUNNING.load(Ordering::SeqCst));
    }

    #[test]
    fn ticker_thread_terminates_on_drop_2() {
        let _guard = TICKER_TEST.lock().unwrap();
        assert!(!TICKER_RUNNING.load(Ordering::SeqCst));

        let pb = ProgressBar::new_spinner();
        pb.enable_steady_tick(Duration::from_millis(50));
        let pb2 = pb.clone();

        // Give the thread time to start up
        thread::sleep(Duration::from_millis(250));

        assert!(TICKER_RUNNING.load(Ordering::SeqCst));

        drop(pb);
        assert!(TICKER_RUNNING.load(Ordering::SeqCst));

        drop(pb2);
        assert!(!TICKER_RUNNING.load(Ordering::SeqCst));
    }
}
