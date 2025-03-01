use super::unixprelude::*;
use crate::AsPtr;
#[allow(unused_imports)]
use crate::{FdOrErrno, OrErrno};
use libc::{sockaddr_un, AF_UNIX};
#[cfg(target_os = "android")]
use std::os::android::net::SocketAddrExt;
#[cfg(target_os = "linux")]
use std::os::linux::net::SocketAddrExt;
use std::{
	io,
	mem::{transmute, zeroed},
	os::unix::net::SocketAddr,
	sync::atomic::{AtomicBool, Ordering::Relaxed},
};

macro_rules! cfg_atomic_cloexec {
	($($block:tt)+) => {
		#[cfg(any(
			// List taken from the standard library, file std/sys/pal/unix/net.rs.
			target_os = "android",
			target_os = "dragonfly",
			target_os = "freebsd",
			target_os = "illumos",
			target_os = "hurd",
			target_os = "linux",
			target_os = "netbsd",
			target_os = "openbsd",
			target_os = "nto",
		))]
		$($block)+
	};
}
macro_rules! cfg_no_atomic_cloexec {
	($($block:tt)+) => {
		#[cfg(not(any(
			target_os = "android",
			target_os = "dragonfly",
			target_os = "freebsd",
			target_os = "illumos",
			target_os = "hurd",
			target_os = "linux",
			target_os = "netbsd",
			target_os = "openbsd",
			target_os = "nto",
		)))]
		$($block)+
	};
}

cfg_atomic_cloexec! {
	pub(super) unsafe fn fcntl_int(
		fd: BorrowedFd<'_>,
		cmd: c_int,
		val: c_int,
	) -> io::Result<c_int> {
		unsafe { libc::fcntl(fd.as_raw_fd(), cmd, val) }.fd_or_errno()
	}
}

pub(super) fn duplicate_fd(fd: BorrowedFd<'_>) -> io::Result<OwnedFd> {
	cfg_atomic_cloexec! {{
		let new_fd = unsafe { fcntl_int(fd, libc::F_DUPFD_CLOEXEC, 0)? };
		Ok(unsafe { OwnedFd::from_raw_fd(new_fd) })
	}}
	cfg_no_atomic_cloexec! {{
		let new_fd = unsafe {
			libc::dup(fd.as_raw_fd())
				.fd_or_errno()
				.map(|fd| OwnedFd::from_raw_fd(fd))?
		};
		set_cloexec(new_fd.as_fd())?;
		Ok(new_fd)
	}}
}

fn get_flflags(fd: BorrowedFd<'_>) -> io::Result<c_int> {
	unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL, 0) }.fd_or_errno()
}
fn set_flflags(fd: BorrowedFd<'_>, flags: c_int) -> io::Result<()> {
	unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags) != -1 }.true_val_or_errno(())
}
pub(super) fn set_nonblocking(fd: BorrowedFd<'_>, nonblocking: bool) -> io::Result<()> {
	let old_flags = get_flflags(fd)? & libc::O_NONBLOCK;
	set_flflags(
		fd,
		old_flags | if nonblocking { libc::O_NONBLOCK } else { 0 },
	)
}

cfg_no_atomic_cloexec! {
	fn get_fdflags(fd: BorrowedFd<'_>) -> io::Result<c_int> {
		unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD, 0) }.fd_or_errno()
	}
}
cfg_no_atomic_cloexec! {
	fn set_fdflags(fd: BorrowedFd<'_>, flags: c_int) -> io::Result<()> {
		unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags) != -1 }.true_val_or_errno(())
	}
}
cfg_no_atomic_cloexec! {
	fn set_cloexec(fd: BorrowedFd<'_>) -> io::Result<()> {
		set_fdflags(fd, get_fdflags(fd)? | libc::FD_CLOEXEC)
	}
}

pub(super) fn set_socket_mode(fd: BorrowedFd<'_>, mode: mode_t) -> io::Result<()> {
	unsafe { libc::fchmod(fd.as_raw_fd(), mode) != -1 }.true_val_or_errno(())
}

pub(super) const CAN_CREATE_NONBLOCKING: bool =
	cfg!(any(target_os = "linux", target_os = "android"));

static CAN_FCHMOD_SOCKETS: AtomicBool = AtomicBool::new(true);

/// Creates a Unix domain socket of the given type. If on Linux or Android and `nonblocking` is
/// `true`, also makes it nonblocking.
#[allow(unused_mut)]
fn create_socket(ty: c_int, nonblocking: bool) -> io::Result<OwnedFd> {
	// Suppress warning on platforms that don't support the flag.
	let _ = nonblocking;
	let mut flags = 0;
	#[cfg(any(target_os = "linux", target_os = "android"))]
	{
		if nonblocking {
			flags |= libc::SOCK_NONBLOCK;
		}
	}
	cfg_atomic_cloexec! {{
		flags |= libc::SOCK_CLOEXEC;
	}}

	let fd = unsafe { libc::socket(AF_UNIX, ty | flags, 0) }
		.fd_or_errno()
		.map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })?;

	cfg_no_atomic_cloexec! {{
		set_cloexec(fd.as_fd())?;
	}}
	Ok(fd)
}

