use std::{
	collections::HashSet,
	env,
	io::{Error, ErrorKind},
	path::{Path, PathBuf},
};

use futures::future::try_join_all;
use gix_config::{path::interpolate::Context as InterpolateContext, File, Path as GitPath};
use miette::{bail, Result};
use normalize_path::NormalizePath;
use project_origins::ProjectType;
use tokio::fs::{canonicalize, metadata, read_dir};
use tracing::{trace, trace_span};

use crate::{IgnoreFile, IgnoreFilter};

/// Arguments for finding ignored files in a given directory and subdirectories
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct IgnoreFilesFromOriginArgs {
	/// Origin from which finding ignored files will start.
	pub origin: PathBuf,

	/// Paths that have been explicitly selected to be watched.
	///
	/// If this list is non-empty, all paths not on this list will be ignored.
	///
	/// These paths *must* be absolute and normalised (no `.` and `..` components).
	pub explicit_watches: Vec<PathBuf>,

	/// Paths that have been explicitly ignored.
	///
	/// If this list is non-empty, all paths on this list will be ignored.
	///
	/// These paths *must* be absolute and normalised (no `.` and `..` components).
	pub explicit_ignores: Vec<PathBuf>,
}

impl IgnoreFilesFromOriginArgs {
	/// Check that this struct is correctly-formed.
	pub fn check(&self) -> Result<()> {
		if self.explicit_watches.iter().any(|p| !p.is_absolute()) {
			bail!("explicit_watches contains non-absolute paths");
		}
		if self.explicit_watches.iter().any(|p| !p.is_normalized()) {
			bail!("explicit_watches contains non-normalised paths");
		}
		if self.explicit_ignores.iter().any(|p| !p.is_absolute()) {
			bail!("explicit_ignores contains non-absolute paths");
		}
		if self.explicit_ignores.iter().any(|p| !p.is_normalized()) {
			bail!("explicit_ignores contains non-normalised paths");
		}

		Ok(())
	}

	/// Canonicalise all paths.
	///
	/// The result is always well-formed.
	pub async fn canonicalise(self) -> std::io::Result<Self> {
		Ok(Self {
			origin: canonicalize(&self.origin).await?,
			explicit_watches: try_join_all(self.explicit_watches.into_iter().map(canonicalize))
				.await?,
			explicit_ignores: try_join_all(self.explicit_ignores.into_iter().map(canonicalize))
				.await?,
		})
	}

	/// Create args with all fields set and check that they are correctly-formed.
	pub fn new(
		origin: impl AsRef<Path>,
		explicit_watches: Vec<PathBuf>,
		explicit_ignores: Vec<PathBuf>,
	) -> Result<Self> {
		let this = Self {
			origin: PathBuf::from(origin.as_ref()),
			explicit_watches,
			explicit_ignores,
		};
		this.check()?;
		Ok(this)
	}

	/// Create args without checking well-formed-ness.
	///
	/// Use this only if you know that the args are well-formed, or if you are about to call
	/// [`canonicalise()`][IgnoreFilesFromOriginArgs::canonicalise()] on them.
	pub fn new_unchecked(
		origin: impl AsRef<Path>,
		explicit_watches: impl IntoIterator<Item = impl Into<PathBuf>>,
		explicit_ignores: impl IntoIterator<Item = impl Into<PathBuf>>,
	) -> Self {
		Self {
			origin: origin.as_ref().into(),
			explicit_watches: explicit_watches.into_iter().map(Into::into).collect(),
			explicit_ignores: explicit_ignores.into_iter().map(Into::into).collect(),
		}
	}
}

impl From<&Path> for IgnoreFilesFromOriginArgs {
	fn from(path: &Path) -> Self {
		Self {
			origin: path.into(),
			..Default::default()
		}
	}
}

