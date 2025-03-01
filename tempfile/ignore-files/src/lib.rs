//! Find, parse, and interpret ignore files.
//!
//! Ignore files are files that contain ignore patterns, often following the `.gitignore` format.
//! There may be one or more global ignore files, which apply everywhere, and one or more per-folder
//! ignore files, which apply to a specific folder and its subfolders. Furthermore, there may be
//! more ignore files in _these_ subfolders, and so on. Discovering and interpreting all of these in
//! a single context is not a simple task: this is what this crate provides.

use std::path::{Path, PathBuf};

use normalize_path::NormalizePath;
use project_origins::ProjectType;

#[doc(inline)]
pub use discover::*;
mod discover;

#[doc(inline)]
pub use error::*;
mod error;

#[doc(inline)]
pub use filter::*;
mod filter;

/// An ignore file.
///
/// This records both the path to the ignore file and some basic metadata about it: which project
/// type it applies to if any, and which subtree it applies in if any (`None` = global ignore file).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IgnoreFile {
	/// The path to the ignore file.
	pub path: PathBuf,

	/// The path to the subtree the ignore file applies to, or `None` for global ignores.
	pub applies_in: Option<PathBuf>,

	/// Which project type the ignore file applies to, or was found through.
	pub applies_to: Option<ProjectType>,
}

pub(crate) fn simplify_path(path: &Path) -> PathBuf {
	dunce::simplified(path).normalize()
}
