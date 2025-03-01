//! Local sockets implemented using Unix domain sockets.

mod listener;
mod stream;

pub use {listener::*, stream::*};

#[cfg(feature = "tokio")]
pub(crate) mod tokio {
	mod listener;
	mod stream;
	pub use {listener::*, stream::*};
}

use crate::{
	local_socket::{Name, NameInner},
	os::unix::unixprelude::*,
};
#[cfg(target_os = "android")]
use std::os::android::net::SocketAddrExt;
#[cfg(target_os = "linux")]
use std::os::linux::net::SocketAddrExt;
use std::{
	borrow::Cow,
	ffi::{OsStr, OsString},
	fs, io, mem,
	os::unix::net::SocketAddr,
	path::Path,
};

#[derive(Clone, Debug, Default)]
struct ReclaimGuard(Option<Name<'static>>);
impl ReclaimGuard {
	fn new(name: Name<'static>) -> Self {
		Self(if name.is_path() { Some(name) } else { None })
	}
	#[cfg_attr(not(feature = "tokio"), allow(dead_code))]
	fn take(&mut self) -> Self {
		Self(self.0.take())
	}
	fn forget(&mut self) {
		self.0 = None;
	}
}
impl Drop for ReclaimGuard {
	fn drop(&mut self) {
		if let Self(Some(Name(NameInner::UdSocketPath(path)))) = self {
			let _ = std::fs::remove_file(path);
		}
	}
}

#[allow(clippy::indexing_slicing)]
fn name_to_addr(name: Name<'_>, create_dirs: bool) -> io::Result<SocketAddr> {
	match name.0 {
		NameInner::UdSocketPath(path) => SocketAddr::from_pathname(path),
		NameInner::UdSocketPseudoNs(name) => construct_and_prepare_pseudo_ns(name, create_dirs),
		#[cfg(any(target_os = "linux", target_os = "android"))]
		NameInner::UdSocketNs(name) => SocketAddr::from_abstract_name(name),
	}
}

const SUN_LEN: usize = {
	let dummy = unsafe { mem::zeroed::<libc::sockaddr_un>() };
	dummy.sun_path.len()
};
const NMCAP: usize = SUN_LEN - "/run/user/18446744073709551614/".len();

static TOOLONG: &str = "local socket name length exceeds capacity of sun_path of sockaddr_un";

/// Checks if `/run/user/<ruid>` exists, returning that path if it does.
fn get_run_user() -> io::Result<Option<OsString>> {
	let path = format!("/run/user/{}", unsafe { libc::getuid() }).into();
	match fs::metadata(&path) {
		Ok(..) => Ok(Some(path)),
		Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
		Err(e) => Err(e),
	}
}

static TMPDIR: &str = {
	#[cfg(target_os = "android")]
	{
		"/data/local/tmp"
	}
	#[cfg(not(target_os = "android"))]
	{
		"/tmp"
	}
};

#[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
fn construct_and_prepare_pseudo_ns(
	name: Cow<'_, OsStr>,
	create_dirs: bool,
) -> io::Result<SocketAddr> {
	let nlen = name.len();
	if nlen > NMCAP {
		return Err(io::Error::new(io::ErrorKind::InvalidInput, TOOLONG));
	}
	let run_user = get_run_user()?;
	let pfx = run_user.map(Cow::Owned).unwrap_or(Cow::Borrowed(OsStr::new(TMPDIR)));
	let pl = pfx.len();
	let mut path = [0; SUN_LEN];
	path[..pl].copy_from_slice(pfx.as_bytes());
	path[pl] = b'/';

	let namestart = pl + 1;
	let fulllen = pl + 1 + nlen;
	path[namestart..fulllen].copy_from_slice(name.as_bytes());

	const ESCCHAR: u8 = b'_';
	for byte in path[namestart..fulllen].iter_mut() {
		if *byte == 0 {
			*byte = ESCCHAR;
		}
	}

	let opath = Path::new(OsStr::from_bytes(&path[..fulllen]));

	if create_dirs {
		let parent = opath.parent();
		if let Some(p) = parent {
			fs::create_dir_all(p)?;
		}
	}
	SocketAddr::from_pathname(opath)
}