/// Finds all ignore files in the given directory and subdirectories.
///
/// This considers:
/// - Git ignore files (`.gitignore`)
/// - Mercurial ignore files (`.hgignore`)
/// - Tool-generic `.ignore` files
/// - `.git/info/exclude` files in the `path` directory only
/// - Git configurable project ignore files (with `core.excludesFile` in `.git/config`)
///
/// Importantly, this should be called from the origin of the project, not a subfolder. This
/// function will not discover the project origin, and will not traverse parent directories. Use the
/// `project-origins` crate for that.
///
/// This function also does not distinguish between project folder types, and collects all files for
/// all supported VCSs and other project types. Use the `applies_to` field to filter the results.
///
/// All errors (permissions, etc) are collected and returned alongside the ignore files: you may
/// want to show them to the user while still using whatever ignores were successfully found. Errors
/// from files not being found are silently ignored (the files are just not returned).
///
/// ## Special case: project-local git config specifying `core.excludesFile`
///
/// If the project's `.git/config` specifies a value for `core.excludesFile`, this function will
/// return an `IgnoreFile { path: path/to/that/file, applies_in: None, applies_to: Some(ProjectType::Git) }`.
/// This is the only case in which the `applies_in` field is None from this function. When such is
/// received the global Git ignore files found by [`from_environment()`] **should be ignored**.
///
/// ## Async
///
/// This future is not `Send` due to [`gix_config`] internals.
///
/// ## Panics
///
/// This function panics if the `args` are not correctly-formed; this can be checked beforehand
/// without panicking with [`IgnoreFilesFromOriginArgs::check()`].
#[expect(
	clippy::future_not_send,
	reason = "gix_config internals, if this changes: update the doc"
)]
#[allow(
	clippy::too_many_lines,
	reason = "it's just the discover_file calls that explode the line count"
)]
pub async fn from_origin(
	args: impl Into<IgnoreFilesFromOriginArgs>,
) -> (Vec<IgnoreFile>, Vec<Error>) {
	let args = args.into();
	args.check()
		.expect("checking well-formedness of IgnoreFilesFromOriginArgs");

	let origin = &args.origin;
	let mut ignore_files = args
		.explicit_ignores
		.iter()
		.map(|p| IgnoreFile {
			path: p.clone(),
			applies_in: Some(origin.clone()),
			applies_to: None,
		})
		.collect();
	let mut errors = Vec::new();

	match find_file(origin.join(".git/config")).await {
		Err(err) => errors.push(err),
		Ok(None) => {}
		Ok(Some(path)) => match path.parent().map(|path| File::from_git_dir(path.into())) {
			None => errors.push(Error::new(
				ErrorKind::Other,
				"unreachable: .git/config must have a parent",
			)),
			Some(Err(err)) => errors.push(Error::new(ErrorKind::Other, err)),
			Some(Ok(config)) => {
				let config_excludes = config.value::<GitPath<'_>>("core.excludesFile");
				if let Ok(excludes) = config_excludes {
					match excludes.interpolate(InterpolateContext {
						home_dir: env::var("HOME").ok().map(PathBuf::from).as_deref(),
						..Default::default()
					}) {
						Ok(e) => {
							discover_file(
								&mut ignore_files,
								&mut errors,
								None,
								Some(ProjectType::Git),
								e.into(),
							)
							.await;
						}
						Err(err) => {
							errors.push(Error::new(ErrorKind::Other, err));
						}
					}
				}
			}
		},
	}

	discover_file(
		&mut ignore_files,
		&mut errors,
		Some(origin.clone()),
		Some(ProjectType::Bazaar),
		origin.join(".bzrignore"),
	)
	.await;

	discover_file(
		&mut ignore_files,
		&mut errors,
		Some(origin.clone()),
		Some(ProjectType::Darcs),
		origin.join("_darcs/prefs/boring"),
	)
	.await;

	discover_file(
		&mut ignore_files,
		&mut errors,
		Some(origin.clone()),
		Some(ProjectType::Fossil),
		origin.join(".fossil-settings/ignore-glob"),
	)
	.await;

	discover_file(
		&mut ignore_files,
		&mut errors,
		Some(origin.clone()),
		Some(ProjectType::Git),
		origin.join(".git/info/exclude"),
	)
	.await;

	trace!("visiting child directories for ignore files");
	match DirTourist::new(origin, &ignore_files, &args.explicit_watches).await {
		Ok(mut dirs) => {
			loop {
				match dirs.next().await {
					Visit::Done => break,
					Visit::Skip => continue,
					Visit::Find(dir) => {
						// Attempt to find a .ignore file in the directory
						if discover_file(
							&mut ignore_files,
							&mut errors,
							Some(dir.clone()),
							None,
							dir.join(".ignore"),
						)
						.await
						{
							dirs.add_last_file_to_filter(&ignore_files, &mut errors)
								.await;
						}

						// Attempt to find a .gitignore file in the directory
						if discover_file(
							&mut ignore_files,
							&mut errors,
							Some(dir.clone()),
							Some(ProjectType::Git),
							dir.join(".gitignore"),
						)
						.await
						{
							dirs.add_last_file_to_filter(&ignore_files, &mut errors)
								.await;
						}

						// Attempt to find a .hgignore file in the directory
						if discover_file(
							&mut ignore_files,
							&mut errors,
							Some(dir.clone()),
							Some(ProjectType::Mercurial),
							dir.join(".hgignore"),
						)
						.await
						{
							dirs.add_last_file_to_filter(&ignore_files, &mut errors)
								.await;
						}
					}
				}
			}
			errors.extend(dirs.errors);
		}
		Err(err) => {
			errors.push(err);
		}
	}

	(ignore_files, errors)
}

