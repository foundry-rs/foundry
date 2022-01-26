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

/// Clones an external repository and makes sure the tests pass.
#[macro_export]
macro_rules! forgetest_external {
    ($test:ident, $repo:literal) => {
        $crate::forgetest_external!($test, $repo, $crate::ethers_solc::PathStyle::Dapptools);
    };
    ($test:ident, $repo:literal, $style:expr) => {
        #[test]
        fn $test() {
            use std::process::{Command, Stdio};
            let (prj, mut cmd) = $crate::util::setup(stringify!($test), $style);

            // Wipe the default structure
            prj.wipe();

            // Clone the external repository
            let git_clone = Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    "--recursive",
                    &format!("https://github.com/{}", $repo),
                    prj.root().to_str().expect("could not get project root"),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("could not clone repository. is git installed?");
            assert!(git_clone.success(), "could not clone repository");

            // We just run make install, but we do not care if it worked or not
            let make_install = Command::new("make")
                .arg("install")
                .current_dir(prj.root())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();

            // Run the tests
            cmd.arg("test")
                .arg("--optimize")
                .arg("--optimize-runs")
                .arg("20000")
                .arg("--ffi")
                .set_env("FOUNDRY_FUZZ_RUNS", "1");
            cmd.assert_non_empty_stdout();
        }
    };
}

/// Like forgetest_external! but with RPC forking
#[macro_export]
macro_rules! forgetest_external_forking {
    ($test:ident, $repo:literal, $fork_block:literal) => {
        $crate::forgetest_external_forking!(
            $test,
            $repo,
            $crate::ethers_solc::PathStyle::Dapptools,
            $fork_block
        );
    };
    ($test:ident, $repo:literal, $style:expr, $fork_block:literal) => {
        #[test]
        fn $test() {
            use std::process::{Command, Stdio};

            // Skip fork tests if the RPC url is not set.
            if std::env::var("FOUNDRY_ETH_RPC_URL").is_err() {
                eprintln!("Skipping test {}. FOUNDRY_ETH_RPC_URL is not set.", $repo);
                return;
            };

            let (prj, mut cmd) = $crate::util::setup(stringify!($test), $style);

            // Wipe the default structure
            prj.wipe();

            // Clone the external repository
            let git_clone = Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    "--recursive",
                    &format!("https://github.com/{}", $repo),
                    prj.root().to_str().expect("could not get project root"),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("could not clone repository. is git installed?");
            assert!(git_clone.success(), "could not clone repository");

            // We just run make install, but we do not care if it worked or not,
            // since some repositories do not have that target
            let make_install = Command::new("make")
                .arg("install")
                .current_dir(prj.root())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();

            // Run the tests
            cmd.arg("test")
                .arg("--optimize")
                .arg("--optimize-runs")
                .arg("20000")
                .arg("--ffi")
                .set_env("FOUNDRY_FUZZ_RUNS", "1");
            cmd.set_env("FOUNDRY_FORK_BLOCK_NUMBER", stringify!($fork_block));
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
