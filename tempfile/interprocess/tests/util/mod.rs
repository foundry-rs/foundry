//! Test utilities for allocating an address for the server and then spawning clients to connect to
//! it.
#![allow(dead_code, unused_macros)]

#[macro_use]
mod eyre;
#[macro_use]
mod namegen;
mod choke;
mod drive;
mod wdt;
mod xorshift;

#[allow(unused_imports)]
pub use {drive::*, eyre::*, namegen::*, xorshift::*};

#[cfg(feature = "tokio")]
pub mod tokio;

const NUM_CLIENTS: u32 = 80;
const NUM_CONCURRENT_CLIENTS: u32 = 6;

use color_eyre::eyre::WrapErr;
use std::{
	fmt::{Arguments, Debug},
	io,
	sync::Arc,
};

pub fn test_wrapper(f: impl (FnOnce() -> TestResult) + Send + 'static) -> TestResult {
	eyre::install();
	self::wdt::run_under_wachdog(f)
}

pub fn message(msg: Option<Arguments<'_>>, server: bool, terminator: Option<char>) -> Box<str> {
	let msg = msg.unwrap_or_else(|| format_args!("Message"));
	let sc = if server { "server" } else { "client" };
	let mut msg = format!("{msg} from {sc}!");
	if let Some(t) = terminator {
		msg.push(t);
	}
	msg.into()
}

pub fn listen_and_pick_name<L: Debug, N: Debug + ?Sized, F: FnMut(u32) -> NameResult<N>>(
	namegen: &mut NameGen<N, F>,
	mut bindfn: impl FnMut(&N) -> io::Result<L>,
) -> TestResult<(Arc<N>, L)> {
	use std::io::ErrorKind::*;
	let listener = namegen
		.find_map(|nm| {
			eprintln!("Trying name {nm:?}...");
			let nm = match nm {
				Ok(ok) => ok,
				Err(e) => return Some(Err(e)),
			};
			let l = match bindfn(&nm) {
				Ok(l) => l,
				Err(e) if matches!(e.kind(), AddrInUse | PermissionDenied) => {
					eprintln!("\"{}\", skipping", e.kind());
					return None;
				}
				Err(e) => return Some(Err(e)),
			};
			Some(Ok((nm, l)))
		})
		.unwrap() // Infinite iterator
		.context("listener bind failed")?;
	eprintln!("Listener successfully created: {listener:#?}");
	Ok(listener)
}