/// Finds all ignore files that apply to the current runtime.
///
/// Takes an optional `appname` for the calling application for application-specific config files.
///
/// This considers:
/// - User-specific git ignore files (e.g. `~/.gitignore`)
/// - Git configurable ignore files (e.g. with `core.excludesFile` in system or user config)
/// - `$XDG_CONFIG_HOME/{appname}/ignore`, as well as other locations (APPDATA on Windows…)
///
/// All errors (permissions, etc) are collected and returned alongside the ignore files: you may
/// want to show them to the user while still using whatever ignores were successfully found. Errors
/// from files not being found are silently ignored (the files are just not returned).
///
/// ## Async
///
/// This future is not `Send` due to [`gix_config`] internals.
#[expect(
	clippy::future_not_send,
	reason = "gix_config internals, if this changes: update the doc"
)]
#[allow(clippy::too_many_lines, reason = "clearer than broken up needlessly")]
pub async fn from_environment(appname: Option<&str>) -> (Vec<IgnoreFile>, Vec<Error>) {
	let mut files = Vec::new();
	let mut errors = Vec::new();

	let mut found_git_global = false;
	match File::from_environment_overrides().map(|mut env| {
		File::from_globals().map(move |glo| {
			env.append(glo);
			env
		})
	}) {
		Err(err) => errors.push(Error::new(ErrorKind::Other, err)),
		Ok(Err(err)) => errors.push(Error::new(ErrorKind::Other, err)),
		Ok(Ok(config)) => {
			let config_excludes = config.value::<GitPath<'_>>("core.excludesFile");
			if let Ok(excludes) = config_excludes {
				match excludes.interpolate(InterpolateContext {
					home_dir: env::var("HOME").ok().map(PathBuf::from).as_deref(),
					..Default::default()
				}) {
					Ok(e) => {
						if discover_file(
							&mut files,
							&mut errors,
							None,
							Some(ProjectType::Git),
							e.into(),
						)
						.await
						{
							found_git_global = true;
						}
					}
					Err(err) => {
						errors.push(Error::new(ErrorKind::Other, err));
					}
				}
			}
		}
	}

	if !found_git_global {
		let mut tries = Vec::with_capacity(5);
		if let Ok(home) = env::var("XDG_CONFIG_HOME") {
			tries.push(Path::new(&home).join("git/ignore"));
		}
		if let Ok(home) = env::var("APPDATA") {
			tries.push(Path::new(&home).join(".gitignore"));
		}
		if let Ok(home) = env::var("USERPROFILE") {
			tries.push(Path::new(&home).join(".gitignore"));
		}
		if let Ok(home) = env::var("HOME") {
			tries.push(Path::new(&home).join(".config/git/ignore"));
			tries.push(Path::new(&home).join(".gitignore"));
		}

		for path in tries {
			if discover_file(&mut files, &mut errors, None, Some(ProjectType::Git), path).await {
				break;
			}
		}
	}

	let mut bzrs = Vec::with_capacity(5);
	if let Ok(home) = env::var("APPDATA") {
		bzrs.push(Path::new(&home).join("Bazzar/2.0/ignore"));
	}
	if let Ok(home) = env::var("HOME") {
		bzrs.push(Path::new(&home).join(".bazarr/ignore"));
	}

	for path in bzrs {
		if discover_file(
			&mut files,
			&mut errors,
			None,
			Some(ProjectType::Bazaar),
			path,
		)
		.await
		{
			break;
		}
	}

	if let Some(name) = appname {
		let mut wgis = Vec::with_capacity(4);
		if let Ok(home) = env::var("XDG_CONFIG_HOME") {
			wgis.push(Path::new(&home).join(format!("{name}/ignore")));
		}
		if let Ok(home) = env::var("APPDATA") {
			wgis.push(Path::new(&home).join(format!("{name}/ignore")));
		}
		if let Ok(home) = env::var("USERPROFILE") {
			wgis.push(Path::new(&home).join(format!(".{name}/ignore")));
		}
		if let Ok(home) = env::var("HOME") {
			wgis.push(Path::new(&home).join(format!(".{name}/ignore")));
		}

		for path in wgis {
			if discover_file(&mut files, &mut errors, None, None, path).await {
				break;
			}
		}
	}

	(files, errors)
}

