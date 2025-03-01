//! Cross-platform terminal screen clearing.
//!
//! This library provides a set of ways to clear a screen, plus a “best effort” convenience function
//! to do the right thing most of the time.
//!
//! Unlike many cross-platform libraries, this one exposes every available choice all the time, and
//! only the convenience function varies based on compilation target or environmental factors.
//!
//! 90% of the time, you’ll want to use the convenience short-hand:
//!
//! ```no_run
//! clearscreen::clear().expect("failed to clear screen");
//! ```
//!
//! For anything else, refer to the [`ClearScreen`] enum.
//!
//! If you are supporting Windows in any capacity, the [`is_windows_10()`] documentation is
//! **required reading**.

#![doc(html_favicon_url = "https://watchexec.github.io/logo:clearscreen.svg")]
#![doc(html_logo_url = "https://watchexec.github.io/logo:clearscreen.svg")]
#![warn(missing_docs)]

use std::{
	borrow::Cow,
	env,
	io::{self, Write},
	process::{Command, ExitStatus},
};

use terminfo::{
	capability::{self, Expansion},
	expand::{Context, Parameter},
	Capability, Database, Value,
};
use thiserror::Error;
use which::which;

/// Ways to clear the screen.
///
/// There isn’t a single way to clear the (terminal/console) screen. Not only are there several
/// techniques to achieve the outcome, there are differences in the way terminal emulators intepret
/// some of these techniques, as well as platform particularities.
///
/// In addition, there are other conditions a screen can be in that might be beneficial to reset,
/// such as when a TUI application crashes and leaves the terminal in a less than useful state.
///
/// Finally, a terminal may have scrollback, and this can be kept as-is or cleared as well.
///
/// Your application may need one particular clearing method, or it might offer several options to
/// the user, such as “hard” and “soft” clearing. This library makes no assumption and no judgement
/// on what is considered hard, soft, or something else: that is your responsibility to determine in
/// your context.
///
/// For most cases, you should use [`ClearScreen::default()`] to select the most appropriate method.
///
/// In any event, once a way is selected, call [`clear()`][ClearScreen::clear()] to apply it.
///
/// # Example
///
/// ```no_run
/// # use clearscreen::ClearScreen;
/// ClearScreen::default().clear().expect("failed to clear the screen");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClearScreen {
	/// Does both [`TerminfoScreen`][ClearScreen::TerminfoScreen] and
	/// [`TerminfoScrollback`][ClearScreen::TerminfoScrollback], in this order, but skips the
	/// scrollback reset if the capability isn’t available.
	///
	/// This is essentially what the [`clear`] command on unix does.
	/// [`clear`]: https://invisible-island.net/ncurses/man/clear.1.html
	Terminfo,

	/// Looks up the `clear` capability in the terminfo (from the TERM env var), and applies it.
	///
	/// A non-hashed terminfo database is required (this is a [terminfo crate] limitation), such as
	/// the one provided with ncurses.
	///
	/// [terminfo crate]: https://lib.rs/crates/terminfo
	TerminfoScreen,

	/// Looks up the `E3` (Erase Scrollback) capability in the terminfo (from the TERM env var), and applies it.
	///
	/// The same terminfo limitation applies as for [`TerminfoScreen`][ClearScreen::TerminfoScreen].
	TerminfoScrollback,

	/// Performs a terminfo-driven terminal reset sequence.
	///
	/// This prints whichever are available of the **rs1**, **rs2**, **rs3**, and **rf** sequences.
	/// If none of these are available, it prints whichever are available of the **is1**, **is2**,
	/// **is3**, and **if** sequences. If none are available, an error is returned.
	///
	/// This generally issues at least an `ESC c` sequence, which resets all terminal state to
	/// default values, and then may issue more sequences to reset other things or enforce a
	/// particular kind of state. See [`XtermReset`][ClearScreen::XtermReset] for a description of
	/// what XTerm does, as an example.
	///
	/// Note that this is _not_ analogous to what `tput reset` does: to emulate that, issuing first
	/// one of VtCooked/VtWellDone/WindowsCooked followed by this variant will come close.
	///
	/// The same terminfo limitation applies as for [`TerminfoScreen`][ClearScreen::TerminfoScreen].
	TerminfoReset,

	/// Prints clear screen and scrollback sequence as if TERM=xterm.
	///
	/// This does not look up the correct sequence in the terminfo database, but rather prints:
	///
	/// - `CSI H` (Cursor Position 0,0), which sets the cursor position to 0,0.
	/// - `CSI 2J` (Erase Screen), which erases the whole screen.
	/// - `CSI 3J` (Erase Scrollback), which erases the scrollback (xterm extension).
	XtermClear,

	/// Prints the terminal reset sequence as if TERM=xterm.
	///
	/// This does not look up the correct sequence in the terminfo database, but rather prints:
	///
	/// - `ESC c` (Reset to Initial State), which nominally resets all terminal state to initial
	///   values, but see the documentation for [`VtRis`][ClearScreen::VtRis].
	/// - `CSI !p` (Soft Terminal Reset), which nominally does the same thing as RIS, but without
	///   disconnecting the terminal data lines… which matters when you’re living in 1970.
	/// - `CSI ?3l` (Reset to 80 Columns), which resets the terminal width to 80 columns, or more
	///   accurately, resets the option that selects 132 column mode, to its default value of no.
	///   I don’t know, man.
	/// - `CSI ?4l` (Reset to Jump Scrolling), which sets the scrolling mode to jump. This is naught
	///   to do with what we think of as “scrolling,” but rather it’s about the speed at which the
	///   terminal will add lines to the screen. Jump mode means “give it to me as fast as it comes”
	///   and Smooth mode means to do some buffering and output lines “at a moderate, smooth rate.”
	/// - `CSI 4l` (Reset to Replace Mode), which sets the cursor writing mode to Replace, i.e.
	///   overwriting characters at cursor position, instead of Insert, which pushes characters
	///   under the cursor to the right.
	/// - `ESC >` (Set Key Pad to Normal), which sets the keyboard’s numeric keypad to send “what’s
	///   printed on the keys” i.e. numbers and the arithmetic symbols.
	/// - `CSI ?69l` (Reset Left and Right Margins to the page), which sets the horizontal margins
	///   to coincide with the page’s margins: nowadays, no margins.
	XtermReset,

	/// Calls the command `tput clear`.
	///
	/// That command most likely does what [`Terminfo`][ClearScreen::Terminfo] does internally, but
	/// may work better in some cases, such as when the terminfo database on the system is hashed or
	/// in a non-standard location that the terminfo crate does not find.
	///
	/// However, it relies on the `tput` command being available, and on being able to run commands.
	TputClear,

	/// Calls the command `tput reset`.
	///
	/// See the documentation above on [`TputClear`][ClearScreen::TputClear] for more details, save
	/// that the equivalent is [`TerminfoReset`][ClearScreen::TerminfoReset].
	TputReset,

	/// Calls the command `cls`.
	///
	/// This is the Windows command to clear the screen. It has the same caveats as
	/// [`TputClear`][ClearScreen::TputClear] does, but its internal mechanism is not known. Prefer
	/// [`WindowsClear`][ClearScreen::WindowsClear] instead to avoid relying on an external command.
	///
	/// This will always attempt to run the command, regardless of compile target, which may have
	/// unintended effects if the `cls` executable does something different on the platform.
	Cls,

	/// Sets the Windows Console to support VT escapes.
	///
	/// This sets the `ENABLE_VIRTUAL_TERMINAL_PROCESSING` bit in the console mode, which enables
	/// support for the terminal escape sequences every other terminal uses. This is supported since
	/// Windows 10, from the Threshold 2 Update in November 2015.
	///
	/// Does nothing on non-Windows targets.
	WindowsVt,

	/// Sets the Windows Console to support VT escapes and prints the clear sequence.
	///
	/// This runs [`WindowsVt`][ClearScreen::WindowsVt] and [`XtermClear`][ClearScreen::XtermClear],
	/// in this order. This is described here:
	/// https://docs.microsoft.com/en-us/windows/console/clearing-the-screen#example-1 as the
	/// recommended clearing method for all new development, although we also reset the cursor
	/// position.
	///
	/// While `WindowsVt` will do nothing on non-Windows targets, `XtermClear` will still run.
	WindowsVtClear,

	/// Uses Windows Console function to scroll the screen buffer and fill it with white space.
	///
	/// - Scrolls up one screenful
	/// - Fills the buffer with whitespace and attributes set to default.
	/// - Flushes the input buffer
	/// - Sets the cursor position to 0,0
	///
	/// This is described here: https://docs.microsoft.com/en-us/windows/console/clearing-the-screen#example-2
	/// as the equivalent to CMD.EXE's `cls` command.
	///
	/// Does nothing on non-Windows targets.
	#[cfg(feature = "windows-console")]
	WindowsConsoleClear,

	/// Uses Windows Console function to blank the screen state.
	///
	/// - Fills the screen buffer with ` ` (space) characters
	/// - Resets cell attributes over the entire buffer
	/// - Flushes the input buffer
	/// - Sets the cursor position to 0,0
	///
	/// This is described here: https://docs.microsoft.com/en-us/windows/console/clearing-the-screen#example-3
	///
	/// Does nothing on non-Windows targets.
	#[cfg(feature = "windows-console")]
	WindowsConsoleBlank,

	/// Uses Windows Console function to disable raw mode.
	///
	/// Does nothing on non-Windows targets.
	WindowsCooked,

	/// Prints the RIS VT100 escape code: Reset to Initial State.
	///
	/// This is the `ESC c` or `1b 63` escape, which by spec is defined to reset the terminal state
	/// to all initial values, which may be a range of things, for example as described in the VT510
	/// manual: https://vt100.net/docs/vt510-rm/RIS
	///
	/// However, the exact behaviour is highly dependent on the terminal emulator, and some modern
	/// terminal emulators do not always clear scrollback, for example Tmux and GNOME VTE.
	VtRis,

	/// Prints the CSI sequence to leave the Alternate Screen mode.
	///
	/// If the screen is in alternate screen mode, like how vim or a pager or another such rich TUI
	/// application would do, this sequence will clear the alternate screen buffer, then revert the
	/// terminal to normal mode, and restore the position of the cursor to what it was before
	/// Alternate Screen mode was entered, assuming the proper sequence was used.
	///
	/// It will not clear the normal mode buffer.
	///
	/// This is useful when recovering from a TUI application which crashed without resetting state.
	VtLeaveAlt,

	/// Sets the terminal to cooked mode.
	///
	/// This attempts to switch the terminal to “cooked” mode, which can be thought of as the
	/// opposite of “raw” mode, where the terminal does not respond to line discipline (which makes
	/// carriage return, line feed, and general typing display out to screen, and translates Ctrl-C
	/// to sending the SIGINT signal, etc) but instead passes all input to the controlling program
	/// and only displays what it outputs explicitly.
	///
	/// There’s also an intermediate “cbreak” or “rare” mode which behaves like “cooked” but sends
	/// each character one at a time immediately rather buffering and sending lines.
	///
	/// TUI applications such as editors and pagers often set raw mode to gain precise control of
	/// the terminal state. If such a program crashes, it may not reset the terminal mode back to
	/// the mode it found it in, which can leave the terminal behaving oddly or rendering it
	/// completely unusable.
	///
	/// In truth, these terminal modes are a set of configuration bits that are given to the
	/// `termios(3)` libc API, and control a variety of terminal modes. “Cooked” mode sets:
	///
	/// - Input BRKINT set: on BREAK, flush i/o queues and send a SIGINT to any running process.
	/// - Input ICRNL set: translate Carriage Returns to New Lines on input.
	/// - Input IGNPAR set: ignore framing and parity errors.
	/// - Input ISTRIP set: strip off eigth bit.
	/// - Input IXON set: enable XON/XOFF flow control on output.
	/// - Output OPOST set: enable output processing.
	/// - Local ICANON set: enable canonical mode (see below).
	/// - Local ISIG set: when Ctrl-C, Ctrl-Q, etc are received, send the appropriate signal.
	///
	/// Canonical mode is really the core of “cooked” mode and enables:
	///
	/// - line buffering, so input is only sent to the underlying program when a line delimiter
	///   character is entered (usually a newline);
	/// - line editing, so ERASE (backspace) and KILL (remove entire line) control characters edit
	///   the line before it is sent to the program;
	/// - a maximum line length of 4096 characters (bytes).
	///
	/// When canonical mode is unset (when the bit is cleared), all input processing is disabled.
	///
	/// Due to how the underlying [`tcsetattr`] function is defined in POSIX, this may complete
	/// without error if _any part_ of the configuration is applied, not just when all of it is set.
	///
	/// Note that you generally want [`VtWellDone`][ClearScreen::VtWellDone] instead.
	///
	/// Does nothing on non-Unix targets.
	///
	/// [`tcsetattr`]: https://pubs.opengroup.org/onlinepubs/9699919799/functions/tcsetattr.html
	VtCooked,

	/// Sets the terminal to “well done” mode.
	///
	/// This is similar to [`VtCooked`][ClearScreen::VtCooked], but with a different, broader, mode
	/// configuration which approximates a terminal’s initial state, such as is expected by a shell,
	/// and clears many bits that should probably never be set (like the translation/mapping modes).
	///
	/// “Well done” mode is an invention of this library, inspired by several other sources such as
	/// Golang’s goterm, the termios(3) and tput(1) manual pages, but not identical to any.
	///
	/// Notably most implementations read the terminal configuration bits and only modify that set,
	/// whereas this library authoritatively writes the entire configuration from scratch.
	///
	/// It is a strict superset of [`VtCooked`][ClearScreen::VtCooked].
	///
	/// - Input BRKINT set: on BREAK, flush i/o queues and send a SIGINT to any running process.
	/// - Input ICRNL set: translate Carriage Return to New Line on input.
	/// - Input IUTF8 set: input is UTF-8 (Linux only, since 2.6.4).
	/// - Input IGNPAR set: ignore framing and parity errors.
	/// - Input IMAXBEL set: ring terminal bell when input queue is full (not implemented in Linux).
	/// - Input ISTRIP set: strip off eigth bit.
	/// - Input IXON set: enable XON/XOFF flow control on output.
	/// - Output ONLCR set: do not translate Carriage Return to CR NL.
	/// - Output OPOST set: enable output processing.
	/// - Control CREAD set: enable receiver.
	/// - Local ICANON set: enable canonical mode (see [`VtCooked`][ClearScreen::VtCooked]).
	/// - Local ISIG set: when Ctrl-C, Ctrl-Q, etc are received, send the appropriate signal.
	///
	/// Does nothing on non-Unix targets.
	VtWellDone,
}

