//! Generic error types used throughout the crate.

use std::{
	error::Error,
	fmt::{self, Debug, Display, Formatter, Write},
	io,
};

/// General error type for fallible constructors.
///
/// In Interprocess, many types feature conversions to and from handles/file descriptors and types
/// from the standard library. Many of those conversions are fallible because the semantic mapping
/// between the source and destination types is not always 1:1, with various invariants that need to
/// be upheld and which are always queried for. With async types, this is further complicated:
/// runtimes typically need to register OS objects in their polling/completion infrastructure to use
/// them asynchronously.
///
/// All those conversion have one thing in common: they consume ownership of one object and return
/// ownership of its new form. If the conversion fails, it would be invaluably useful in some cases
/// to return ownership of the original object back to the caller, so that it could use it in some
/// other way. The `source` field allows for that, but also reserves the callee's freedom not to do
/// so. (Hey, it's not like I *wanted* it to be like this, Tokio just doesn't have `.try_clone()`
/// and returns [`io::Error`]. Oh well, unwrapping's always an option.)
///
/// Many (but not all) of those conversions also have an OS error they can attribute the failure to.
///
/// Additionally, some conversions have an additional "details" error field that contains extra
/// infromation about the error. That is typically an enumeration that specifies which stage of the
/// conversion the OS error happened on.
#[derive(Debug)]
pub struct ConversionError<S, E = NoDetails> {
	/// Extra information about the error.
	pub details: E,
	/// The underlying OS error, if any.
	pub cause: Option<io::Error>,
	/// Ownership of the input of the conversion.
	pub source: Option<S>,
}
impl<S, E: Default> ConversionError<S, E> {
	/// Constructs an error value without an OS cause and with default contents for the "details"
	/// field.
	pub fn from_source(source: S) -> Self {
		Self {
			details: Default::default(),
			cause: None,
			source: Some(source),
		}
	}
	/// Constructs an error value that doesn't return input ownership, with default contents for the
	/// "details" field and an OS cause.
	pub fn from_cause(cause: io::Error) -> Self {
		Self {
			details: Default::default(),
			cause: Some(cause),
			source: None,
		}
	}
	/// Constructs an error value from a given OS cause, filling the "details" field with its
	/// default value.
	pub fn from_source_and_cause(source: S, cause: io::Error) -> Self {
		Self {
			details: Default::default(),
			cause: Some(cause),
			source: Some(source),
		}
	}
}
impl<S, E> ConversionError<S, E> {
	/// Constructs an error value without an OS cause.
	pub fn from_source_and_details(source: S, details: E) -> Self {
		Self {
			details,
			cause: None,
			source: Some(source),
		}
	}
	/// Constructs an error value that doesn't return input ownership.
	pub fn from_cause_and_details(cause: io::Error, details: E) -> Self {
		Self {
			details,
			cause: Some(cause),
			source: None,
		}
	}
	/// Maps the type with which ownership over the input is returned using the given closure.
	///
	/// This utility is mostly used in the crate's internals.
	pub fn map_source<Sb>(self, f: impl FnOnce(S) -> Sb) -> ConversionError<Sb, E> {
		ConversionError {
			details: self.details,
			cause: self.cause,
			source: self.source.map(f),
		}
	}
	/// Maps the type with which ownership over the input is returned using the given closure, also
	/// allowing it to drop the ownership.
	///
	/// This utility is mostly used in the crate's internals.
	pub fn try_map_source<Sb>(self, f: impl FnOnce(S) -> Option<Sb>) -> ConversionError<Sb, E> {
		ConversionError {
			details: self.details,
			cause: self.cause,
			source: self.source.and_then(f),
		}
	}
}
impl<S, E: Display> ConversionError<S, E> {
	/// Boxes the error into an `io::Error`.
	pub fn to_io_error(&self) -> io::Error {
		let msg = self.to_string();
		io::Error::new(io::ErrorKind::Other, msg)
	}
}
/// Boxes the error into an `io::Error`, dropping the retained file descriptor in the process.
impl<S, E: Display> From<ConversionError<S, E>> for io::Error {
	fn from(e: ConversionError<S, E>) -> Self {
		e.to_io_error()
	}
}
impl<S, E: Default> Default for ConversionError<S, E> {
	fn default() -> Self {
		Self {
			details: Default::default(),
			cause: None,
			source: None,
		}
	}
}
impl<S, E: Display> Display for ConversionError<S, E> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		let mut snp = FormatSnooper::new(f);
		write!(snp, "{}", &self.details)?;
		if let Some(e) = &self.cause {
			if snp.anything_written() {
				f.write_str(": ")?;
			}
			Display::fmt(e, f)?;
		}
		Ok(())
	}
}
impl<S: Debug, E: Error + 'static> Error for ConversionError<S, E> {
	#[inline]
	#[allow(clippy::as_conversions)]
	fn source(&self) -> Option<&(dyn Error + 'static)> {
		self.cause.as_ref().map(|r| r as &_)
	}
}