// TODO: add context to these errors

/// Utility function to handle looking for an ignore file and adding it to a list if found.
///
/// This is mostly an internal function, but it is exposed for other filterers to use.
#[allow(clippy::future_not_send)]
#[tracing::instrument(skip(files, errors), level = "trace")]
#[inline]
pub async fn discover_file(
	files: &mut Vec<IgnoreFile>,
	errors: &mut Vec<Error>,
	applies_in: Option<PathBuf>,
	applies_to: Option<ProjectType>,
	path: PathBuf,
) -> bool {
	match find_file(path).await {
		Err(err) => {
			trace!(?err, "found an error");
			errors.push(err);
			false
		}
		Ok(None) => {
			trace!("found nothing");
			false
		}
		Ok(Some(path)) => {
			trace!(?path, "found a file");
			files.push(IgnoreFile {
				path,
				applies_in,
				applies_to,
			});
			true
		}
	}
}

async fn find_file(path: PathBuf) -> Result<Option<PathBuf>, Error> {
	match metadata(&path).await {
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(err) => Err(err),
		Ok(meta) if meta.is_file() && meta.len() > 0 => Ok(Some(path)),
		Ok(_) => Ok(None),
	}
}

#[derive(Debug)]
struct DirTourist {
	base: PathBuf,
	to_visit: Vec<PathBuf>,
	to_skip: HashSet<PathBuf>,
	to_explicitly_watch: HashSet<PathBuf>,
	pub errors: Vec<std::io::Error>,
	filter: IgnoreFilter,
}

#[derive(Debug)]
enum Visit {
	Find(PathBuf),
	Skip,
	Done,
}

