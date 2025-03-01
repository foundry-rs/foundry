#![allow(unexpected_cfgs)]

mod null_dacl;
mod sd_graft;

use crate::tests::util::*;

#[cfg(not(ci))]
#[test]
fn sd_graft() -> TestResult {
	test_wrapper(sd_graft::test_main)
}

#[test]
fn null_dacl() -> TestResult {
	test_wrapper(null_dacl::test_main)
}
