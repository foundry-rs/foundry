use std::path::PathBuf;

use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum Error {
	/// Error received when an [`IgnoreFile`] cannot be read.
	///
	/// [`IgnoreFile`]: crate::IgnoreFile
	#[error("cannot read ignore '{file}': {err}")]
	Read {
		/// The path to the erroring ignore file.
		file: PathBuf,

		/// The underlying error.
		#[source]
		err: std::io::Error,
	},

	/// Error received when parsing a glob fails.
	#[error("cannot parse glob from ignore '{file:?}': {err}")]
	Glob {
		/// The path to the erroring ignore file.
		file: Option<PathBuf>,

		/// The underlying error.
		#[source]
		err: ignore::Error,
		// TODO: extract glob error into diagnostic
	},

	/// Multiple related [`Error`](enum@Error)s.
	#[error("multiple: {0:?}")]
	Multi(#[related] Vec<Error>),

	/// Error received when trying to canonicalize a path
	#[error("cannot canonicalize '{path:?}'")]
	Canonicalize {
		/// the path that cannot be canonicalized
		path: PathBuf,

		/// the underlying error
		#[source]
		err: std::io::Error,
	},
}