/// Thunk type used to specialize on the type of `details`, preventing ": " from being at the
/// beginning of the output with nothing preceding it.
struct FormatSnooper<'a, 'b> {
	formatter: &'b mut Formatter<'a>,
	anything_written: bool,
}
impl<'a, 'b> FormatSnooper<'a, 'b> {
	fn new(formatter: &'b mut Formatter<'a>) -> Self {
		Self {
			formatter,
			anything_written: false,
		}
	}
	fn anything_written(&self) -> bool {
		self.anything_written
	}
}
impl Write for FormatSnooper<'_, '_> {
	fn write_str(&mut self, s: &str) -> fmt::Result {
		if !s.is_empty() {
			self.anything_written = true;
			self.formatter.write_str(s)
		} else {
			Ok(())
		}
	}
}

/// Marker type used as the generic argument of [`ConversionError`] to denote that there are no
/// error details.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct NoDetails;
impl Display for NoDetails {
	fn fmt(&self, _f: &mut Formatter<'_>) -> fmt::Result {
		Ok(()) //
	}
}

/// Error type of `TryFrom<OwnedHandle>` conversions.
#[cfg(windows)]
#[cfg_attr(feature = "doc_cfg", doc(cfg(windows)))]
pub type FromHandleError<E = NoDetails> = ConversionError<std::os::windows::io::OwnedHandle, E>;

/// Error type of `TryFrom<OwnedFd>` conversions.
#[cfg(unix)]
#[cfg_attr(feature = "doc_cfg", doc(cfg(unix)))]
pub type FromFdError<E = NoDetails> = ConversionError<std::os::unix::io::OwnedFd, E>;

/// Error type of `.reunite()` on splittable stream types, indicating that the two halves belong to
/// different streams.
#[derive(Debug)]
pub struct ReuniteError<R, S> {
	/// Ownership of the receive half.
	pub rh: R,
	/// Ownership of the send half.
	pub sh: S,
}
impl<R, S> ReuniteError<R, S> {
	/// Maps the halves of the stream using the given closures.
	#[inline]
	pub fn map_halves<NR: From<R>, NS: From<S>>(
		self,
		fr: impl FnOnce(R) -> NR,
		fs: impl FnOnce(S) -> NS,
	) -> ReuniteError<NR, NS> {
		let Self { rh, sh } = self;
		ReuniteError {
			rh: fr(rh),
			sh: fs(sh),
		}
	}
	/// Maps the halves of the stream using the [`From`] trait.
	///
	/// This is useful when implementing wrappers around stream types.
	#[inline]
	pub fn convert_halves<NR: From<R>, NS: From<S>>(self) -> ReuniteError<NR, NS> {
		self.map_halves(From::from, From::from)
	}
}
impl<R, S> Display for ReuniteError<R, S> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.write_str("attempt to reunite stream halves that come from different streams")
	}
}
impl<R: Debug, S: Debug> Error for ReuniteError<R, S> {}

/// Result type of `.reunite()` on splittable stream types.
pub type ReuniteResult<T, R, S> = Result<T, ReuniteError<R, S>>;
