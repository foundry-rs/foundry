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
    ($(#[$attr:meta])* $test:ident, |$prj:ident, $cmd:ident| $e:expr) => {
        $crate::casttest!($(#[$attr])* $test, $crate::foundry_compilers::PathStyle::Dapptools, |$prj, $cmd| $e);
    };
    ($(#[$attr:meta])* $test:ident, $style:expr, |$prj:ident, $cmd:ident| $e:expr) => {
        #[test]
        $(#[$attr])*
        fn $test() {
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

/// Clones an external repository and makes sure the tests pass.
/// Can optionally enable fork mode as well if a fork block is passed.
/// The fork block needs to be greater than 0.
#[macro_export]
macro_rules! forgetest_external {
    // forgetest_external!(test_name, "owner/repo");
    ($(#[$attr:meta])* $test:ident, $repo:literal) => {
        $crate::forgetest_external!($(#[$attr])* $test, $repo, 0, Vec::<String>::new());
    };
    // forgetest_external!(test_name, "owner/repo", 1234);
    ($(#[$attr:meta])* $test:ident, $repo:literal, $fork_block:literal) => {
        $crate::forgetest_external!(
            $(#[$attr])*
            $test,
            $repo,
            $crate::foundry_compilers::PathStyle::Dapptools,
            $fork_block,
            Vec::<String>::new()
        );
    };
    // forgetest_external!(test_name, "owner/repo", &["--extra-opt", "val"]);
    ($(#[$attr:meta])* $test:ident, $repo:literal, $forge_opts:expr) => {
        $crate::forgetest_external!($(#[$attr])* $test, $repo, 0, $forge_opts);
    };
    // forgetest_external!(test_name, "owner/repo", 1234, &["--extra-opt", "val"]);
    ($(#[$attr:meta])* $test:ident, $repo:literal, $fork_block:literal, $forge_opts:expr) => {
        $crate::forgetest_external!(
            $(#[$attr])*
            $test,
            $repo,
            $crate::foundry_compilers::PathStyle::Dapptools,
            $fork_block,
            $forge_opts
        );
    };
    // forgetest_external!(test_name, "owner/repo", PathStyle::Dapptools, 123);
    ($(#[$attr:meta])* $test:ident, $repo:literal, $style:expr, $fork_block:literal, $forge_opts:expr) => {
        #[test]
        $(#[$attr])*
        fn $test() {
            use std::process::{Command, Stdio};

            // Skip fork tests if the RPC url is not set.
            if $fork_block > 0 && std::env::var("ETH_RPC_URL").is_err() {
                eprintln!("Skipping test {}. ETH_RPC_URL is not set.", $repo);
                return
            };

            let (prj, mut cmd) = $crate::util::setup_forge(stringify!($test), $style);

            // Wipe the default structure
            prj.wipe();

            // Clone the external repository
            $crate::util::clone_remote(concat!("https://github.com/", $repo), prj.root().to_str().unwrap());

            // Run common installation commands
            $crate::util::run_install_commands(prj.root());

            // Run the tests
            cmd.arg("test").args($forge_opts).args([
                "--optimize",
                "--optimizer-runs=20000",
                "--fuzz-runs=256",
                "--ffi",
                "-vvvvv",
            ]);
            cmd.set_env("FOUNDRY_FUZZ_RUNS", "1");

            let next_eth_rpc_url = foundry_common::rpc::next_http_archive_rpc_endpoint();
            if $fork_block > 0 {
                cmd.set_env("FOUNDRY_ETH_RPC_URL", next_eth_rpc_url);
                cmd.set_env("FOUNDRY_FORK_BLOCK_NUMBER", stringify!($fork_block));
            }
            cmd.assert_non_empty_stdout();
        }
    };
}
