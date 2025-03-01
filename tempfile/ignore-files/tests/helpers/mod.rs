use std::path::{Path, PathBuf};

use ignore::{gitignore::Glob, Match};
use ignore_files::{IgnoreFile, IgnoreFilter};

pub mod ignore_tests {
	pub use super::ig_file as file;
	pub use super::ignore_filt as filt;
	pub use super::Applies;
	pub use super::PathHarness;
}

/// Get the drive letter of the current working directory.
#[cfg(windows)]
fn drive_root() -> String {
	let path = std::fs::canonicalize(".").unwrap();

	let Some(prefix) = path.components().next() else {
		return r"C:\".into();
	};

	match prefix {
		std::path::Component::Prefix(prefix_component) => prefix_component
			.as_os_str()
			.to_str()
			.map(|p| p.to_owned() + r"\")
			.unwrap_or(r"C:\".into()),
		_ => r"C:\".into(),
	}
}

fn normalize_path(path: &str) -> PathBuf {
	#[cfg(windows)]
	let path: &str = &String::from(path)
		.strip_prefix("/")
		.map_or(path.into(), |p| drive_root() + p);

	let path: PathBuf = if Path::new(path).has_root() {
		path.into()
	} else {
		std::fs::canonicalize(".").unwrap().join("tests").join(path)
	};

	dunce::simplified(&path).into()
}

pub trait PathHarness {
	fn check_path(&self, path: &Path, is_dir: bool) -> Match<&Glob>;

	fn path_pass(&self, path: &str, is_dir: bool, pass: bool) {
		let full_path = &normalize_path(path);

		tracing::info!(?path, ?is_dir, ?pass, "check");

		let result = self.check_path(full_path, is_dir);

		assert_eq!(
			match result {
				Match::None => true,
				Match::Ignore(glob) => !glob.from().map_or(true, |f| full_path.starts_with(f)),
				Match::Whitelist(_glob) => true,
			},
			pass,
			"{} {:?} (expected {}) [result: {}]",
			if is_dir { "dir" } else { "file" },
			full_path,
			if pass { "pass" } else { "fail" },
			match result {
				Match::None => String::from("None"),
				Match::Ignore(glob) => format!(
					"Ignore({})",
					glob.from()
						.map_or(String::new(), |f| f.display().to_string())
				),
				Match::Whitelist(glob) => format!(
					"Whitelist({})",
					glob.from()
						.map_or(String::new(), |f| f.display().to_string())
				),
			},
		);
	}

	fn file_does_pass(&self, path: &str) {
		self.path_pass(path, false, true);
	}

	fn file_doesnt_pass(&self, path: &str) {
		self.path_pass(path, false, false);
	}

	fn dir_does_pass(&self, path: &str) {
		self.path_pass(path, true, true);
	}

	fn dir_doesnt_pass(&self, path: &str) {
		self.path_pass(path, true, false);
	}

	fn agnostic_pass(&self, path: &str) {
		self.file_does_pass(path);
		self.dir_does_pass(path);
	}

	fn agnostic_fail(&self, path: &str) {
		self.file_doesnt_pass(path);
		self.dir_doesnt_pass(path);
	}
}

impl PathHarness for IgnoreFilter {
	fn check_path(&self, path: &Path, is_dir: bool) -> Match<&Glob> {
		self.match_path(path, is_dir)
	}
}

fn tracing_init() {
	use tracing_subscriber::{
		fmt::{format::FmtSpan, Subscriber},
		util::SubscriberInitExt,
		EnvFilter,
	};
	Subscriber::builder()
		.pretty()
		.with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
		.with_env_filter(EnvFilter::from_default_env())
		.finish()
		.try_init()
		.ok();
}

pub async fn ignore_filt(origin: &str, ignore_files: &[IgnoreFile]) -> IgnoreFilter {
	tracing_init();
	let origin = normalize_path(origin);
	IgnoreFilter::new(origin, ignore_files)
		.await
		.expect("making filterer")
}

pub fn ig_file(name: &str) -> IgnoreFile {
	let path = normalize_path(name);
	let parent: PathBuf = path.parent().unwrap_or(&path).into();
	IgnoreFile {
		path,
		applies_in: Some(parent),
		applies_to: None,
	}
}

pub trait Applies {
	fn applies_globally(self) -> Self;
}

impl Applies for IgnoreFile {
	fn applies_globally(mut self) -> Self {
		self.applies_in = None;
		self
	}
}
