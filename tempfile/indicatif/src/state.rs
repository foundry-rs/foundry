use std::borrow::Cow;
use std::io;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use portable_atomic::{AtomicU64, AtomicU8, Ordering};
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::draw_target::{LineType, ProgressDrawTarget};
use crate::style::ProgressStyle;

pub(crate) struct BarState {
    pub(crate) draw_target: ProgressDrawTarget,
    pub(crate) on_finish: ProgressFinish,
    pub(crate) style: ProgressStyle,
    pub(crate) state: ProgressState,
    pub(crate) tab_width: usize,
}

impl BarState {
    pub(crate) fn new(
        len: Option<u64>,
        draw_target: ProgressDrawTarget,
        pos: Arc<AtomicPosition>,
    ) -> Self {
        Self {
            draw_target,
            on_finish: ProgressFinish::default(),
            style: ProgressStyle::default_bar(),
            state: ProgressState::new(len, pos),
            tab_width: DEFAULT_TAB_WIDTH,
        }
    }

    /// Finishes the progress bar using the [`ProgressFinish`] behavior stored
    /// in the [`ProgressStyle`].
    pub(crate) fn finish_using_style(&mut self, now: Instant, finish: ProgressFinish) {
        self.state.status = Status::DoneVisible;
        match finish {
            ProgressFinish::AndLeave => {
                if let Some(len) = self.state.len {
                    self.state.pos.set(len);
                }
            }
            ProgressFinish::WithMessage(msg) => {
                if let Some(len) = self.state.len {
                    self.state.pos.set(len);
                }
                self.state.message = TabExpandedString::new(msg, self.tab_width);
            }
            ProgressFinish::AndClear => {
                if let Some(len) = self.state.len {
                    self.state.pos.set(len);
                }
                self.state.status = Status::DoneHidden;
            }
            ProgressFinish::Abandon => {}
            ProgressFinish::AbandonWithMessage(msg) => {
                self.state.message = TabExpandedString::new(msg, self.tab_width);
            }
        }

        // There's no need to update the estimate here; once the `status` is no longer
        // `InProgress`, we will use the length and elapsed time to estimate.
        let _ = self.draw(true, now);
    }

    pub(crate) fn reset(&mut self, now: Instant, mode: Reset) {
        // Always reset the estimator; this is the only reset that will occur if mode is
        // `Reset::Eta`.
        self.state.est.reset(now);

        if let Reset::Elapsed | Reset::All = mode {
            self.state.started = now;
        }

        if let Reset::All = mode {
            self.state.pos.reset(now);
            self.state.status = Status::InProgress;

            for tracker in self.style.format_map.values_mut() {
                tracker.reset(&self.state, now);
            }

            let _ = self.draw(false, now);
        }
    }

    pub(crate) fn update(&mut self, now: Instant, f: impl FnOnce(&mut ProgressState), tick: bool) {
        f(&mut self.state);
        if tick {
            self.tick(now);
        }
    }

    pub(crate) fn unset_length(&mut self, now: Instant) {
        self.state.len = None;
        self.update_estimate_and_draw(now);
    }

    pub(crate) fn set_length(&mut self, now: Instant, len: u64) {
        self.state.len = Some(len);
        self.update_estimate_and_draw(now);
    }

    pub(crate) fn inc_length(&mut self, now: Instant, delta: u64) {
        if let Some(len) = self.state.len {
            self.state.len = Some(len.saturating_add(delta));
        }
        self.update_estimate_and_draw(now);
    }

    pub(crate) fn dec_length(&mut self, now: Instant, delta: u64) {
        if let Some(len) = self.state.len {
            self.state.len = Some(len.saturating_sub(delta));
        }
        self.update_estimate_and_draw(now);
    }

    pub(crate) fn set_tab_width(&mut self, tab_width: usize) {
        self.tab_width = tab_width;
        self.state.message.set_tab_width(tab_width);
        self.state.prefix.set_tab_width(tab_width);
        self.style.set_tab_width(tab_width);
    }

