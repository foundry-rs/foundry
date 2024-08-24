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
/// use foundry_test_utils::*;
/// forgetest!(my_test, |prj, cmd| {
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
/// use foundry_test_utils::*;
/// use foundry_test_utils::foundry_compilers::PathStyle;
/// forgetest!(can_clean_hardhat, PathStyle::HardHat, |prj, cmd| {
///     prj.assert_create_dirs_exists();
///     prj.assert_style_paths_exist(PathStyle::HardHat);
///     cmd.arg("clean");
///     cmd.assert_empty_stdout();
///     prj.assert_cleaned();
/// });
#[macro_export]
macro_rules! forgetest {
    ($(#[$attr:meta])* $test:ident, |$prj:ident, $cmd:ident| $e:expr) => {
        $crate::forgetest!($(#[$attr])* $test, $crate::foundry_compilers::PathStyle::Dapptools, |$prj, $cmd| $e);
    };
    ($(#[$attr:meta])* $test:ident, $style:expr, |$prj:ident, $cmd:ident| $e:expr) => {
        #[test]
        $(#[$attr])*
        fn $test() {
            let (mut $prj, mut $cmd) = $crate::util::setup_forge(stringify!($test), $style);
            $e
        }
    };
}

#[macro_export]
macro_rules! forgetest_async {
    ($(#[$attr:meta])* $test:ident, |$prj:ident, $cmd:ident| $e:expr) => {
        $crate::forgetest_async!($(#[$attr])* $test, $crate::foundry_compilers::PathStyle::Dapptools, |$prj, $cmd| $e);
    };
    ($(#[$attr:meta])* $test:ident, $style:expr, |$prj:ident, $cmd:ident| $e:expr) => {
        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn $test() {
            let (mut $prj, mut $cmd) = $crate::util::setup_forge(stringify!($test), $style);
            $e
        }
    };
}

#[macro_export]
macro_rules! casttest {
    ($(#[$attr:meta])* $test:ident, $($async:ident)? |$prj:ident, $cmd:ident| $e:expr) => {
        $crate::casttest!($(#[$attr])* $test, $crate::foundry_compilers::PathStyle::Dapptools, $($async)? |$prj, $cmd| $e);
    };
    ($(#[$attr:meta])* $test:ident, $style:expr, |$prj:ident, $cmd:ident| $e:expr) => {
        #[test]
        $(#[$attr])*
        fn $test() {
            let (mut $prj, mut $cmd) = $crate::util::setup_cast(stringify!($test), $style);
            $e
        }
    };
    ($(#[$attr:meta])* $test:ident, $style:expr, async |$prj:ident, $cmd:ident| $e:expr) => {
        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn $test() {
            let (mut $prj, mut $cmd) = $crate::util::setup_cast(stringify!($test), $style);
            $e
        }
    };
}

/// Same as `forgetest` but returns an already initialized project workspace (`forge init`)
#[macro_export]
macro_rules! forgetest_init {
    ($(#[$attr:meta])* $test:ident, |$prj:ident, $cmd:ident| $e:expr) => {
        $crate::forgetest_init!($(#[$attr])* $test, $crate::foundry_compilers::PathStyle::Dapptools, |$prj, $cmd| $e);
    };
    ($(#[$attr:meta])* $test:ident, $style:expr, |$prj:ident, $cmd:ident| $e:expr) => {
        #[test]
        $(#[$attr])*
        fn $test() {
            let (mut $prj, mut $cmd) = $crate::util::setup_forge(stringify!($test), $style);
            $crate::util::initialize($prj.root());
            $e
        }
    };
}

/// Setup forge soldeer
#[macro_export]
macro_rules! forgesoldeer {
    ($(#[$attr:meta])* $test:ident, |$prj:ident, $cmd:ident| $e:expr) => {
        $crate::forgesoldeer!($(#[$attr])* $test, $crate::foundry_compilers::PathStyle::Dapptools, |$prj, $cmd| $e);
    };
    ($(#[$attr:meta])* $test:ident, $style:expr, |$prj:ident, $cmd:ident| $e:expr) => {
        #[test]
        $(#[$attr])*
        fn $test() {
            let (mut $prj, mut $cmd) = $crate::util::setup_forge(stringify!($test), $style);
            $crate::util::initialize($prj.root());
            $e
        }
    };
}
