mod basic;

use super::util::{tokio::*, TestResult};

#[test]
fn basic() -> TestResult {
	test_wrapper(basic::main())
}
