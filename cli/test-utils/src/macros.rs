/// A macro to generate a new integration test case
///
/// The `forgetest!` macro's first argument is the name of the test, the second argument is a
/// closure to configure and execute the test. The `TestProject` provides utility functions to setup
/// the project's workspace. The `TestCommand` is a wrapper around the actual `forge` executable
/// that this then executed with the configured command arguments.
///
/// # Example
///
/// run `forge init`
///
/// ```no_run
/// use foundry_cli_test_utils::*;
/// forgetest!(my_test, |prj: TestProject, mut cmd: TestCommand| {
///     // adds `init` to forge's command arguments
///     cmd.arg("init");
///     // executes forge <args> and panics if the command failed or output is empty
///     cmd.assert_non_empty_stdout();
/// });
/// ```
///
/// Configure a hardhat project layout by adding a `PathStyle::HardHat` argument
///
/// ```no_run
/// use foundry_cli_test_utils::*;
/// use foundry_cli_test_utils::ethers_solc::PathStyle;
/// forgetest!(can_clean_hardhat, PathStyle::HardHat, |prj: TestProject, mut cmd: TestCommand| {
///     prj.assert_create_dirs_exists();
///     prj.assert_style_paths_exist(PathStyle::HardHat);
///     cmd.arg("clean");
///     cmd.assert_empty_stdout();
///     prj.assert_cleaned();
/// });
#[macro_export]
macro_rules! forgetest {
    ($test:ident, $fun:expr) => {
        $crate::forgetest!($test, $crate::ethers_solc::PathStyle::Dapptools, $fun);
    };
    ($test:ident, $style:expr, $fun:expr) => {
        #[test]
        fn $test() {
            let (prj, cmd) = $crate::util::setup(stringify!($test), $style);
            $fun(prj, cmd);
        }
    };
    (#[ignore], $test:ident, $style:expr, $fun:expr) => {};
}

/// A helper macro to ignore `forgetest!` that should not run on CI
#[macro_export]
macro_rules! forgetest_ignore {
    ($test:ident, $fun:expr) => {
        $crate::forgetest_ignore!($test, $crate::ethers_solc::PathStyle::Dapptools, $fun);
    };
    ($test:ident, $style:expr, $fun:expr) => {
        #[test]
        #[ignore]
        fn $test() {
            let (prj, cmd) = $crate::util::setup(stringify!($test), $style);
            $fun(prj, cmd);
        }
    };
}

/// Same as `forgetest` but returns an already initialized project workspace (`forge init`)
#[macro_export]
macro_rules! forgetest_init {
    ($test:ident, $fun:expr) => {
        $crate::forgetest_init!($test, $crate::ethers_solc::PathStyle::Dapptools, $fun);
    };
    ($test:ident, $style:expr, $fun:expr) => {
        #[test]
        fn $test() {
            let (prj, cmd) = $crate::util::setup(stringify!($test), $style);
            $crate::util::initialize(prj.root());
            $fun(prj, cmd);
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
