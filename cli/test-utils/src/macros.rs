/// A macro to generate a new test case
/// accepts a closure with an empty project and the forge executable
#[macro_export]
macro_rules! forgetest {
    ($test:ident, $fun:expr) => {
        $crate::forgetest!($test, $crate::ethers_solc::PathStyle::Dapptools, $fun);
    };
    ($test:ident, $style:expr, $fun:expr) => {
        #[test]
        fn $test() {
            let (dir, cmd) = $crate::util::setup(stringify!($test), $style);
            $fun(dir, cmd);
        }
    };
}

/// A macro to compare outputs
#[macro_export]
macro_rules! pretty_eq {
    ($expected:expr, $got:expr) => {
        let expected = &*$expected;
        let got = &*$got;
        if expected != got {
            panic!(
                "
outputs differ!

expected:
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
{}
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

got:
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
{}
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
",
                expected, got
            );
        }
    };
}
