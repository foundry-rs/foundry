use trybuild::TestCases;

#[test]
fn ui_compile_pass() {
    let t = TestCases::new();
    t.pass("tests/compile-pass/*.rs");
}

#[rustversion::nightly]
#[test]
fn ui_compile_fail() {
    let t = TestCases::new();
    t.compile_fail("tests/compile-fail/*.rs");
}

#[rustversion::since(1.51)]
#[test]
fn ui_since_1_51_compile_pass() {
    let t = TestCases::new();
    t.pass("tests/since_1.51/compile-pass/*.rs");
}

#[rustversion::since(1.75)]
#[test]
fn ui_since_1_75_compile_pass() {
    let t = TestCases::new();
    t.pass("tests/since_1.75/compile-pass/*.rs");
}
