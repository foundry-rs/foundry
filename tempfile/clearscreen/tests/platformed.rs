use std::env::var;

use clearscreen::ClearScreen;

#[test]
fn terminfo() {
	if var("TERM").is_ok() && (cfg!(unix) || var("TERMINFO").is_ok()) {
		ClearScreen::Terminfo.clear().unwrap();
	}
}

#[test]
fn terminfo_screen() {
	if var("TERM").is_ok() && (cfg!(unix) || var("TERMINFO").is_ok()) {
		ClearScreen::TerminfoScreen.clear().unwrap();
	}
}

#[test]
fn terminfo_scrollback() {
	if var("TERM").is_ok() && (cfg!(unix) || var("TERMINFO").is_ok()) {
		ClearScreen::TerminfoScrollback.clear().unwrap();
	}
}

#[test]
fn terminfo_reset() {
	if var("TERM").is_ok() && (cfg!(unix) || var("TERMINFO").is_ok()) {
		ClearScreen::TerminfoReset.clear().unwrap();
	}
}

#[test]
fn xterm_clear() {
	ClearScreen::XtermClear.clear().unwrap();
}

#[test]
fn xterm_reset() {
	ClearScreen::XtermReset.clear().unwrap();
}

#[test]
fn tput_clear() {
	if var("TERM").is_ok() && (cfg!(unix) || var("TERMINFO").is_ok()) {
		ClearScreen::TputClear.clear().unwrap();
	}
}

#[test]
fn tput_reset() {
	if var("TERM").is_ok() && (cfg!(unix) || var("TERMINFO").is_ok()) {
		ClearScreen::TputReset.clear().unwrap();
	}
}

#[cfg(windows)]
#[test]
fn windows_cls() {
	ClearScreen::Cls.clear().unwrap();
}

#[test]
fn windows_vt() {
	ClearScreen::WindowsVt.clear().unwrap();
}

#[test]
fn windows_vt_clear() {
	ClearScreen::WindowsVtClear.clear().unwrap();
}

#[test]
fn vt_ris() {
	ClearScreen::VtRis.clear().unwrap();
}

// TODO: test these under Win8? why don't they work
//
// #[test]
// fn windows_console_clear() {
// 	ClearScreen::WindowsConsoleClear.clear().unwrap();
// }
//
// #[test]
// fn windows_console_blank() {
// 	ClearScreen::WindowsConsoleBlank.clear().unwrap();
// }
