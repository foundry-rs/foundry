mod basic;

use super::util::*;

#[test]
fn basic() -> TestResult {
	test_wrapper(basic::main)
}