impl Default for ClearScreen {
	/// Detects the environment and makes its best guess as how to clear the screen.
	///
	/// This function’s behaviour (but not its type signature) may change without notice, as better
	/// techniques appear. However, it will always strive to provide the best method. It will also
	/// never have side-effects, and finding any such behaviour should be reported as a bug.
	///
	/// If you wish to make your own, the [`is_microsoft_terminal()`] and [`is_windows_10()`]
	/// functions may be useful.
	///
	/// The [`ClearScreen`] variant selected is always in the “clear” behaviour side of things. If
	/// you wish to only clear the screen and not the scrollback, or to perform a terminal reset, or
	/// apply the other available clearing strategies, you’ll need to select what’s best yourself.
	///
	/// See the [TERMINALS.md file in the repo][TERMINALS.md] for research on many terminals as well
	/// as the current result of this function for each terminal.
	///
	/// [TERMINALS.md]: https://github.com/watchexec/clearscreen/blob/main/TERMINALS.md
	fn default() -> Self {
		use env::var;
		use std::ffi::OsStr;

		fn varfull(key: impl AsRef<OsStr>) -> bool {
			var(key).map_or(false, |s| !s.is_empty())
		}

		let term = var("TERM").ok();
		let term = term.as_ref();

		if cfg!(windows) {
			return if is_microsoft_terminal() {
				Self::XtermClear
			} else if is_windows_10() {
				Self::WindowsVtClear
			} else if term.is_some() && varfull("TERMINFO") {
				Self::Terminfo
			} else if term.is_some() && which("tput").is_ok() {
				Self::TputClear
			} else {
				Self::Cls
			};
		}

		if let Some(term) = term {
			// These VTE-based terminals support CSI 3J but their own terminfos don’t have E3
			if (term.starts_with("gnome")
				&& varfull("GNOME_TERMINAL_SCREEN")
				&& varfull("GNOME_TERMINAL_SERVICE"))
				|| term == "xfce"
				|| term.contains("termite")
			{
				return Self::XtermClear;
			}

			// - SyncTERM does support the XtermClear sequence but does not clear the scrollback,
			// and does not have a terminfo, so VtRis is the only option.
			// - rxvt, when using its own terminfos, erases the screen instead of clearing and
			// doesn’t clear scrollback. It supports and behave properly for the entire XtermClear
			// sequence, but it also does the right thing with VtRis, and that seems more reliable.
			// - Other variants of (u)rxvt do the same.
			// - Kitty does as rxvt does here.
			// - Tess does support the XtermClear sequence but has a weird scrollbar behaviour,
			// which does not happen with VtRis.
			// - Zutty does not support E3, and erases the buffer on clear like rxvt, but does work
			// properly with VtRis.
			// - Same behaviour with the multiplexer Zellij.
			if term == "syncterm"
				|| term.contains("rxvt")
				|| term.contains("kitty")
				|| var("CHROME_DESKTOP").map_or(false, |cd| cd == "tess.desktop")
				|| varfull("ZUTTY_VERSION")
				|| varfull("ZELLIJ")
			{
				return Self::VtRis;
			}

			// - screen supports CSI 3J only within the XtermClear sequence, without E3 capability.
			// - Konsole handles CSI 3J correctly only within the XtermClear sequence.
			// - Wezterm handles CSI 3J correctly only within the XtermClear sequence.
			// - assume tmux TERMs are only used within tmux, and avoid the requirement for a functioning terminfo then
			if term.starts_with("screen")
				|| term.starts_with("konsole")
				|| term == "wezterm"
				|| term.starts_with("tmux")
			{
				return Self::XtermClear;
			}

			// Default xterm* terminfo on macOS does not include E3, but many terminals support it.
			if cfg!(target_os = "macos")
				&& term.starts_with("xterm")
				&& Database::from_env()
					.map(|info| info.get::<ResetScrollback>().is_none())
					.unwrap_or(true)
			{
				return Self::XtermClear;
			}

			if !term.is_empty() && Database::from_env().is_ok() {
				return Self::Terminfo;
			}
		}

		Self::XtermClear
	}
}

