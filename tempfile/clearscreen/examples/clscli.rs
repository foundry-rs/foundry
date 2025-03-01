use clearscreen::ClearScreen;
use std::{env, thread::sleep, time::Duration};
use thiserror::Error;

fn main() -> Result<(), Error> {
	if let Some(variant) = env::args().nth(1) {
		let cs = match variant.as_str() {
			"auto" => ClearScreen::default(),
			"Terminfo" => ClearScreen::Terminfo,
			"TerminfoScreen" => ClearScreen::TerminfoScreen,
			"TerminfoScrollback" => ClearScreen::TerminfoScrollback,
			"TerminfoReset" => ClearScreen::TerminfoReset,
			"XtermClear" => ClearScreen::XtermClear,
			"XtermReset" => ClearScreen::XtermReset,
			"TputClear" => ClearScreen::TputClear,
			"TputReset" => ClearScreen::TputReset,
			"Cls" => ClearScreen::Cls,
			"WindowsVt" => ClearScreen::WindowsVt,
			"WindowsVtClear" => ClearScreen::WindowsVtClear,
			#[cfg(feature = "windows-console")]
			"WindowsConsoleClear" => ClearScreen::WindowsConsoleClear,
			#[cfg(feature = "windows-console")]
			"WindowsConsoleBlank" => ClearScreen::WindowsConsoleBlank,
			"WindowsCooked" => ClearScreen::WindowsCooked,
			"VtRis" => ClearScreen::VtRis,
			"VtLeaveAlt" => ClearScreen::VtLeaveAlt,
			"VtCooked" => ClearScreen::VtCooked,
			"VtWellDone" => ClearScreen::VtWellDone,
			_ => return Err(Error::UnknownVariant(variant)),
		};

		println!("variant = {:?}, sleeping 1 second", cs);
		sleep(Duration::from_secs(1));
		cs.clear()?;

		Ok(())
	} else {
		println!("Usage: cargo run --example clscli -- <variant>\nWhere <variant> is one of the ClearScreen enum variants, same casing, or 'auto'.\nI recommend piping into `hexdump -C` to see what’s happening.");
		Ok(())
	}
}

#[derive(Debug, Error)]
enum Error {
	#[error("unknown variant: {0}")]
	UnknownVariant(String),

	#[error(transparent)]
	ClearScreen(#[from] clearscreen::Error),
}