    pub(crate) fn set_style(&mut self, style: ProgressStyle) {
        self.style = style;
        self.style.set_tab_width(self.tab_width);
    }

    pub(crate) fn tick(&mut self, now: Instant) {
        self.state.tick = self.state.tick.saturating_add(1);
        self.update_estimate_and_draw(now);
    }

    pub(crate) fn update_estimate_and_draw(&mut self, now: Instant) {
        let pos = self.state.pos.pos.load(Ordering::Relaxed);
        self.state.est.record(pos, now);

        for tracker in self.style.format_map.values_mut() {
            tracker.tick(&self.state, now);
        }

        let _ = self.draw(false, now);
    }

    pub(crate) fn println(&mut self, now: Instant, msg: &str) {
        let width = self.draw_target.width();
        let mut drawable = match self.draw_target.drawable(true, now) {
            Some(drawable) => drawable,
            None => return,
        };

        let mut draw_state = drawable.state();
        let lines: Vec<LineType> = msg.lines().map(|l| LineType::Text(Into::into(l))).collect();
        // Empty msg should trigger newline as we are in println
        if lines.is_empty() {
            draw_state.lines.push(LineType::Empty);
        } else {
            draw_state.lines.extend(lines);
        }

        if let Some(width) = width {
            if !matches!(self.state.status, Status::DoneHidden) {
                self.style
                    .format_state(&self.state, &mut draw_state.lines, width);
            }
        }

        drop(draw_state);
        let _ = drawable.draw();
    }

    pub(crate) fn suspend<F: FnOnce() -> R, R>(&mut self, now: Instant, f: F) -> R {
        if let Some((state, _)) = self.draw_target.remote() {
            return state.write().unwrap().suspend(f, now);
        }

        if let Some(drawable) = self.draw_target.drawable(true, now) {
            let _ = drawable.clear();
        }

        let ret = f();
        let _ = self.draw(true, Instant::now());
        ret
    }

    pub(crate) fn draw(&mut self, mut force_draw: bool, now: Instant) -> io::Result<()> {
        // `|= self.is_finished()` should not be needed here, but we used to always draw for
        // finished progress bars, so it's kept as to not cause compatibility issues in weird cases.
        force_draw |= self.state.is_finished();
        let mut drawable = match self.draw_target.drawable(force_draw, now) {
            Some(drawable) => drawable,
            None => return Ok(()),
        };

        // Getting the width can be expensive; thus this should happen after checking drawable.
        let width = drawable.width();

        let mut draw_state = drawable.state();

        if let Some(width) = width {
            if !matches!(self.state.status, Status::DoneHidden) {
                self.style
                    .format_state(&self.state, &mut draw_state.lines, width);
            }
        }

        drop(draw_state);
        drawable.draw()
    }
}

impl Drop for BarState {
    fn drop(&mut self) {
        // Progress bar is already finished.  Do not need to do anything other than notify
        // the `MultiProgress` that we're now a zombie.
        if self.state.is_finished() {
            self.draw_target.mark_zombie();
            return;
        }

        self.finish_using_style(Instant::now(), self.on_finish.clone());

        // Notify the `MultiProgress` that we're now a zombie.
        self.draw_target.mark_zombie();
    }
}

pub(crate) enum Reset {
    Eta,
    Elapsed,
    All,
}

/// The state of a progress bar at a moment in time.
#[non_exhaustive]
pub struct ProgressState {
    pos: Arc<AtomicPosition>,
    len: Option<u64>,
    pub(crate) tick: u64,
    pub(crate) started: Instant,
    status: Status,
    est: Estimator,
    pub(crate) message: TabExpandedString,
    pub(crate) prefix: TabExpandedString,
}