const ESC: &[u8] = b"\x1b";
const CSI: &[u8] = b"\x1b[";
const RIS: &[u8] = b"c";

impl ClearScreen {
	/// Performs the clearing action, printing to stdout.
	pub fn clear(self) -> Result<(), Error> {
		let mut stdout = io::stdout();
		self.clear_to(&mut stdout)
	}

	/// Performs the clearing action, printing to a given writer.
	///
	/// This allows to capture any escape sequences that might be printed, for example, but note
	/// that it will not prevent actions taken via system APIs, such as the Windows, VtCooked, and
	/// VtWellDone variants do.
	///
	/// For normal use, prefer [`clear()`].
	pub fn clear_to(self, mut w: &mut impl Write) -> Result<(), Error> {
		match self {
			Self::Terminfo => {
				let info = Database::from_env()?;
				let mut ctx = Context::default();

				if let Some(seq) = info.get::<capability::ClearScreen>() {
					seq.expand().with(&mut ctx).to(&mut w)?;
					w.flush()?;
				} else {
					return Err(Error::TerminfoCap("clear"));
				}

				if let Some(seq) = info.get::<ResetScrollback>() {
					seq.expand().with(&mut ctx).to(&mut w)?;
					w.flush()?;
				}
			}
			Self::TerminfoScreen => {
				let info = Database::from_env()?;
				if let Some(seq) = info.get::<capability::ClearScreen>() {
					seq.expand().to(&mut w)?;
					w.flush()?;
				} else {
					return Err(Error::TerminfoCap("clear"));
				}
			}
			Self::TerminfoScrollback => {
				let info = Database::from_env()?;
				if let Some(seq) = info.get::<ResetScrollback>() {
					seq.expand().to(&mut w)?;
					w.flush()?;
				} else {
					return Err(Error::TerminfoCap("E3"));
				}
			}
			Self::TerminfoReset => {
				let info = Database::from_env()?;
				let mut ctx = Context::default();
				let mut reset = false;

				if let Some(seq) = info.get::<capability::Reset1String>() {
					reset = true;
					seq.expand().with(&mut ctx).to(&mut w)?;
				}
				if let Some(seq) = info.get::<capability::Reset2String>() {
					reset = true;
					seq.expand().with(&mut ctx).to(&mut w)?;
				}
				if let Some(seq) = info.get::<capability::Reset3String>() {
					reset = true;
					seq.expand().with(&mut ctx).to(&mut w)?;
				}
				if let Some(seq) = info.get::<capability::ResetFile>() {
					reset = true;
					seq.expand().with(&mut ctx).to(&mut w)?;
				}

				w.flush()?;

				if reset {
					return Ok(());
				}

				if let Some(seq) = info.get::<capability::Init1String>() {
					reset = true;
					seq.expand().with(&mut ctx).to(&mut w)?;
				}
				if let Some(seq) = info.get::<capability::Init2String>() {
					reset = true;
					seq.expand().with(&mut ctx).to(&mut w)?;
				}
				if let Some(seq) = info.get::<capability::Init3String>() {
					reset = true;
					seq.expand().with(&mut ctx).to(&mut w)?;
				}
				if let Some(seq) = info.get::<capability::InitFile>() {
					reset = true;
					seq.expand().with(&mut ctx).to(&mut w)?;
				}

				w.flush()?;

				if !reset {
					return Err(Error::TerminfoCap("reset"));
				}
			}
			Self::XtermClear => {
				const CURSOR_HOME: &[u8] = b"H";
				const ERASE_SCREEN: &[u8] = b"2J";
				const ERASE_SCROLLBACK: &[u8] = b"3J";

				w.write_all(CSI)?;
				w.write_all(CURSOR_HOME)?;

				w.write_all(CSI)?;
				w.write_all(ERASE_SCREEN)?;

				w.write_all(CSI)?;
				w.write_all(ERASE_SCROLLBACK)?;

				w.flush()?;
			}
			Self::XtermReset => {
				const STR: &[u8] = b"!p";
				const RESET_WIDTH_AND_SCROLL: &[u8] = b"?3;4l";
				const RESET_REPLACE: &[u8] = b"4l";
				const RESET_KEYPAD: &[u8] = b">";
				const RESET_MARGINS: &[u8] = b"?69l";

				w.write_all(ESC)?;
				w.write_all(RIS)?;

				w.write_all(CSI)?;
				w.write_all(STR)?;

				w.write_all(CSI)?;
				w.write_all(RESET_WIDTH_AND_SCROLL)?;

				w.write_all(CSI)?;
				w.write_all(RESET_REPLACE)?;

				w.write_all(ESC)?;
				w.write_all(RESET_KEYPAD)?;

				w.write_all(CSI)?;
				w.write_all(RESET_MARGINS)?;

				w.flush()?;
			}
			Self::TputClear => {
				let status = Command::new("tput").arg("clear").status()?;
				if !status.success() {
					return Err(Error::Command("tput clear", status));
				}
			}
			Self::TputReset => {
				let status = Command::new("tput").arg("reset").status()?;
				if !status.success() {
					return Err(Error::Command("tput reset", status));
				}
			}
			Self::Cls => {
				let status = Command::new("cmd.exe").arg("/C").arg("cls").status()?;
				if !status.success() {
					return Err(Error::Command("cls", status));
				}
			}
			Self::WindowsVt => win::vt()?,
			Self::WindowsVtClear => {
				let vtres = win::vt();
				Self::XtermClear.clear_to(w)?;
				vtres?;
			}
			#[cfg(feature = "windows-console")]
			Self::WindowsConsoleClear => win::clear()?,
			#[cfg(feature = "windows-console")]
			Self::WindowsConsoleBlank => win::blank()?,
			Self::WindowsCooked => win::cooked()?,
			Self::VtRis => {
				w.write_all(ESC)?;
				w.write_all(RIS)?;
				w.flush()?;
			}
			Self::VtLeaveAlt => {
				const LEAVE_ALT: &[u8] = b"?1049l";
				w.write_all(CSI)?;
				w.write_all(LEAVE_ALT)?;
				w.flush()?;
			}
			Self::VtCooked => unix::vt_cooked()?,
			Self::VtWellDone => unix::vt_well_done()?,
		}

		Ok(())
	}
}

