#[test]
fn default() {
	let cs = clearscreen::ClearScreen::default();
	dbg!(&cs);
	cs.clear().unwrap();
}

#[test]
fn shorthand() {
	clearscreen::clear().unwrap();
}