impl ProgressState {
    pub(crate) fn new(len: Option<u64>, pos: Arc<AtomicPosition>) -> Self {
        let now = Instant::now();
        Self {
            pos,
            len,
            tick: 0,
            status: Status::InProgress,
            started: now,
            est: Estimator::new(now),
            message: TabExpandedString::NoTabs("".into()),
            prefix: TabExpandedString::NoTabs("".into()),
        }
    }

    /// Indicates that the progress bar finished.
    pub fn is_finished(&self) -> bool {
        match self.status {
            Status::InProgress => false,
            Status::DoneVisible => true,
            Status::DoneHidden => true,
        }
    }

    /// Returns the completion as a floating-point number between 0 and 1
    pub fn fraction(&self) -> f32 {
        let pos = self.pos.pos.load(Ordering::Relaxed);
        let pct = match (pos, self.len) {
            (_, None) => 0.0,
            (_, Some(0)) => 1.0,
            (0, _) => 0.0,
            (pos, Some(len)) => pos as f32 / len as f32,
        };
        pct.clamp(0.0, 1.0)
    }

    /// The expected ETA
    pub fn eta(&self) -> Duration {
        if self.is_finished() {
            return Duration::new(0, 0);
        }

        let len = match self.len {
            Some(len) => len,
            None => return Duration::new(0, 0),
        };

        let pos = self.pos.pos.load(Ordering::Relaxed);

        let sps = self.est.steps_per_second(Instant::now());

        // Infinite duration should only ever happen at the beginning, so in this case it's okay to
        // just show an ETA of 0 until progress starts to occur.
        if sps == 0.0 {
            return Duration::new(0, 0);
        }

        secs_to_duration(len.saturating_sub(pos) as f64 / sps)
    }

    /// The expected total duration (that is, elapsed time + expected ETA)
    pub fn duration(&self) -> Duration {
        if self.len.is_none() || self.is_finished() {
            return Duration::new(0, 0);
        }
        self.started.elapsed().saturating_add(self.eta())
    }