/// Shorthand for `ClearScreen::default().clear()`.
pub fn clear() -> Result<(), Error> {
	ClearScreen::default().clear()
}

/// Detects Microsoft Terminal.
///
/// Note that this is only provided to write your own clearscreen logic and _should not_ be relied
/// on for other purposes, as it makes no guarantees of reliable detection, and its internal
/// behaviour may change without notice.
pub fn is_microsoft_terminal() -> bool {
	env::var("WT_SESSION").is_ok()
}

/// Detects Windows ≥10.
///
/// As mentioned in the [`WindowsVt`][ClearScreen::WindowsVt] documentation, Windows 10 from the
/// Threshold 2 Update in November 2015 supports the `ENABLE_VIRTUAL_TERMINAL_PROCESSING` console
/// mode bit, which enables VT100/ECMA-48 escape sequence processing in the console. This in turn
/// makes clearing the console vastly easier and is the recommended mode of operation by Microsoft.
///
/// However, detecting Windows ≥10 is not trivial. To mitigate broken programs that incorrectly
/// perform version shimming, Microsoft has deprecated most ways to obtain the version of Windows by
/// making the relevant APIs _lie_ unless the calling executable [embeds a manifest that explicitely
/// opts-in to support Windows 10][manifesting].
///
/// To be clear, **this is the proper way to go**, and while this function tries, it may return
/// false under some Win10s if you don't manifest. If you are writing an application which uses this
/// library, or indeed any application targeting Windows at all, you should embed such a manifest
/// (and take that opportunity to opt-in to long path support, see e.g. [watchexec#163]). If you are
/// writing a library on top of this one, it is your responsibility to communicate this requirement
/// to your users.
///
/// It is important to remark that it is not possible to manifest twice. In plainer words,
/// **libraries _must not_ embed a manifest** as that will make it impossible for applications which
/// depend on them to embed their own manifest.
///
/// This function tries its best to detect Windows ≥10, and specifically, whether the mentioned mode
/// bit can be used. Critically, it leaves trying to set the bit as feature detection as a last
/// resort, such that _an error setting the bit_ is not confunded with _the bit not being supported_.
///
/// Note that this is only provided to write your own clearscreen logic and _should not_ be relied
/// on for other purposes, as it makes no guarantees of reliable detection, and its internal
/// behaviour may change without notice. Additionally, this will always return false if the library
/// was compiled for a non-Windows target, even if e.g. it’s running under WSL in a Windows 10 host.
///
/// TL;DR:
///
/// - Runs on Windows ≥10 without manifest and returns `true`: good, expected behaviour.
/// - Runs on Windows ≥10 without manifest and returns `false`: **not a bug**, please manifest.
/// - Runs on Windows ≥10 with manifest and returns `true`: good, expected behaviour.
/// - Runs on Windows ≥10 with manifest and returns `false`: **is a bug**, please report it.
/// - Runs on Windows <10 and returns `true`: **is a bug**, please report it. [ex #5]
/// - Runs on Windows <10 and returns `false`: good, expected behaviour.
///
/// [ex #5]: https://github.com/watchexec/clearscreen/issues/5
/// [manifesting]: https://docs.microsoft.com/en-us/windows/win32/sysinfo/targeting-your-application-at-windows-8-1
/// [watchexec#163]: https://github.com/watchexec/watchexec/issues/163
pub fn is_windows_10() -> bool {
	win::is_windows_10()
}