fn addr_to_slice(addr: &SocketAddr) -> (&[u8], usize) {
	if let Some(slice) = addr.as_pathname() {
		(slice.as_os_str().as_bytes(), 0)
	} else {
		#[cfg(any(target_os = "linux", target_os = "android"))]
		if let Some(slice) = addr.as_abstract_name() {
			return (slice, 1);
		}
		(&[], 0)
	}
}

#[allow(clippy::as_conversions)]
const SUN_PATH_OFFSET: usize = unsafe {
	// This code may or may not have been copied from the standard library
	let addr = zeroed::<sockaddr_un>();
	let base = (&addr as *const sockaddr_un).cast::<libc::c_char>();
	let path = &addr.sun_path as *const c_char;
	path.byte_offset_from(base) as usize
};

#[allow(
	clippy::indexing_slicing,
	clippy::arithmetic_side_effects,
	clippy::as_conversions
)]
fn bind(fd: BorrowedFd<'_>, addr: &SocketAddr) -> io::Result<()> {
	let (path, extra) = addr_to_slice(addr);
	let path = unsafe { transmute::<&[u8], &[libc::c_char]>(path) };

	let mut addr = unsafe { zeroed::<sockaddr_un>() };
	addr.sun_family = AF_UNIX as _;
	addr.sun_path[extra..(extra + path.len())].copy_from_slice(path);

	let len = path.len() + extra + SUN_PATH_OFFSET;

	unsafe {
		libc::bind(
			fd.as_raw_fd(),
			addr.as_ptr().cast(),
			// It's impossible for this to exceed socklen_t::MAX, since it came from a valid
			// SocketAddr
			len as _,
		) != -1
	}
	.true_val_or_errno(())
}

fn listen(fd: BorrowedFd<'_>) -> io::Result<()> {
	// The standard library does this
	#[cfg(any(
		target_os = "windows",
		target_os = "redox",
		target_os = "espidf",
		target_os = "horizon"
	))]
	const BACKLOG: libc::c_int = 128;
	#[cfg(any(
		target_os = "linux",
		target_os = "freebsd",
		target_os = "openbsd",
		target_os = "macos"
	))]
	const BACKLOG: libc::c_int = -1;
	#[cfg(not(any(
		target_os = "windows",
		target_os = "redox",
		target_os = "linux",
		target_os = "freebsd",
		target_os = "openbsd",
		target_os = "macos",
		target_os = "espidf",
		target_os = "horizon"
	)))]
	const BACKLOG: libc::c_int = libc::SOMAXCONN;
	unsafe { libc::listen(fd.as_raw_fd(), BACKLOG) != -1 }.true_val_or_errno(())
}

struct WithUmask {
	new: mode_t,
	old: mode_t,
}
impl WithUmask {
	pub fn set(new: mode_t) -> Self {
		Self {
			new,
			old: Self::umask(new),
		}
	}
	fn umask(mode: mode_t) -> mode_t {
		unsafe { libc::umask(mode) }
	}
}
impl Drop for WithUmask {
	fn drop(&mut self) {
		let expected_new = Self::umask(self.old);
		assert_eq!(self.new, expected_new, "concurrent umask use detected");
	}
}

pub(super) fn bind_and_listen_with_mode(
	ty: c_int,
	addr: &SocketAddr,
	nonblocking: bool,
	mode: Option<mode_t>,
) -> io::Result<OwnedFd> {
	let mut unset_can_fchmod = false;

	let _dg = if let Some(mode) = mode {
		if mode & 0o111 != 0 {
			return Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"sockets can not be marked executable",
			));
		}

		if CAN_FCHMOD_SOCKETS.load(Relaxed) {
			let sock = create_socket(ty, nonblocking)?;
			match set_socket_mode(sock.as_fd(), mode) {
				Ok(()) => {
					bind(sock.as_fd(), addr)?;
					listen(sock.as_fd())?;
					return Ok(sock);
				}
				Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
					unset_can_fchmod = true;
				}
				Err(e) => return Err(e),
			}
		}
		// If we haven't returned by this point, we can't fchmod sockets. Invert the mode to
		// obtain an anti-mask, call umask() and return a drop guard that lasts until the end of
		// the whole function's scope.
		Some(WithUmask::set(!mode & 0o777))
	} else {
		// If no file mode had to be set, we don't get a umask drop guard.
		None
	};
	// All of the below calls happen under umask if necessary (we race in this muthafucka, better
	// get yo secure code ass back to Linux)

	let rslt = (|| {
		let sock = create_socket(ty, nonblocking)?;
		bind(sock.as_fd(), addr)?;
		listen(sock.as_fd())?;
		io::Result::Ok(sock)
	})();

	// If we're sure that the EINVAL is our fault, unset the global variable.
	match rslt {
		Ok(sock) => Ok(sock),
		Err(e) if e.kind() == io::ErrorKind::InvalidInput => Err(e),
		Err(e) => {
			if unset_can_fchmod {
				CAN_FCHMOD_SOCKETS.store(false, Relaxed);
			}
			Err(e)
		}
	}
}

#[allow(dead_code)]
pub(super) fn shutdown(fd: BorrowedFd<'_>, how: std::net::Shutdown) -> io::Result<()> {
	use std::net::Shutdown::*;
	let how = match how {
		Read => libc::SHUT_RD,
		Write => libc::SHUT_WR,
		Both => libc::SHUT_RDWR,
	};
	unsafe { libc::shutdown(fd.as_raw_fd(), how) != -1 }.true_val_or_errno(())
}