    /// The number of steps per second
    pub fn per_sec(&self) -> f64 {
        if let Status::InProgress = self.status {
            self.est.steps_per_second(Instant::now())
        } else {
            self.pos() as f64 / self.started.elapsed().as_secs_f64()
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn pos(&self) -> u64 {
        self.pos.pos.load(Ordering::Relaxed)
    }

    pub fn set_pos(&mut self, pos: u64) {
        self.pos.set(pos);
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> Option<u64> {
        self.len
    }

    pub fn set_len(&mut self, len: u64) {
        self.len = Some(len);
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum TabExpandedString {
    NoTabs(Cow<'static, str>),
    WithTabs {
        original: Cow<'static, str>,
        expanded: OnceLock<String>,
        tab_width: usize,
    },
}

impl TabExpandedString {
    pub(crate) fn new(s: Cow<'static, str>, tab_width: usize) -> Self {
        if !s.contains('\t') {
            Self::NoTabs(s)
        } else {
            Self::WithTabs {
                original: s,
                tab_width,
                expanded: OnceLock::new(),
            }
        }
    }

    pub(crate) fn expanded(&self) -> &str {
        match &self {
            Self::NoTabs(s) => {
                debug_assert!(!s.contains('\t'));
                s
            }
            Self::WithTabs {
                original,
                tab_width,
                expanded,
            } => expanded.get_or_init(|| original.replace('\t', &" ".repeat(*tab_width))),
        }
    }

    pub(crate) fn set_tab_width(&mut self, new_tab_width: usize) {
        if let Self::WithTabs {
            expanded,
            tab_width,
            ..
        } = self
        {
            if *tab_width != new_tab_width {
                *tab_width = new_tab_width;
                expanded.take();
            }
        }
    }
}

/// Double-smoothed exponentially weighted estimator
///
/// This uses an exponentially weighted *time-based* estimator, meaning that it exponentially
/// downweights old data based on its age. The rate at which this occurs is currently a constant
/// value of 15 seconds for 90% weighting. This means that all data older than 15 seconds has a
/// collective weight of 0.1 in the estimate, and all data older than 30 seconds has a collective
/// weight of 0.01, and so on.
///
/// The primary value exposed by `Estimator` is `steps_per_second`. This value is doubly-smoothed,
/// meaning that is the result of using an exponentially weighted estimator (as described above) to
/// estimate the value of another exponentially weighted estimator, which estimates the value of
/// the raw data.
///
/// The purpose of this extra smoothing step is to reduce instantaneous fluctations in the estimate
/// when large updates are received. Without this, estimates might have a large spike followed by a
/// slow asymptotic approach to zero (until the next spike).
#[derive(Debug)]
pub(crate) struct Estimator {
    smoothed_steps_per_sec: f64,
    double_smoothed_steps_per_sec: f64,
    prev_steps: u64,
    prev_time: Instant,
    start_time: Instant,
}

impl Estimator {
    fn new(now: Instant) -> Self {
        Self {
            smoothed_steps_per_sec: 0.0,
            double_smoothed_steps_per_sec: 0.0,
            prev_steps: 0,
            prev_time: now,
            start_time: now,
        }
    }

    fn record(&mut self, new_steps: u64, now: Instant) {
        // sanity check: don't record data if time or steps have not advanced
        if new_steps <= self.prev_steps || now <= self.prev_time {
            // Reset on backwards seek to prevent breakage from seeking to the end for length determination
            // See https://github.com/console-rs/indicatif/issues/480
            if new_steps < self.prev_steps {
                self.prev_steps = new_steps;
                self.reset(now);
            }
            return;
        }

        let delta_steps = new_steps - self.prev_steps;
        let delta_t = duration_to_secs(now - self.prev_time);

        // the rate of steps we saw in this update
        let new_steps_per_second = delta_steps as f64 / delta_t;

        // update the estimate: a weighted average of the old estimate and new data
        let weight = estimator_weight(delta_t);
        self.smoothed_steps_per_sec =
            self.smoothed_steps_per_sec * weight + new_steps_per_second * (1.0 - weight);

        // An iterative estimate like `smoothed_steps_per_sec` is supposed to be an exponentially
        // weighted average from t=0 back to t=-inf; Since we initialize it to 0, we neglect the
        // (non-existent) samples in the weighted average prior to the first one, so the resulting
        // average must be normalized. We normalize the single estimate here in order to use it as
        // a source for the double smoothed estimate. See comment on normalization in
        // `steps_per_second` for details.
        let delta_t_start = duration_to_secs(now - self.start_time);
        let total_weight = 1.0 - estimator_weight(delta_t_start);
        let normalized_smoothed_steps_per_sec = self.smoothed_steps_per_sec / total_weight;

        // determine the double smoothed value (EWA smoothing of the single EWA)
        self.double_smoothed_steps_per_sec = self.double_smoothed_steps_per_sec * weight
            + normalized_smoothed_steps_per_sec * (1.0 - weight);

        self.prev_steps = new_steps;
        self.prev_time = now;
    }

    /// Reset the state of the estimator. Once reset, estimates will not depend on any data prior
    /// to `now`. This does not reset the stored position of the progress bar.
    pub(crate) fn reset(&mut self, now: Instant) {
        self.smoothed_steps_per_sec = 0.0;
        self.double_smoothed_steps_per_sec = 0.0;

        // only reset prev_time, not prev_steps
        self.prev_time = now;
        self.start_time = now;
    }

    /// Average time per step in seconds, using double exponential smoothing
    fn steps_per_second(&self, now: Instant) -> f64 {
        // Because the value stored in the Estimator is only updated when the Estimator receives an
        // update, this value will become stuck if progress stalls. To return an accurate estimate,
        // we determine how much time has passed since the last update, and treat this as a
        // pseudo-update with 0 steps.
        let delta_t = duration_to_secs(now - self.prev_time);
        let reweight = estimator_weight(delta_t);

        // Normalization of estimates:
        //
        // The raw estimate is a single value (smoothed_steps_per_second) that is iteratively
        // updated. At each update, the previous value of the estimate is downweighted according to
        // its age, receiving the iterative weight W(t) = 0.1 ^ (t/15).
        //
        // Since W(Sum(t_n)) = Prod(W(t_n)), the total weight of a sample after a series of
        // iterative steps is simply W(t_e) - W(t_b), where t_e is the time since the end of the
        // sample, and t_b is the time since the beginning. The resulting estimate is therefore a
        // weighted average with sample weights W(t_e) - W(t_b).
        //
        // Notice that the weighting function generates sample weights that sum to 1 only when the
        // sample times span from t=0 to t=inf; but this is not the case. We have a first sample
        // with finite, positive t_b = t_f. In the raw estimate, we handle times prior to t_f by
        // setting an initial value of 0, meaning that these (non-existent) samples have no weight.
        //
        // Therefore, the raw estimate must be normalized by dividing it by the sum of the weights
        // in the weighted average. This sum is just W(0) - W(t_f), where t_f is the time since the
        // first sample, and W(0) = 1.
        let delta_t_start = duration_to_secs(now - self.start_time);
        let total_weight = 1.0 - estimator_weight(delta_t_start);

        // Generate updated values for `smoothed_steps_per_sec` and `double_smoothed_steps_per_sec`
        // (sps and dsps) without storing them. Note that we normalize sps when using it as a
        // source to update dsps, and then normalize dsps itself before returning it.
        let sps = self.smoothed_steps_per_sec * reweight / total_weight;
        let dsps = self.double_smoothed_steps_per_sec * reweight + sps * (1.0 - reweight);
        dsps / total_weight
    }
}

pub(crate) struct AtomicPosition {
    pub(crate) pos: AtomicU64,
    capacity: AtomicU8,
    prev: AtomicU64,
    start: Instant,
}

impl AtomicPosition {
    pub(crate) fn new() -> Self {
        Self {
            pos: AtomicU64::new(0),
            capacity: AtomicU8::new(MAX_BURST),
            prev: AtomicU64::new(0),
            start: Instant::now(),
        }
    }

    pub(crate) fn allow(&self, now: Instant) -> bool {
        if now < self.start {
            return false;
        }

        let mut capacity = self.capacity.load(Ordering::Acquire);
        // `prev` is the number of ns after `self.started` we last returned `true`
        let prev = self.prev.load(Ordering::Acquire);
        // `elapsed` is the number of ns since `self.started`
        let elapsed = (now - self.start).as_nanos() as u64;
        // `diff` is the number of ns since we last returned `true`
        let diff = elapsed.saturating_sub(prev);

        // If `capacity` is 0 and not enough time (1ms) has passed since `prev`
        // to add new capacity, return `false`. The goal of this method is to
        // make this decision as efficient as possible.
        if capacity == 0 && diff < INTERVAL {
            return false;
        }

        // We now calculate `new`, the number of INTERVALs since we last returned `true`,
        // and `remainder`, which represents a number of ns less than INTERVAL which we cannot
        // convert into capacity now, so we're saving it for later. We do this by
        // subtracting this from `elapsed` before storing it into `self.prev`.
        let (new, remainder) = ((diff / INTERVAL), (diff % INTERVAL));
        // We add `new` to `capacity`, subtract one for returning `true` from here,
        // then make sure it does not exceed a maximum of `MAX_BURST`.
        capacity = Ord::min(MAX_BURST as u128, (capacity as u128) + (new as u128) - 1) as u8;

        // Then, we just store `capacity` and `prev` atomically for the next iteration
        self.capacity.store(capacity, Ordering::Release);
        self.prev.store(elapsed - remainder, Ordering::Release);
        true
    }

    fn reset(&self, now: Instant) {
        self.set(0);
        let elapsed = (now.saturating_duration_since(self.start)).as_nanos() as u64;
        self.prev.store(elapsed, Ordering::Release);
    }

    pub(crate) fn inc(&self, delta: u64) {
        self.pos.fetch_add(delta, Ordering::SeqCst);
    }

    pub(crate) fn dec(&self, delta: u64) {
        self.pos.fetch_sub(delta, Ordering::SeqCst);
    }

    pub(crate) fn set(&self, pos: u64) {
        self.pos.store(pos, Ordering::Release);
    }
}

const INTERVAL: u64 = 1_000_000;
const MAX_BURST: u8 = 10;

/// Behavior of a progress bar when it is finished
///
/// This is invoked when a [`ProgressBar`] or [`ProgressBarIter`] completes and
/// [`ProgressBar::is_finished`] is false.
///
/// [`ProgressBar`]: crate::ProgressBar
/// [`ProgressBarIter`]: crate::ProgressBarIter
/// [`ProgressBar::is_finished`]: crate::ProgressBar::is_finished
#[derive(Clone, Debug)]
pub enum ProgressFinish {
    /// Finishes the progress bar and leaves the current message
    ///
    /// Same behavior as calling [`ProgressBar::finish()`](crate::ProgressBar::finish).
    AndLeave,
    /// Finishes the progress bar and sets a message
    ///
    /// Same behavior as calling [`ProgressBar::finish_with_message()`](crate::ProgressBar::finish_with_message).
    WithMessage(Cow<'static, str>),
    /// Finishes the progress bar and completely clears it (this is the default)
    ///
    /// Same behavior as calling [`ProgressBar::finish_and_clear()`](crate::ProgressBar::finish_and_clear).
    AndClear,
    /// Finishes the progress bar and leaves the current message and progress
    ///
    /// Same behavior as calling [`ProgressBar::abandon()`](crate::ProgressBar::abandon).
    Abandon,
    /// Finishes the progress bar and sets a message, and leaves the current progress
    ///
    /// Same behavior as calling [`ProgressBar::abandon_with_message()`](crate::ProgressBar::abandon_with_message).
    AbandonWithMessage(Cow<'static, str>),
}

impl Default for ProgressFinish {
    fn default() -> Self {
        Self::AndClear
    }
}

/// Get the appropriate dilution weight for Estimator data given the data's age (in seconds)
///
/// Whenever an update occurs, we will create a new estimate using a weight `w_i` like so:
///
/// ```math
/// <new estimate> = <previous estimate> * w_i + <new data> * (1 - w_i)
/// ```
///
/// In other words, the new estimate is a weighted average of the previous estimate and the new
/// data. We want to choose weights such that for any set of samples where `t_0, t_1, ...` are
/// the durations of the samples:
///
/// ```math
/// Sum(t_i) = ews ==> Prod(w_i) = 0.1
/// ```
///
/// With this constraint it is easy to show that
///
/// ```math
/// w_i = 0.1 ^ (t_i / ews)
/// ```
///
/// Notice that the constraint implies that estimates are independent of the durations of the
/// samples, a very useful feature.
fn estimator_weight(age: f64) -> f64 {
    const EXPONENTIAL_WEIGHTING_SECONDS: f64 = 15.0;
    0.1_f64.powf(age / EXPONENTIAL_WEIGHTING_SECONDS)
}

fn duration_to_secs(d: Duration) -> f64 {
    d.as_secs() as f64 + f64::from(d.subsec_nanos()) / 1_000_000_000f64
}

fn secs_to_duration(s: f64) -> Duration {
    let secs = s.trunc() as u64;
    let nanos = (s.fract() * 1_000_000_000f64) as u32;
    Duration::new(secs, nanos)
}

#[derive(Debug)]
pub(crate) enum Status {
    InProgress,
    DoneVisible,
    DoneHidden,
}

pub(crate) const DEFAULT_TAB_WIDTH: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProgressBar;

    // https://github.com/rust-lang/rust-clippy/issues/10281
    #[allow(clippy::uninlined_format_args)]
    #[test]
    fn test_steps_per_second() {
        let test_rate = |items_per_second| {
            let mut now = Instant::now();
            let mut est = Estimator::new(now);
            let mut pos = 0;

            for _ in 0..20 {
                pos += items_per_second;
                now += Duration::from_secs(1);
                est.record(pos, now);
            }
            let avg_steps_per_second = est.steps_per_second(now);

            assert!(avg_steps_per_second > 0.0);
            assert!(avg_steps_per_second.is_finite());

            let absolute_error = (avg_steps_per_second - items_per_second as f64).abs();
            let relative_error = absolute_error / items_per_second as f64;
            assert!(
                relative_error < 1.0 / 1e9,
                "Expected rate: {}, actual: {}, relative error: {}",
                items_per_second,
                avg_steps_per_second,
                relative_error
            );
        };

        test_rate(1);
        test_rate(1_000);
        test_rate(1_000_000);
        test_rate(1_000_000_000);
        test_rate(1_000_000_001);
        test_rate(100_000_000_000);
        test_rate(1_000_000_000_000);
        test_rate(100_000_000_000_000);
        test_rate(1_000_000_000_000_000);
    }

    #[test]
    fn test_double_exponential_ave() {
        let mut now = Instant::now();
        let mut est = Estimator::new(now);
        let mut pos = 0;

        // note: this is the default weight set in the Estimator
        let weight = 15;

        for _ in 0..weight {
            pos += 1;
            now += Duration::from_secs(1);
            est.record(pos, now);
        }
        now += Duration::from_secs(weight);

        // The first level EWA:
        //   -> 90% weight @ 0 eps, 9% weight @ 1 eps, 1% weight @ 0 eps
        //   -> then normalized by deweighting the 1% weight (before -30 seconds)
        let single_target = 0.09 / 0.99;

        // The second level EWA:
        //   -> same logic as above, but using the first level EWA as the source
        let double_target = (0.9 * single_target + 0.09) / 0.99;
        assert_eq!(est.steps_per_second(now), double_target);
    }

    #[test]
    fn test_estimator_rewind_position() {
        let mut now = Instant::now();
        let mut est = Estimator::new(now);

        now += Duration::from_secs(1);
        est.record(1, now);

        // should not panic
        now += Duration::from_secs(1);
        est.record(0, now);

        // check that reset occurred (estimator at 1 event per sec)
        now += Duration::from_secs(1);
        est.record(1, now);
        assert_eq!(est.steps_per_second(now), 1.0);

        // check that progress bar handles manual seeking
        let pb = ProgressBar::hidden();
        pb.set_length(10);
        pb.set_position(1);
        pb.tick();
        // Should not panic.
        pb.set_position(0);
    }

    #[test]
    fn test_reset_eta() {
        let mut now = Instant::now();
        let mut est = Estimator::new(now);

        // two per second, then reset
        now += Duration::from_secs(1);
        est.record(2, now);
        est.reset(now);

        // now one per second, and verify
        now += Duration::from_secs(1);
        est.record(3, now);
        assert_eq!(est.steps_per_second(now), 1.0);
    }

    #[test]
    fn test_duration_stuff() {
        let duration = Duration::new(42, 100_000_000);
        let secs = duration_to_secs(duration);
        assert_eq!(secs_to_duration(secs), duration);
    }

    #[test]
    fn test_atomic_position_large_time_difference() {
        let atomic_position = AtomicPosition::new();
        let later = atomic_position.start + Duration::from_nanos(INTERVAL * u64::from(u8::MAX));
        // Should not panic.
        atomic_position.allow(later);
    }

    #[test]
    fn test_atomic_position_reset() {
        const ELAPSE_TIME: Duration = Duration::from_millis(20);
        let mut pos = AtomicPosition::new();
        pos.reset(pos.start + ELAPSE_TIME);

        // prev should be exactly ELAPSE_TIME after reset
        assert_eq!(*pos.pos.get_mut(), 0);
        assert_eq!(*pos.prev.get_mut(), ELAPSE_TIME.as_nanos() as u64);
    }
}