impl DirTourist {
	pub async fn new(
		base: &Path,
		ignore_files: &[IgnoreFile],
		watch_files: &[PathBuf],
	) -> Result<Self, Error> {
		let base = canonicalize(base).await?;
		trace!("create IgnoreFilterer for visiting directories");
		let mut filter = IgnoreFilter::new(&base, ignore_files)
			.await
			.map_err(|err| Error::new(ErrorKind::Other, err))?;

		filter
			.add_globs(
				&[
					"/.git",
					"/.hg",
					"/.bzr",
					"/_darcs",
					"/.fossil-settings",
					"/.svn",
					"/.pijul",
				],
				Some(&base),
			)
			.map_err(|err| Error::new(ErrorKind::Other, err))?;

		Ok(Self {
			to_visit: vec![base.clone()],
			base,
			to_skip: HashSet::new(),
			to_explicitly_watch: watch_files.iter().cloned().collect(),
			errors: Vec::new(),
			filter,
		})
	}

	#[allow(clippy::future_not_send)]
	pub async fn next(&mut self) -> Visit {
		if let Some(path) = self.to_visit.pop() {
			self.visit_path(path).await
		} else {
			Visit::Done
		}
	}

	#[allow(clippy::future_not_send)]
	#[tracing::instrument(skip(self), level = "trace")]
	async fn visit_path(&mut self, path: PathBuf) -> Visit {
		if self.must_skip(&path) {
			trace!("in skip list");
			return Visit::Skip;
		}

		if !self.filter.check_dir(&path) {
			trace!(?path, "path is ignored, adding to skip list");
			self.skip(path);
			return Visit::Skip;
		}

		// If explicitly watched paths were not specified, we can include any path
		//
		// If explicitly watched paths *were* specified, then to include the path, either:
		// - the path in question starts with an explicitly included path (/a/b starting with /a)
		// - the path in question is *above* the explicitly included path (/a is above /a/b)
		if self.to_explicitly_watch.is_empty()
			|| self
				.to_explicitly_watch
				.iter()
				.any(|p| path.starts_with(p) || p.starts_with(&path))
		{
			trace!(?path, ?self.to_explicitly_watch, "including path; it starts with one of the explicitly watched paths");
		} else {
			trace!(?path, ?self.to_explicitly_watch, "excluding path; it did not start with any of explicitly watched paths");
			self.skip(path);
			return Visit::Skip;
		}

		let mut dir = match read_dir(&path).await {
			Ok(dir) => dir,
			Err(err) => {
				trace!("failed to read dir: {}", err);
				self.errors.push(err);
				return Visit::Skip;
			}
		};

		while let Some(entry) = match dir.next_entry().await {
			Ok(entry) => entry,
			Err(err) => {
				trace!("failed to read dir entries: {}", err);
				self.errors.push(err);
				return Visit::Skip;
			}
		} {
			let path = entry.path();
			let _span = trace_span!("dir_entry", ?path).entered();

			if self.must_skip(&path) {
				trace!("in skip list");
				continue;
			}

			match entry.file_type().await {
				Ok(ft) => {
					if ft.is_dir() {
						if !self.filter.check_dir(&path) {
							trace!("path is ignored, adding to skip list");
							self.skip(path);
							continue;
						}

						trace!("found a dir, adding to list");
						self.to_visit.push(path);
					} else {
						trace!("not a dir");
					}
				}
				Err(err) => {
					trace!("failed to read filetype, adding to skip list: {}", err);
					self.errors.push(err);
					self.skip(path);
				}
			}
		}

		Visit::Find(path)
	}

	pub fn skip(&mut self, path: PathBuf) {
		let check_path = path.as_path();
		self.to_visit.retain(|p| !p.starts_with(check_path));
		self.to_skip.insert(path);
	}

	pub(crate) async fn add_last_file_to_filter(
		&mut self,
		files: &[IgnoreFile],
		errors: &mut Vec<Error>,
	) {
		if let Some(ig) = files.last() {
			if let Err(err) = self.filter.add_file(ig).await {
				errors.push(Error::new(ErrorKind::Other, err));
			}
		}
	}

	fn must_skip(&self, mut path: &Path) -> bool {
		if self.to_skip.contains(path) {
			return true;
		}
		while let Some(parent) = path.parent() {
			if parent == self.base {
				break;
			}
			if self.to_skip.contains(parent) {
				return true;
			}
			path = parent;
		}

		false
	}
}