/// Error type.
#[derive(Debug, Error)]
pub enum Error {
	/// Any I/O error.
	#[error("io: {0}")]
	Io(#[from] io::Error),

	/// A non-success exit status from a command.
	#[error("command: {0}: {1}")]
	Command(&'static str, ExitStatus),

	/// Any nix (libc) error.
	#[cfg(unix)]
	#[error("unix: {0}")]
	Nix(#[from] NixError),

	/// Any terminfo error.
	#[error("terminfo: {0}")]
	Terminfo(#[from] TerminfoError),

	/// A missing terminfo capability.
	#[error("terminfo: capability not available: {0}")]
	TerminfoCap(&'static str),

	/// A null-pointer error.
	#[error("ffi: encountered a null pointer while reading {0}")]
	NullPtr(&'static str),
}

/// Nix error type.
///
/// This wraps a [`nix::Error`] to avoid directly exposing the type in the public API, which
/// required a breaking change every time `clearscreen` updated its `nix` version.
///
/// To obtain a nix error, convert this error to an `i32` then use [`nix::Error::from_raw`]:
///
/// Creating a [`NixError`] is explicitly not possible from the public API.
///
/// ```no_compile
/// let nix_error = nix::Error::from_raw(error.into());
/// assert_eq!(nix_error, nix::Error::EINVAL);
/// ```
#[cfg(unix)]
#[derive(Debug, Error)]
#[error(transparent)]
pub struct NixError(nix::Error);

#[cfg(unix)]
impl From<NixError> for i32 {
	fn from(err: NixError) -> Self {
		err.0 as _
	}
}

/// Terminfo error type.
///
/// This wraps a [`terminfo::Error`] to avoid directly exposing the type in the public API, which
/// required a breaking change every time `clearscreen` updated its `terminfo` version.
#[derive(Debug, Error)]
#[error("{description}")]
pub struct TerminfoError {
	inner: terminfo::Error,
	description: String,
}

impl From<terminfo::Error> for TerminfoError {
	fn from(inner: terminfo::Error) -> Self {
		Self {
			description: inner.to_string(),
			inner,
		}
	}
}

impl From<terminfo::Error> for Error {
	fn from(err: terminfo::Error) -> Self {
		Self::Terminfo(TerminfoError::from(err))
	}
}

#[cfg(unix)]
mod unix {
	use super::{Error, NixError};

	use nix::{
		sys::termios::{
			tcgetattr, tcsetattr, ControlFlags, InputFlags, LocalFlags, OutputFlags,
			SetArg::TCSANOW, Termios,
		},
		unistd::isatty,
	};

	use std::{fs::OpenOptions, io::stdin, os::fd::AsFd, os::unix::prelude::AsRawFd};

	pub(crate) fn vt_cooked() -> Result<(), Error> {
		write_termios(|t| {
			t.input_flags.insert(
				InputFlags::BRKINT
					| InputFlags::ICRNL
					| InputFlags::IGNPAR
					| InputFlags::ISTRIP
					| InputFlags::IXON,
			);
			t.output_flags.insert(OutputFlags::OPOST);
			t.local_flags.insert(LocalFlags::ICANON | LocalFlags::ISIG);
		})
	}

	pub(crate) fn vt_well_done() -> Result<(), Error> {
		write_termios(|t| {
			let mut inserts = InputFlags::BRKINT
				| InputFlags::ICRNL
				| InputFlags::IGNPAR
				| InputFlags::IMAXBEL
				| InputFlags::ISTRIP
				| InputFlags::IXON;

			#[cfg(any(target_os = "android", target_os = "linux", target_os = "macos"))]
			{
				inserts |= InputFlags::IUTF8;
			}

			t.input_flags.insert(inserts);
			t.output_flags
				.insert(OutputFlags::ONLCR | OutputFlags::OPOST);
			t.control_flags.insert(ControlFlags::CREAD);
			t.local_flags.insert(LocalFlags::ICANON | LocalFlags::ISIG);
		})
	}

	fn reset_termios(t: &mut Termios) {
		t.input_flags.remove(InputFlags::all());
		t.output_flags.remove(OutputFlags::all());
		t.control_flags.remove(ControlFlags::all());
		t.local_flags.remove(LocalFlags::all());
	}

	fn write_termios(f: impl Fn(&mut Termios)) -> Result<(), Error> {
		if isatty(stdin().as_raw_fd()).map_err(NixError)? {
			let mut t = tcgetattr(stdin().as_fd()).map_err(NixError)?;
			reset_termios(&mut t);
			f(&mut t);
			tcsetattr(stdin().as_fd(), TCSANOW, &t).map_err(NixError)?;
		} else {
			let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
			let fd = tty.as_fd();

			let mut t = tcgetattr(fd).map_err(NixError)?;
			reset_termios(&mut t);
			f(&mut t);
			tcsetattr(fd, TCSANOW, &t).map_err(NixError)?;
		}

		Ok(())
	}
}

#[cfg(windows)]
mod win {
	use super::Error;

	use std::{io, mem::size_of, ptr};

	use windows_sys::Win32::Foundation::{FALSE, HANDLE, INVALID_HANDLE_VALUE};
	use windows_sys::Win32::NetworkManagement::NetManagement::{
		NetApiBufferAllocate, NetApiBufferFree, NetServerGetInfo, NetWkstaGetInfo,
		MAJOR_VERSION_MASK, SERVER_INFO_101, SV_PLATFORM_ID_NT, WKSTA_INFO_100,
	};
	use windows_sys::Win32::System::Console::{
		GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE, ENABLE_ECHO_INPUT,
		ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
		STD_OUTPUT_HANDLE,
	};
	use windows_sys::Win32::System::SystemInformation::{
		VerSetConditionMask, VerifyVersionInfoW, OSVERSIONINFOEXW, VER_MAJORVERSION,
		VER_MINORVERSION, VER_SERVICEPACKMAJOR,
	};
	use windows_sys::Win32::System::SystemServices::VER_GREATER_EQUAL;

	#[cfg(feature = "windows-console")]
	use windows_sys::Win32::System::Console::{
		FillConsoleOutputAttribute, FillConsoleOutputCharacterW, GetConsoleScreenBufferInfo,
		ScrollConsoleScreenBufferW, SetConsoleCursorPosition, CHAR_INFO, CHAR_INFO_0,
		CONSOLE_SCREEN_BUFFER_INFO, COORD, SMALL_RECT,
	};

	fn console_handle() -> Result<HANDLE, Error> {
		match unsafe { GetStdHandle(STD_OUTPUT_HANDLE) } {
			INVALID_HANDLE_VALUE => Err(io::Error::last_os_error().into()),
			handle => Ok(handle),
		}
	}

	#[cfg(feature = "windows-console")]
	fn buffer_info(console: HANDLE) -> Result<CONSOLE_SCREEN_BUFFER_INFO, Error> {
		let csbi: *mut CONSOLE_SCREEN_BUFFER_INFO = ptr::null_mut();
		if unsafe { GetConsoleScreenBufferInfo(console, csbi) } == FALSE {
			return Err(io::Error::last_os_error().into());
		}

		if csbi.is_null() {
			Err(Error::NullPtr("GetConsoleScreenBufferInfo"))
		} else {
			Ok(unsafe { ptr::read(csbi) })
		}
	}

	pub(crate) fn vt() -> Result<(), Error> {
		let stdout = console_handle()?;

		let mut mode = 0;
		if unsafe { GetConsoleMode(stdout, &mut mode) } == FALSE {
			return Err(io::Error::last_os_error().into());
		}

		mode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING;
		if unsafe { SetConsoleMode(stdout, mode) } == FALSE {
			return Err(io::Error::last_os_error().into());
		}

		Ok(())
	}

	// Ref https://docs.microsoft.com/en-us/windows/console/clearing-the-screen#example-2
	#[cfg(feature = "windows-console")]
	pub(crate) fn clear() -> Result<(), Error> {
		let console = console_handle()?;
		let csbi = buffer_info(console)?;

		// Scroll the rectangle of the entire buffer.
		let rect = SMALL_RECT {
			Left: 0,
			Top: 0,
			Right: csbi.dwSize.X,
			Bottom: csbi.dwSize.Y,
		};

		// Scroll it upwards off the top of the buffer with a magnitude of the entire height.
		let target = COORD {
			X: 0,
			Y: (0 - csbi.dwSize.Y) as i16,
		};

		// Fill with empty spaces with the buffer’s default text attribute.
		let space = CHAR_INFO_0 {
			AsciiChar: b' ' as i8,
		};

		let fill = CHAR_INFO {
			Char: space,
			Attributes: csbi.wAttributes,
		};

		// Do the scroll.
		if unsafe { ScrollConsoleScreenBufferW(console, &rect, ptr::null(), target, &fill) }
			== FALSE
		{
			return Err(io::Error::last_os_error().into());
		}

		// Move the cursor to the top left corner too.
		let mut cursor = csbi.dwCursorPosition;
		cursor.X = 0;
		cursor.Y = 0;

		if unsafe { SetConsoleCursorPosition(console, cursor) } == FALSE {
			return Err(io::Error::last_os_error().into());
		}

		Ok(())
	}

	// Ref https://docs.microsoft.com/en-us/windows/console/clearing-the-screen#example-3
	#[cfg(feature = "windows-console")]
	pub(crate) fn blank() -> Result<(), Error> {
		let console = console_handle()?;

		// Fill the entire screen with blanks.
		let csbi = buffer_info(console)?;

		let buffer_size = csbi.dwSize.X * csbi.dwSize.Y;
		let home_coord = COORD { X: 0, Y: 0 };

		if FALSE
			== unsafe {
				FillConsoleOutputCharacterW(
					console,
					b' ' as u16,
					u32::try_from(buffer_size).unwrap_or(0),
					home_coord,
					ptr::null_mut(),
				)
			} {
			return Err(io::Error::last_os_error().into());
		}

		// Set the buffer's attributes accordingly.
		let csbi = buffer_info(console)?;
		if FALSE
			== unsafe {
				FillConsoleOutputAttribute(
					console,
					csbi.wAttributes,
					u32::try_from(buffer_size).unwrap_or(0),
					home_coord,
					ptr::null_mut(),
				)
			} {
			return Err(io::Error::last_os_error().into());
		}

		// Put the cursor at its home coordinates.
		if unsafe { SetConsoleCursorPosition(console, home_coord) } == FALSE {
			return Err(io::Error::last_os_error().into());
		}

		Ok(())
	}

	const ENABLE_COOKED_MODE: CONSOLE_MODE =
		ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT;

	pub(crate) fn cooked() -> Result<(), Error> {
		let stdout = console_handle()?;

		let mut mode = 0;
		if unsafe { GetConsoleMode(stdout, &mut mode) } == FALSE {
			return Err(io::Error::last_os_error().into());
		}

		mode |= ENABLE_COOKED_MODE;
		if unsafe { SetConsoleMode(stdout, mode) } == FALSE {
			return Err(io::Error::last_os_error().into());
		}

		Ok(())
	}

	// I hope someone searches for this one day and gets mad at me for making their life harder.
	const ABRACADABRA_THRESHOLD: (u8, u8) = (0x0A, 0x00);

	// proper way, requires manifesting
	#[inline]
	fn um_verify_version() -> bool {
		let condition_mask: u64 = unsafe {
			VerSetConditionMask(
				VerSetConditionMask(
					VerSetConditionMask(0, VER_MAJORVERSION, VER_GREATER_EQUAL as u8),
					VER_MINORVERSION,
					VER_GREATER_EQUAL as u8,
				),
				VER_SERVICEPACKMAJOR,
				VER_GREATER_EQUAL as u8,
			)
		};

		let mut osvi = OSVERSIONINFOEXW {
			dwMinorVersion: ABRACADABRA_THRESHOLD.1 as _,
			dwMajorVersion: ABRACADABRA_THRESHOLD.0 as _,
			wServicePackMajor: 0,
			dwOSVersionInfoSize: size_of::<OSVERSIONINFOEXW>() as u32,
			dwBuildNumber: 0,
			dwPlatformId: 0,
			szCSDVersion: [0; 128],
			wServicePackMinor: 0,
			wSuiteMask: 0,
			wProductType: 0,
			wReserved: 0,
		};

		let ret = unsafe {
			VerifyVersionInfoW(
				&mut osvi,
				VER_MAJORVERSION | VER_MINORVERSION | VER_SERVICEPACKMAJOR,
				condition_mask,
			)
		};

		ret != FALSE
	}

	// querying the local netserver management api?
	#[inline]
	fn um_netserver() -> Result<bool, Error> {
		unsafe {
			let mut buf = ptr::null_mut();
			match NetApiBufferAllocate(
				u32::try_from(size_of::<SERVER_INFO_101>()).unwrap(),
				&mut buf,
			) {
				0 => {}
				err => return Err(io::Error::from_raw_os_error(i32::try_from(err).unwrap()).into()),
			}

			let ret = match NetServerGetInfo(ptr::null_mut(), 101, buf as _) {
				0 => {
					let info: SERVER_INFO_101 = ptr::read(buf as _);
					let version = info.sv101_version_major | MAJOR_VERSION_MASK;

					// IS it using the same magic version number? who the fuck knows. let's hope so.
					Ok(info.sv101_platform_id == SV_PLATFORM_ID_NT
						&& version > ABRACADABRA_THRESHOLD.0 as _)
				}
				err => Err(io::Error::from_raw_os_error(i32::try_from(err).unwrap()).into()),
			};

			// always free, even if the netservergetinfo call fails
			match NetApiBufferFree(buf) {
				0 => {}
				err => return Err(io::Error::from_raw_os_error(i32::try_from(err).unwrap()).into()),
			}

			ret
		}
	}

	// querying the local workstation management api?
	#[inline]
	fn um_workstation() -> Result<bool, Error> {
		unsafe {
			let mut buf = ptr::null_mut();
			match NetApiBufferAllocate(
				u32::try_from(size_of::<WKSTA_INFO_100>()).unwrap(),
				&mut buf,
			) {
				0 => {}
				err => return Err(io::Error::from_raw_os_error(i32::try_from(err).unwrap()).into()),
			}

			let ret = match NetWkstaGetInfo(ptr::null_mut(), 100, buf as _) {
				0 => {
					let info: WKSTA_INFO_100 = ptr::read(buf as _);

					// IS it using the same magic version number? who the fuck knows. let's hope so.
					Ok(info.wki100_platform_id == SV_PLATFORM_ID_NT
						&& info.wki100_ver_major > ABRACADABRA_THRESHOLD.0 as _)
				}
				err => Err(io::Error::from_raw_os_error(i32::try_from(err).unwrap()).into()),
			};

			// always free, even if the netservergetinfo call fails
			match NetApiBufferFree(buf) {
				0 => {}
				err => return Err(io::Error::from_raw_os_error(i32::try_from(err).unwrap()).into()),
			}

			ret
		}
	}

	// attempt to set the bit, then undo it
	fn vt_attempt() -> Result<bool, Error> {
		let stdout = console_handle()?;

		let mut mode = 0;
		if unsafe { GetConsoleMode(stdout, &mut mode) } == FALSE {
			return Err(io::Error::last_os_error().into());
		}

		let mut support = false;

		let mut newmode = mode;
		newmode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING;
		if unsafe { SetConsoleMode(stdout, newmode) } != FALSE {
			support = true;
		}

		// reset it to original value, whatever we do
		unsafe { SetConsoleMode(stdout, mode) };

		Ok(support)
	}

	#[inline]
	pub(crate) fn is_windows_10() -> bool {
		if um_verify_version() {
			return true;
		}

		if um_netserver().unwrap_or(false) {
			return true;
		}

		if um_workstation().unwrap_or(false) {
			return true;
		}

		vt_attempt().unwrap_or(false)
	}
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
mod unix {
	use super::Error;

	pub(crate) fn vt_cooked() -> Result<(), Error> {
		Ok(())
	}

	pub(crate) fn vt_well_done() -> Result<(), Error> {
		Ok(())
	}
}

#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
mod win {
	use super::Error;

	pub(crate) fn vt() -> Result<(), Error> {
		Ok(())
	}

	#[cfg(feature = "windows-console")]
	pub(crate) fn clear() -> Result<(), Error> {
		Ok(())
	}

	#[cfg(feature = "windows-console")]
	pub(crate) fn blank() -> Result<(), Error> {
		Ok(())
	}

	pub(crate) fn cooked() -> Result<(), Error> {
		Ok(())
	}

	#[inline]
	pub(crate) fn is_windows_10() -> bool {
		false
	}
}

#[derive(Eq, PartialEq, Clone, Debug)]
struct ResetScrollback<'a>(Cow<'a, [u8]>);

impl<'a> Capability<'a> for ResetScrollback<'a> {
	#[inline]
	fn name() -> &'static str {
		"E3"
	}

	#[inline]
	fn from(value: Option<&'a Value>) -> Option<Self> {
		if let Some(Value::String(value)) = value {
			Some(Self(Cow::Borrowed(value)))
		} else {
			None
		}
	}

	#[inline]
	fn into(self) -> Option<Value> {
		Some(Value::String(match self.0 {
			Cow::Borrowed(value) => value.into(),

			Cow::Owned(value) => value,
		}))
	}
}

impl<'a, T: AsRef<&'a [u8]>> From<T> for ResetScrollback<'a> {
	#[inline]
	fn from(value: T) -> Self {
		Self(Cow::Borrowed(value.as_ref()))
	}
}

impl AsRef<[u8]> for ResetScrollback<'_> {
	#[inline]
	fn as_ref(&self) -> &[u8] {
		&self.0
	}
}

impl ResetScrollback<'_> {
	#[inline]
	fn expand(&self) -> Expansion<Self> {
		#[allow(dead_code)]
		struct ExpansionHere<'a, T: 'a + AsRef<[u8]>> {
			string: &'a T,
			params: [Parameter; 9],
			context: Option<&'a mut Context>,
		}

		let here = ExpansionHere {
			string: self,
			params: Default::default(),
			context: None,
		};

		// UNSAFE >:( this is iffy af but also the only way to create an Expansion
		// such that we can add the E3 capability.
		unsafe { std::mem::transmute(here) }
	}
}
