use super::{TestResult, WrapErrExt};
use color_eyre::eyre::bail;
use std::{sync::mpsc, thread, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(2);

pub fn run_under_wachdog(f: impl (FnOnce() -> TestResult) + Send + 'static) -> TestResult {
	let (killswitch, timeout_joiner) = mpsc::channel();
	let joiner = thread::Builder::new()
		.name("test main (under watchdog)".to_owned())
		.spawn(move || {
			let ret = f();
			let _ = killswitch.send(());
			ret
		})
		.opname("watchdog scrutinee thread spawn")?;
	if let Err(mpsc::RecvTimeoutError::Timeout) = timeout_joiner.recv_timeout(TIMEOUT) {
		bail!("watchdog timer has run out");
	}
	joiner.join().unwrap()
}
