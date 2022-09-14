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
    ($(#[$meta:meta])* $test:ident, $fun:expr) => {
        $crate::forgetest!($(#[$meta])* $test, $crate::ethers_solc::PathStyle::Dapptools, $fun);
    };
    ($(#[$meta:meta])* $test:ident, $style:expr, $fun:expr) => {
        #[test]
        $(#[$meta])*
        fn $test() {
            let (prj, cmd) = $crate::util::setup_forge(stringify!($test), $style);
            let f = $fun;
            f(prj, cmd);
        }
    };
}

#[macro_export]
macro_rules! forgetest_async {
    ($(#[$meta:meta])* $test:ident, $fun:expr) => {
        $crate::forgetest_async!($(#[$meta])* $test, $crate::ethers_solc::PathStyle::Dapptools, $fun);
    };
    ($(#[$meta:meta])* $test:ident, $style:expr, $fun:expr) => {
        #[tokio::test(flavor = "multi_thread")]
        $(#[$meta])*
        async fn $test() {
            let (prj, cmd) = $crate::util::setup_forge(stringify!($test), $style);
            let f = $fun;
            f(prj, cmd).await;
        }
    };
}

#[macro_export]
macro_rules! casttest {
    ($(#[$meta:meta])* $test:ident, $fun:expr) => {
        $crate::casttest!($(#[$meta])* $test, $crate::ethers_solc::PathStyle::Dapptools, $fun);
    };
    ($(#[$meta:meta])* $test:ident, $style:expr, $fun:expr) => {
        #[test]
        $(#[$meta])*
        fn $test() {
            let (prj, cmd) = $crate::util::setup_cast(stringify!($test), $style);
            let f = $fun;
            f(prj, cmd);
        }
    };
}

/// Same as `forgetest` but returns an already initialized project workspace (`forge init`)
#[macro_export]
macro_rules! forgetest_init {
    ($(#[$meta:meta])* $test:ident, $fun:expr) => {
        $crate::forgetest_init!($(#[$meta])* $test, $crate::ethers_solc::PathStyle::Dapptools, $fun);
    };
    ($(#[$meta:meta])* $test:ident, $style:expr, $fun:expr) => {
        #[test]
        $(#[$meta])*
        fn $test() {
            let (prj, cmd) = $crate::util::setup_forge(stringify!($test), $style);
            $crate::util::initialize(prj.root());
            let f = $fun;
            f(prj, cmd);
        }
    };
}

/// Clones an external repository and makes sure the tests pass.
/// Can optionally enable fork mode as well if a fork block is passed.
/// The fork block needs to be greater than 0.
#[macro_export]
macro_rules! forgetest_external {
    // forgetest_external!(test_name, "owner/repo");
    ($(#[$meta:meta])* $test:ident, $repo:literal) => {
        $crate::forgetest_external!($(#[$meta])* $test, $repo, 0, Vec::<String>::new());
    };
    // forgetest_external!(test_name, "owner/repo", 1234);
    ($(#[$meta:meta])* $test:ident, $repo:literal, $fork_block:literal) => {
        $crate::forgetest_external!(
            $(#[$meta])*
            $test,
            $repo,
            $crate::ethers_solc::PathStyle::Dapptools,
            $fork_block,
            Vec::<String>::new()
        );
    };
    // forgetest_external!(test_name, "owner/repo", &["--extra-opt", "val"]);
    ($(#[$meta:meta])* $test:ident, $repo:literal, $forge_opts:expr) => {
        $crate::forgetest_external!($(#[$meta])* $test, $repo, 0, $forge_opts);
    };
    // forgetest_external!(test_name, "owner/repo", 1234, &["--extra-opt", "val"]);
    ($(#[$meta:meta])* $test:ident, $repo:literal, $fork_block:literal, $forge_opts:expr) => {
        $crate::forgetest_external!(
            $(#[$meta])*
            $test,
            $repo,
            $crate::ethers_solc::PathStyle::Dapptools,
            $fork_block,
            $forge_opts
        );
    };
    // forgetest_external!(test_name, "owner/repo", PathStyle::Dapptools, 123);
    ($(#[$meta:meta])* $test:ident, $repo:literal, $style:expr, $fork_block:literal, $forge_opts:expr) => {
        #[test]
        $(#[$meta])*
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
            let git_clone =
                $crate::util::clone_remote(&format!("https://github.com/{}", $repo), prj.root())
                    .expect("Could not clone repository. Is git installed?");
            assert!(
                git_clone.status.success(),
                "could not clone repository:\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&git_clone.stdout),
                String::from_utf8_lossy(&git_clone.stderr)
            );

            // We just run make install, but we do not care if it worked or not,
            // since some repositories do not have that target
            let make_install = Command::new("make")
                .arg("install")
                .current_dir(prj.root())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();

            // Run the tests
            cmd.arg("test").args($forge_opts).args([
                "--optimize",
                "--optimizer-runs",
                "20000",
                "--ffi",
            ]);
            cmd.set_env("FOUNDRY_FUZZ_RUNS", "1");

            let next_eth_rpc_url = foundry_utils::rpc::next_http_archive_rpc_endpoint();
            if $fork_block > 0 {
                cmd.set_env("FOUNDRY_ETH_RPC_URL", next_eth_rpc_url);
                cmd.set_env("FOUNDRY_FORK_BLOCK_NUMBER", stringify!($fork_block));
            }
            cmd.assert_non_empty_stdout();
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
