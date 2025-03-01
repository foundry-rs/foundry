/// Calculates the successor element of the given integer using your processor's preferred internal
/// algorithm, panicking in debug mode if overflow occurs and overflowing to 0 in release mode.
///
/// # Examples
/// ```
#[doc = doctest_file::include_doctest!("examples/doc.rs")]
/// ```
pub fn plus_1(n: u32) -> u32 {
	n + 1
}
