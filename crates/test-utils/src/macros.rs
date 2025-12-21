/// A macro to generate a new integration test case
///
/// The `forgetest!` macro's first argument is the name of the test, the second argument is a
/// closure to configure and execute the test. The `TestProject` provides utility functions to setup
/// the project's workspace. The `TestCommand` is a wrapper around the actual `forge` executable
/// that this then executed with the configured command arguments.
#[macro_export]
macro_rules! forgetest {
    ($(#[$attr:meta])* $test:ident, |$prj:ident, $cmd:ident| $e:expr) => {
        $crate::forgetest!($(#[$attr])* $test, $crate::foundry_compilers::PathStyle::Dapptools, |$prj, $cmd| $e);
    };
    ($(#[$attr:meta])* $test:ident, $style:expr, |$prj:ident, $cmd:ident| $e:expr) => {
        #[expect(clippy::disallowed_macros)]
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
        #[expect(clippy::disallowed_macros)]
        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn $test() {
            let (mut $prj, mut $cmd) = $crate::util::setup_forge(stringify!($test), $style);
            $e;
            return (); // Works around weird method resolution in `$e` due to `#[tokio::test]`.
        }
    };
}

#[macro_export]
macro_rules! casttest {
    ($(#[$attr:meta])* $test:ident, $($async:ident)? |$prj:ident, $cmd:ident| $e:expr) => {
        $crate::casttest!($(#[$attr])* $test, $crate::foundry_compilers::PathStyle::Dapptools, $($async)? |$prj, $cmd| $e);
    };
    ($(#[$attr:meta])* $test:ident, $style:expr, |$prj:ident, $cmd:ident| $e:expr) => {
        #[expect(clippy::disallowed_macros)]
        #[test]
        $(#[$attr])*
        fn $test() {
            let (mut $prj, mut $cmd) = $crate::util::setup_cast(stringify!($test), $style);
            $e
        }
    };
    ($(#[$attr:meta])* $test:ident, $style:expr, async |$prj:ident, $cmd:ident| $e:expr) => {
        #[expect(clippy::disallowed_macros)]
        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn $test() {
            let (mut $prj, mut $cmd) = $crate::util::setup_cast(stringify!($test), $style);
            $e;
            return (); // Works around weird method resolution in `$e` due to `#[tokio::test]`.
        }
    };
}

/// Same as `forgetest` but returns an already initialized project workspace (`forge init --empty`).
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
        #[expect(clippy::disallowed_macros)]
        #[test]
        $(#[$attr])*
        fn $test() {
            let (mut $prj, mut $cmd) = $crate::util::setup_forge(stringify!($test), $style);
            $crate::util::initialize($prj.root());
            $e
        }
    };
}

#[macro_export]
macro_rules! test_debug {
    ($($args:tt)*) => {
        $crate::test_debug(format_args!($($args)*))
    }
}

#[macro_export]
macro_rules! test_trace {
    ($($args:tt)*) => {
        $crate::test_trace(format_args!($($args)*))
    }
}
