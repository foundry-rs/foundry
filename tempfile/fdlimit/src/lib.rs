// Copyright 2016-2020 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

/// Outcome of raising file descriptor resource limit
pub enum Outcome {
	/// Limit was raised successfully
	LimitRaised {
		/// Previous limit (likely soft limit)
		from: u64,
		/// New limit (likely hard limit)
		to: u64,
	},
	/// Raising limit is not supported on this platform
	Unsupported,
}

/// Errors that happen when trying to raise file descriptor resource limit
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// Failed to call sysctl to get max supported value configured in sysctl
	#[error("Failed to call sysctl to get max supported value configured in sysctl: {0}")]
	#[cfg(any(target_os = "macos", target_os = "ios"))]
	FailedToCallSysctl(std::io::Error),
	/// Failed to get current limit
	#[error("Failed to get current limit: {0}")]
	FailedToGetLimit(std::io::Error),
	/// Failed to set new limit
	#[error("Failed to set new limit ({from}->{to}): {error}")]
	FailedToSetLimit {
		/// Current limit
		from: u64,
		/// New desired limit
		to: u64,
		/// Low level OS error
		error: std::io::Error,
	},
}

/// Raise the soft open file descriptor resource limit to the smaller of the
/// kernel limit and the hard resource limit.
///
/// darwin_fd_limit exists to work around an issue where launchctl on Mac OS X
/// defaults the rlimit maxfiles to 256/unlimited. The default soft limit of 256
/// ends up being far too low for our multithreaded scheduler testing, depending
/// on the number of cores available.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[allow(clippy::useless_conversion, non_camel_case_types)]
pub fn raise_fd_limit() -> Result<Outcome, Error> {
	use std::cmp;
	use std::io;
	use std::mem::size_of_val;
	use std::ptr::null_mut;

	unsafe {
		static CTL_KERN: libc::c_int = 1;
		static KERN_MAXFILESPERPROC: libc::c_int = 29;

		// The strategy here is to fetch the current resource limits, read the
		// kern.maxfilesperproc sysctl value, and bump the soft resource limit for
		// maxfiles up to the sysctl value.

		// Fetch the kern.maxfilesperproc value
		let mut mib: [libc::c_int; 2] = [CTL_KERN, KERN_MAXFILESPERPROC];
		let mut maxfiles: libc::c_int = 0;
		let mut size: libc::size_t = size_of_val(&maxfiles) as libc::size_t;
		if libc::sysctl(&mut mib[0], 2, &mut maxfiles as *mut _ as *mut _, &mut size, null_mut(), 0)
			!= 0
		{
			return Err(Error::FailedToCallSysctl(io::Error::last_os_error()));
		}

		// Fetch the current resource limits
		let mut rlim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
		if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) != 0 {
			return Err(Error::FailedToGetLimit(io::Error::last_os_error()));
		}

		let old_value = rlim.rlim_cur;

		// Bump the soft limit to the smaller of kern.maxfilesperproc and the hard
		// limit
		rlim.rlim_cur = cmp::min(maxfiles as libc::rlim_t, rlim.rlim_max);

		// Set our newly-increased resource limit
		if libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) != 0 {
			return Err(Error::FailedToSetLimit {
				from: old_value.into(),
				to: rlim.rlim_cur.into(),
				error: io::Error::last_os_error(),
			});
		}

		Ok(Outcome::LimitRaised { from: old_value.into(), to: rlim.rlim_cur.into() })
	}
}

/// Raise the soft open file descriptor resource limit to the hard resource
/// limit.
#[cfg(target_os = "linux")]
#[allow(clippy::useless_conversion, non_camel_case_types)]
pub fn raise_fd_limit() -> Result<Outcome, Error> {
	use std::io;

	unsafe {
		// Fetch the current resource limits
		let mut rlim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
		if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) != 0 {
			return Err(Error::FailedToGetLimit(io::Error::last_os_error()));
		}

		let old_value = rlim.rlim_cur;

		// Set soft limit to hard imit
		rlim.rlim_cur = rlim.rlim_max;

		// Set our newly-increased resource limit
		if libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) != 0 {
			return Err(Error::FailedToSetLimit {
				from: old_value.into(),
				to: rlim.rlim_cur.into(),
				error: io::Error::last_os_error(),
			});
		}

		Ok(Outcome::LimitRaised { from: old_value.into(), to: rlim.rlim_cur.into() })
	}
}

/// Does nothing on unsupported platform
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
pub fn raise_fd_limit() -> Result<Outcome, Error> {
	Ok(Outcome::Unsupported)
}
